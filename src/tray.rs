//! System-tray (notification-area) icon with a popup menu.
//!
//! Runs on its **own dedicated thread** with its own Win32 message loop so it
//! never touches the winit event loop. The public API is the same on every
//! platform; on non-Windows the spawn is a no-op.

// ---------------------------------------------------------------------------
// Public types (cross-platform)
// ---------------------------------------------------------------------------

/// Commands that the tray menu can emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    OpenSettings,
    NextScene,
    ToggleAutostart,
    Quit,
}

/// Opaque handle returned by [`spawn`].
///
/// Dropping it sends `WM_CLOSE` to the tray window (best-effort), which causes
/// the tray icon to be removed and the message loop to exit.
pub struct TrayHandle {
    #[cfg(windows)]
    hwnd: SendHwnd,
}

/// Spawn a tray icon on a new thread.
///
/// * `autostart_on`  – initial check-state of the "자동 시작" menu item.
/// * `on_cmd`        – callback invoked **from the tray thread** for each
///   menu action.
pub fn spawn(
    autostart_on: bool,
    on_cmd: Box<dyn Fn(TrayCommand) + Send + 'static>,
) -> anyhow::Result<TrayHandle> {
    #[cfg(windows)]
    {
        win::spawn(autostart_on, on_cmd)
    }
    #[cfg(not(windows))]
    {
        let _ = (autostart_on, on_cmd);
        Ok(TrayHandle {})
    }
}

