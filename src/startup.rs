//! Auto-start on login via the HKCU Run registry key (Windows).
//!
//! On non-Windows platforms both functions compile to no-ops so the rest of
//! the codebase can call them unconditionally.

/// Enable or disable auto-start on login.
///
/// When `enabled` is `true`, writes `command` (already properly quoted by the
/// caller: exe path + any args) as a `REG_SZ` value named `real_live_wall`
/// under `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`.
///
/// When `enabled` is `false`, deletes that value (silently ignores "not
/// found" errors so the call is idempotent).
pub fn set_autostart(enabled: bool, command: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        win::set_autostart(enabled, command)
    }
    #[cfg(not(windows))]
    {
        let _ = (enabled, command);
        Ok(())
    }
}

/// Returns `true` if the `real_live_wall` Run value currently exists under
/// `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`.
pub fn autostart_enabled() -> bool {
    #[cfg(windows)]
    {
        win::autostart_enabled()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

// ---------------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use anyhow::{bail, Context};

    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    /// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
    const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

    /// The registry value name used to identify this application.
    const VALUE_NAME: &str = "real_live_wall";

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Convert a Rust `&str` to a null-terminated UTF-16 `Vec<u16>`.
    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0u16)).collect()
    }

    /// Wrap a `WIN32_ERROR` return value: `ERROR_SUCCESS` → `Ok(())`, anything
    /// else → `Err` with the numeric code and a human-readable label.
    fn check(err: WIN32_ERROR, context: &'static str) -> anyhow::Result<()> {
        if err == ERROR_SUCCESS {
            Ok(())
        } else {
            bail!("{context}: Win32 error {}", err.0)
        }
    }

    // -----------------------------------------------------------------------
    // RAII wrapper so we never leak an HKEY
    // -----------------------------------------------------------------------

    struct RegKey(HKEY);

    impl Drop for RegKey {
        fn drop(&mut self) {
            // RegCloseKey returns WIN32_ERROR but there is nothing useful to do
            // on failure here.
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Open helpers
    // -----------------------------------------------------------------------

    /// Open the Run key for writing (`KEY_SET_VALUE`), creating it if absent.
    ///
    /// `RegCreateKeyExW` is gated on `Win32_Security` (for `SECURITY_ATTRIBUTES`
    /// in its signature).  We pass `None` for the security-attributes parameter
    /// so the key inherits default security; no `Win32_Security` types are
    /// actually *used* in our code — only the feature flag must be enabled in
    /// Cargo.toml.
    fn open_for_write() -> anyhow::Result<RegKey> {
        let subkey = to_wide(RUN_SUBKEY);
        let mut hkey = HKEY::default();

        let err = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                windows::core::PCWSTR(subkey.as_ptr()),
                None,                               // reserved
                windows::core::PCWSTR::null(),      // lpClass (no class)
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,                               // default security
                &mut hkey,
                None,                               // disposition (don't care)
            )
        };
        check(err, "RegCreateKeyExW(Run)")?;
        Ok(RegKey(hkey))
    }

    /// Open the Run key for reading (`KEY_QUERY_VALUE`).  Returns `None` if
    /// the key does not exist (unlikely for the system Run key, but defensive).
    fn open_for_read() -> Option<RegKey> {
        let subkey = to_wide(RUN_SUBKEY);
        let mut hkey = HKEY::default();

        let err = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                windows::core::PCWSTR(subkey.as_ptr()),
                None,               // uloptions
                KEY_QUERY_VALUE,
                &mut hkey,
            )
        };
        if err == ERROR_SUCCESS {
            Some(RegKey(hkey))
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // Public entry points (called from the super-module stubs)
    // -----------------------------------------------------------------------

    pub fn set_autostart(enabled: bool, command: &str) -> anyhow::Result<()> {
        if enabled {
            let key = open_for_write()?;
            let value_name = to_wide(VALUE_NAME);
            // Encode the command as null-terminated UTF-16 and pass as raw
            // bytes.  RegSetValueExW's safe wrapper accepts &[u8] and deduces
            // the length automatically, so we must provide the full byte slice
            // including the null terminator (2 bytes).
            let data_utf16 = to_wide(command);
            let data_bytes: &[u8] = unsafe {
                std::slice::from_raw_parts(
                    data_utf16.as_ptr() as *const u8,
                    data_utf16.len() * 2,
                )
            };

            let err = unsafe {
                RegSetValueExW(
                    key.0,
                    windows::core::PCWSTR(value_name.as_ptr()),
                    None,           // reserved
                    REG_SZ,
                    Some(data_bytes),
                )
            };
            check(err, "RegSetValueExW(real_live_wall)").context("failed to write autostart value")
        } else {
            // Delete the value; silently ignore "value not found".
            let key = match open_for_write() {
                Ok(k) => k,
                Err(_) => return Ok(()), // key doesn't exist → nothing to delete
            };
            let value_name = to_wide(VALUE_NAME);
            let err = unsafe {
                RegDeleteValueW(
                    key.0,
                    windows::core::PCWSTR(value_name.as_ptr()),
                )
            };
            if err == ERROR_SUCCESS || err == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                bail!("RegDeleteValueW(real_live_wall): Win32 error {}", err.0)
            }
        }
    }

    pub fn autostart_enabled() -> bool {
        let key = match open_for_read() {
            Some(k) => k,
            None => return false,
        };
        let value_name = to_wide(VALUE_NAME);
        // Query with null data pointer to check existence without reading data.
        let mut cb_data: u32 = 0;
        let err = unsafe {
            RegQueryValueExW(
                key.0,
                windows::core::PCWSTR(value_name.as_ptr()),
                None,           // lpreserved
                None,           // lptype  (don't care)
                None,           // lpdata  (just checking existence)
                Some(&mut cb_data),
            )
        };
        err == ERROR_SUCCESS
    }
}
