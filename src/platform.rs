//! Desktop-wallpaper surface acquisition and lifecycle, abstracted per OS.
//!
//! * Windows: spawn the `WorkerW` window behind the desktop icons and re-parent
//!   one borderless window per monitor into it (the technique used by Lively /
//!   Wallpaper Engine style tools). A named event lets any other instance of the
//!   exe (or the preview panel) ask a running wallpaper to exit cleanly.
//! * macOS / Linux: not yet wired up — windows are shown normally with a
//!   warning, so the engine still runs everywhere.

use winit::window::Window;

/// A monitor's rectangle in virtual-desktop (physical pixel) coordinates.
#[derive(Clone, Copy, Debug)]
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Re-parent `window` so it renders as the live desktop wallpaper covering the
/// monitor described by `rect`.
pub fn attach_to_desktop(window: &Window, rect: MonitorRect) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        win::attach(window, rect)
    }
    #[cfg(not(windows))]
    {
        let _ = (window, rect);
        log::warn!(
            "wallpaper attach is not implemented on this platform yet; showing a normal window"
        );
        Ok(())
    }
}

/// Called by a wallpaper process: register the stop channel and run `on_stop`
/// (once) from a background thread when another instance requests a stop. Keep
/// the returned guard alive for the process lifetime.
pub fn watch_for_stop<F: FnOnce() + Send + 'static>(on_stop: F) -> Option<StopGuard> {
    #[cfg(windows)]
    {
        win::watch_for_stop(on_stop)
    }
    #[cfg(not(windows))]
    {
        let _ = on_stop;
        None
    }
}

