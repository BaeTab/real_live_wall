//! Build script: on Windows, embed the app icon (and version info) into the exe
//! so it shows in Explorer, the taskbar and the window title bar.

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            // Don't fail the build just because the icon couldn't be embedded.
            println!("cargo:warning=icon embed skipped: {e}");
        }
    }
}