// ---------------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::mem;

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{
        ExtractIconW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
        NOTIFYICONDATAW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DispatchMessageW,
        GetCursorPos, GetMessageW, GetWindowLongPtrW, LoadIconW, PostQuitMessage,
        RegisterClassW, SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu, TranslateMessage,
        CREATESTRUCTW, GWLP_USERDATA, HICON, HMENU, IDI_APPLICATION, MF_CHECKED, MF_SEPARATOR,
        MF_STRING, MF_UNCHECKED, MSG, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, WM_APP, WM_COMMAND,
        WM_DESTROY, WM_LBUTTONUP, WM_NCCREATE, WM_NCDESTROY, WM_RBUTTONUP, WNDCLASS_STYLES,
        WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_OVERLAPPED, HWND_MESSAGE,
    };

    use crate::tray::TrayCommand;

    // -----------------------------------------------------------------------
    // Send wrappers
    // -----------------------------------------------------------------------

    /// `HWND` is `*mut c_void` — not `Send` by default. We own it exclusively
    /// from the tray thread (for message delivery) so the wrapper is safe.
    pub(crate) struct SendHwnd(pub HWND);
    unsafe impl Send for SendHwnd {}

    // -----------------------------------------------------------------------
    // Per-window state stored in GWLP_USERDATA
    // -----------------------------------------------------------------------

    struct TrayState {
        on_cmd: Box<dyn Fn(TrayCommand)>,
        hmenu: HMENU,
    }

    // -----------------------------------------------------------------------
    // Tray-icon callback message id
    // -----------------------------------------------------------------------

    const WM_TRAY: u32 = WM_APP + 1;
    const TRAY_ICON_ID: u32 = 1;

    // -----------------------------------------------------------------------
    // WndProc
    // -----------------------------------------------------------------------

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            // ----------------------------------------------------------------
            // WM_NCCREATE — stash the TrayState pointer in GWLP_USERDATA.
            // The lParam for WM_NCCREATE is a *const CREATESTRUCTW.
            // ----------------------------------------------------------------
            m if m == WM_NCCREATE => {
                let cs = lparam.0 as *const CREATESTRUCTW;
                if !cs.is_null() {
                    let ptr = (*cs).lpCreateParams as isize;
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }

            // ----------------------------------------------------------------
            // WM_TRAY (WM_APP+1) — right- or left-click on tray icon.
            // ----------------------------------------------------------------
            m if m == WM_TRAY => {
                let event = lparam.0 as u32 & 0xFFFF;
                if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                    // Bring our (invisible) window to the foreground so the
                    // popup menu dismisses properly when the user clicks away.
                    let _ = SetForegroundWindow(hwnd);

                    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                    if ptr != 0 {
                        let state = &*(ptr as *const TrayState);
                        let mut pt = POINT::default();
                        let _ = GetCursorPos(&mut pt);
                        // TPM_BOTTOMALIGN|TPM_RIGHTBUTTON — menu appears above cursor
                        let _ = TrackPopupMenu(
                            state.hmenu,
                            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                            pt.x,
                            pt.y,
                            None,
                            hwnd,
                            None,
                        );
                    }
                }
                LRESULT(0)
            }

            // ----------------------------------------------------------------
            // WM_COMMAND — a menu item was selected.
            // ----------------------------------------------------------------
            m if m == WM_COMMAND => {
                let menu_id = wparam.0 as u32 & 0xFFFF;
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if ptr != 0 {
                    let state = &*(ptr as *const TrayState);
                    let cmd = match menu_id {
                        1 => Some(TrayCommand::OpenSettings),
                        2 => Some(TrayCommand::NextScene),
                        3 => Some(TrayCommand::ToggleAutostart),
                        9 => Some(TrayCommand::Quit),
                        _ => None,
                    };
                    if let Some(c) = cmd {
                        (state.on_cmd)(c);
                    }
                }
                LRESULT(0)
            }

            // ----------------------------------------------------------------
            // WM_DESTROY — remove tray icon and quit.
            // ----------------------------------------------------------------
            m if m == WM_DESTROY => {
                let mut nid: NOTIFYICONDATAW = mem::zeroed();
                nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = hwnd;
                nid.uID = TRAY_ICON_ID;
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                PostQuitMessage(0);
                LRESULT(0)
            }

            // ----------------------------------------------------------------
            // WM_NCDESTROY — last message; free the boxed TrayState.
            // ----------------------------------------------------------------
            m if m == WM_NCDESTROY => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
                if ptr != 0 {
                    // Re-take ownership so the box is dropped.
                    let _ = Box::from_raw(ptr as *mut TrayState);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    // -----------------------------------------------------------------------
    // spawn
    // -----------------------------------------------------------------------

    /// Wide string helper — copies up to N chars of a string literal into a
    /// fixed-size `[u16; N]` array, null-terminating.
    fn wide_arr<const N: usize>(s: &str) -> [u16; N] {
        let mut buf = [0u16; N];
        for (i, c) in s.encode_utf16().enumerate().take(N - 1) {
            buf[i] = c;
        }
        buf
    }

    pub fn spawn(
        autostart_on: bool,
        on_cmd: Box<dyn Fn(TrayCommand) + Send + 'static>,
    ) -> anyhow::Result<crate::tray::TrayHandle> {
        // Channel to get the HWND back from the thread after creation.
        let (tx, rx) = std::sync::mpsc::channel::<Result<SendHwnd, String>>();

        std::thread::Builder::new()
            .name("rlw-tray".into())
            .spawn(move || {
                // ----------------------------------------------------------------
                // Get HINSTANCE
                // ----------------------------------------------------------------
                let hinstance: HINSTANCE = match unsafe { GetModuleHandleW(PCWSTR::null()) } {
                    Ok(h) => HINSTANCE::from(h),
                    Err(e) => {
                        let _ = tx.send(Err(format!("GetModuleHandleW failed: {e}")));
                        return;
                    }
                };

                // ----------------------------------------------------------------
                // Load icon: try the exe's own embedded icon first.
                // ----------------------------------------------------------------
                let hicon: HICON = unsafe {
                    let mut icon = HICON(std::ptr::null_mut());
                    if let Ok(exe_path) = std::env::current_exe() {
                        // Build a wide string for the exe path.
                        let wide_path: Vec<u16> = exe_path
                            .to_string_lossy()
                            .encode_utf16()
                            .chain(std::iter::once(0u16))
                            .collect();
                        let pcwstr = PCWSTR(wide_path.as_ptr());
                        let extracted = ExtractIconW(Some(hinstance), pcwstr, 0);
                        if !extracted.is_invalid() {
                            icon = extracted;
                        }
                    }
                    if icon.is_invalid() {
                        // Fall back to the generic application icon.
                        if let Ok(h) = LoadIconW(None, IDI_APPLICATION) {
                            icon = h;
                        }
                    }
                    icon
                };

                // ----------------------------------------------------------------
                // Build popup menu.
                // ----------------------------------------------------------------
                let hmenu: HMENU = unsafe {
                    let m = match CreatePopupMenu() {
                        Ok(h) => h,
                        Err(e) => {
                            let _ = tx.send(Err(format!("CreatePopupMenu failed: {e}")));
                            return;
                        }
                    };
                    let autostart_flag = if autostart_on { MF_CHECKED } else { MF_UNCHECKED };
                    let _ = AppendMenuW(m, MF_STRING, 1, w!("설정 열기"));
                    let _ = AppendMenuW(m, MF_STRING, 2, w!("다음 씬"));
                    let _ = AppendMenuW(m, MF_STRING | autostart_flag, 3, w!("자동 시작"));
                    let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
                    let _ = AppendMenuW(m, MF_STRING, 9, w!("종료"));
                    m
                };

                // ----------------------------------------------------------------
                // Box the TrayState and pass a raw ptr through lpParam.
                // ----------------------------------------------------------------
                let state = Box::new(TrayState {
                    on_cmd: Box::new(on_cmd),
                    hmenu,
                });
                let state_ptr = Box::into_raw(state) as *const c_void;

                // ----------------------------------------------------------------
                // Register window class.
                // ----------------------------------------------------------------
                let class_name = w!("real_live_wall_tray");
                unsafe {
                    let wc = WNDCLASSW {
                        style: WNDCLASS_STYLES(0),
                        lpfnWndProc: Some(wnd_proc),
                        cbClsExtra: 0,
                        cbWndExtra: 0,
                        hInstance: hinstance,
                        hIcon: hicon,
                        hCursor: Default::default(),
                        hbrBackground: Default::default(),
                        lpszMenuName: PCWSTR::null(),
                        lpszClassName: class_name,
                    };
                    // Return value 0 means failure, but we ignore it here because
                    // re-registering the same class returns 0 as well (already exists).
                    RegisterClassW(&wc);
                }

                // ----------------------------------------------------------------
                // Create message-only window.
                // HWND_MESSAGE is a pseudo-parent that makes the window message-only
                // (no screen presence, no z-order). Value is -3.
                // ----------------------------------------------------------------
                let hwnd = unsafe {
                    match CreateWindowExW(
                        WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                        class_name,
                        w!(""),
                        WS_OVERLAPPED,
                        0,
                        0,
                        0,
                        0,
                        Some(HWND_MESSAGE),
                        None,
                        Some(hinstance),
                        Some(state_ptr),
                    ) {
                        Ok(h) => h,
                        Err(e) => {
                            // Reclaim the state box so it's not leaked.
                            let _ = Box::from_raw(state_ptr as *mut TrayState);
                            let _ = tx.send(Err(format!("CreateWindowExW failed: {e}")));
                            return;
                        }
                    }
                };

                // ----------------------------------------------------------------
                // Add tray icon.
                // ----------------------------------------------------------------
                unsafe {
                    let tip = wide_arr::<128>("real_live_wall");
                    let mut nid: NOTIFYICONDATAW = mem::zeroed();
                    nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
                    nid.hWnd = hwnd;
                    nid.uID = TRAY_ICON_ID;
                    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
                    nid.uCallbackMessage = WM_TRAY;
                    nid.hIcon = hicon;
                    nid.szTip = tip;
                    let _ = Shell_NotifyIconW(NIM_ADD, &nid);
                }

                // Send HWND back to spawn() so it can build the TrayHandle.
                let _ = tx.send(Ok(SendHwnd(hwnd)));

                // ----------------------------------------------------------------
                // Message loop.
                // ----------------------------------------------------------------
                unsafe {
                    let mut msg = MSG::default();
                    loop {
                        // GetMessageW returns BOOL(0) for WM_QUIT, BOOL(-1) for error.
                        let ret = GetMessageW(&mut msg, None, 0, 0);
                        if ret.0 == 0 {
                            break; // WM_QUIT
                        }
                        if ret.0 == -1 {
                            log::warn!("tray: GetMessageW error");
                            break;
                        }
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }

                // Clean up menu (state box freed in WM_NCDESTROY; icon freed below).
                // NIM_DELETE was already sent in WM_DESTROY; DestroyMenu cleans the menu.
                // The icon handle loaded from the exe should not be destroyed (shell32
                // owns the returned HICON from ExtractIconW), but if we fell back to
                // LoadIconW the handle is shared and also must not be destroyed.
            })?;

        // Receive the HWND (or error) from the tray thread.
        let send_hwnd = rx
            .recv()
            .map_err(|_| anyhow::anyhow!("tray thread exited before sending HWND"))?
            .map_err(|e| anyhow::anyhow!("tray thread error: {e}"))?;

        Ok(crate::tray::TrayHandle { hwnd: send_hwnd })
    }
}

// ---------------------------------------------------------------------------
// Drop for TrayHandle
// ---------------------------------------------------------------------------

#[cfg(windows)]
use win::SendHwnd;

#[cfg(windows)]
impl Drop for TrayHandle {
    fn drop(&mut self) {
        // Best-effort: ask the tray thread to tear down and exit.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(self.hwnd.0),
                windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            );
        }
    }
}
