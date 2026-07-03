//! Desktop-wallpaper surface acquisition, abstracted per OS.
//!
//! * Windows: spawn the `WorkerW` window behind the desktop icons and re-parent
//!   our borderless window into it (the technique used by Lively / Wallpaper
//!   Engine style tools).
//! * macOS / Linux: not yet wired up — the window is shown normally with a
//!   warning, so the engine still runs everywhere.

use winit::window::Window;

/// Re-parent `window` so it renders as the live desktop wallpaper.
pub fn attach_to_desktop(window: &Window) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        win::attach(window)
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        log::warn!(
            "wallpaper attach is not implemented on this platform yet; showing a normal window"
        );
        Ok(())
    }
}

#[cfg(windows)]
mod win {
    use anyhow::{anyhow, bail};
    use std::ffi::c_void;

    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::window::Window;

    use windows::core::{w, BOOL, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, FindWindowExW, FindWindowW, GetSystemMetrics, SendMessageTimeoutW, SetParent,
        SetWindowPos, HWND_BOTTOM, SMTO_NORMAL, SM_CXSCREEN, SM_CYSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    };

    /// Message that asks Progman to spawn a `WorkerW` behind the icons.
    const WM_SPAWN_WORKERW: u32 = 0x052C;

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

    pub fn attach(window: &Window) -> anyhow::Result<()> {
        let child = hwnd_of(window)?;
        unsafe {
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

            let parent = if data.worker.0.is_null() { progman } else { data.worker };
            SetParent(child, Some(parent)).map_err(|e| anyhow!("SetParent failed: {e}"))?;

            // Cover exactly the primary monitor. WorkerW's client origin is the
            // virtual-desktop top-left, so the primary monitor sits at
            // (-SM_XVIRTUALSCREEN, -SM_YVIRTUALSCREEN) in that space. Drop to the
            // bottom of the z-order so the desktop icons stay on top.
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let pw = GetSystemMetrics(SM_CXSCREEN);
            let ph = GetSystemMetrics(SM_CYSCREEN);
            let _ = SetWindowPos(
                child,
                Some(HWND_BOTTOM),
                -vx,
                -vy,
                pw,
                ph,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            );
            log::info!("wallpaper: attached to primary monitor ({pw}x{ph} at {},{})", -vx, -vy);
        }
        Ok(())
    }
}