/// Ask a running wallpaper process to exit. Returns true if one was signalled.
pub fn signal_stop() -> bool {
    #[cfg(windows)]
    {
        win::signal_stop()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Whether a wallpaper process is currently running (advertising its stop event).
pub fn wallpaper_running() -> bool {
    #[cfg(windows)]
    {
        win::wallpaper_running()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Best-effort: force the shell to repaint the static desktop wallpaper. Call
/// after a wallpaper process tears its windows down so no black area is left.
pub fn restore_desktop() {
    #[cfg(windows)]
    {
        win::restore_desktop();
    }
}

/// Opaque handle that keeps the stop-watcher alive. Dropping it does not stop
/// the watcher thread (which owns the event handle); it just marks ownership.
pub struct StopGuard {
    #[cfg(windows)]
    _private: (),
}

#[cfg(windows)]
mod win {
    use anyhow::{anyhow, bail};
    use std::ffi::c_void;

    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    use windows::core::{w, BOOL, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::{
        CreateEventW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, FindWindowExW, FindWindowW, GetSystemMetrics, SendMessageTimeoutW, SetParent,
        SetWindowPos, SystemParametersInfoW, HWND_BOTTOM, SMTO_NORMAL, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN, SPIF_SENDWININICHANGE, SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER,
        SWP_NOACTIVATE, SWP_SHOWWINDOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    use super::MonitorRect;

    /// Message that asks Progman to spawn a `WorkerW` behind the icons.
    const WM_SPAWN_WORKERW: u32 = 0x052C;

    /// Session-local name of the wallpaper stop event (bump the suffix if the
    /// protocol ever changes).
    fn stop_event_name() -> PCWSTR {
        w!("real_live_wall_stop_event_v1")
    }

    struct FindData {
        worker: HWND,
    }

    unsafe extern "system" fn enum_proc(top: HWND, lparam: LPARAM) -> BOOL {
        let data = &mut *(lparam.0 as *mut FindData);
        // The desktop's `SHELLDLL_DefView` lives under one specific top window;
        // its sibling `WorkerW` is the layer we want to draw into.
        if FindWindowExW(Some(top), None, w!("SHELLDLL_DefView"), PCWSTR::null()).is_ok() {
            if let Ok(worker) = FindWindowExW(None, Some(top), w!("WorkerW"), PCWSTR::null()) {
                data.worker = worker;
            }
        }
        BOOL(1)
    }

    fn hwnd_of(window: &Window) -> anyhow::Result<HWND> {
        let handle = window.window_handle()?.as_raw();
        match handle {
            RawWindowHandle::Win32(h) => Ok(HWND(h.hwnd.get() as *mut c_void)),
            other => bail!("expected a Win32 window handle, got {other:?}"),
        }
    }

    /// Locate (spawning if needed) the `WorkerW` layer behind the desktop icons.
    unsafe fn find_workerw() -> anyhow::Result<HWND> {
        let progman =
            FindWindowW(w!("Progman"), PCWSTR::null()).map_err(|e| anyhow!("Progman not found: {e}"))?;

        // Ask the shell to create the WorkerW layer.
        let mut result: usize = 0;
        SendMessageTimeoutW(
            progman,
            WM_SPAWN_WORKERW,
            WPARAM(0),
            LPARAM(0),
            SMTO_NORMAL,
            1000,
            Some(&mut result as *mut usize),
        );

        // Find the WorkerW behind the desktop icons.
        let mut data = FindData {
            worker: HWND(std::ptr::null_mut()),
        };
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut data as *mut _ as isize));
        // Win11 fallback: WorkerW can be a direct child of Progman.
        if data.worker.0.is_null() {
            if let Ok(w) = FindWindowExW(Some(progman), None, w!("WorkerW"), PCWSTR::null()) {
                data.worker = w;
            }
        }
        Ok(if data.worker.0.is_null() { progman } else { data.worker })
    }

    pub fn attach(window: &Window, rect: MonitorRect) -> anyhow::Result<()> {
        let child = hwnd_of(window)?;
        unsafe {
            let parent = find_workerw()?;
            SetParent(child, Some(parent)).map_err(|e| anyhow!("SetParent failed: {e}"))?;

            // WorkerW's client origin is the virtual-desktop top-left, i.e. the
            // desktop point (SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN). Convert this
            // monitor's desktop rect into that space and drop to the bottom of the
            // z-order so the desktop icons stay on top.
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let _ = SetWindowPos(
                child,
                Some(HWND_BOTTOM),
                rect.x - vx,
                rect.y - vy,
                rect.w,
                rect.h,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            );
            log::info!(
                "wallpaper: attached monitor {}x{} at desktop ({},{})",
                rect.w,
                rect.h,
                rect.x,
                rect.y
            );
        }
        Ok(())
    }

    // --- stop channel -------------------------------------------------------

    /// A `HANDLE` we promise to only touch from the owning thread.
    struct SendHandle(HANDLE);
    unsafe impl Send for SendHandle {}

    pub fn watch_for_stop<F: FnOnce() + Send + 'static>(on_stop: F) -> Option<super::StopGuard> {
        // Auto-reset, initially non-signalled. If it already exists another
        // wallpaper owns it; CreateEventW still returns a usable handle.
        let event = unsafe { CreateEventW(None, false, false, stop_event_name()) };
        let event = match event {
            Ok(h) if !h.is_invalid() => SendHandle(h),
            _ => {
                log::warn!("wallpaper: could not create stop event; remote stop disabled");
                return None;
            }
        };

        std::thread::Builder::new()
            .name("rlw-stop-watcher".into())
            .spawn(move || {
                let handle = event; // move ownership into the thread
                // Block until another instance signals the event.
                let r = unsafe { WaitForSingleObject(handle.0, INFINITE) };
                if r.0 == 0 {
                    log::info!("wallpaper: stop requested");
                    on_stop();
                }
                unsafe {
                    let _ = CloseHandle(handle.0);
                }
            })
            .ok()?;

        Some(super::StopGuard { _private: () })
    }

    pub fn signal_stop() -> bool {
        unsafe {
            match OpenEventW(EVENT_MODIFY_STATE, false, stop_event_name()) {
                Ok(h) if !h.is_invalid() => {
                    let ok = SetEvent(h).is_ok();
                    let _ = CloseHandle(h);
                    ok
                }
                _ => false,
            }
        }
    }

    pub fn wallpaper_running() -> bool {
        unsafe {
            match OpenEventW(EVENT_MODIFY_STATE, false, stop_event_name()) {
                Ok(h) if !h.is_invalid() => {
                    let _ = CloseHandle(h);
                    true
                }
                _ => false,
            }
        }
    }

    pub fn restore_desktop() {
        // Read the current wallpaper path, then set it again so the shell
        // repaints the desktop where our windows used to be.
        let mut buf = [0u16; 260];
        unsafe {
            let _ = SystemParametersInfoW(
                SPI_GETDESKWALLPAPER,
                buf.len() as u32,
                Some(buf.as_mut_ptr() as *mut c_void),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            let _ = SystemParametersInfoW(
                SPI_SETDESKWALLPAPER,
                0,
                Some(buf.as_ptr() as *mut c_void),
                SPIF_SENDWININICHANGE,
            );
        }
    }
}
