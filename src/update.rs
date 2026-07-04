//! Self-update via GitHub Releases.
//!
//! [`check`] polls `GET /repos/BaeTab/real_live_wall/releases/latest` and
//! compares its `tag_name` against the running binary's version. On Windows,
//! [`download_and_apply`] can then fetch the release's Windows zip asset and
//! stage an in-place swap: it downloads + extracts the asset, writes a small
//! detached helper batch script that waits for this process to exit, mirrors
//! the extracted files over the install directory, and relaunches the exe.
//! Everything here is best-effort and never panics — network failures,
//! missing assets, and I/O errors all surface as `None`/`Err`.
//!
//! Not yet wired into `app`/`ui`/`tray` — the public API below is this
//! module's contract for that integration, so `dead_code` is silenced here
//! rather than left to warn until that call site exists.
#![allow(dead_code)]

use std::time::Duration;

use serde::Deserialize;

/// GitHub REST endpoint for this project's latest (non-prerelease, non-draft) release.
const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/BaeTab/real_live_wall/releases/latest";

/// GitHub requires a `User-Agent` header on API requests.
const USER_AGENT: &str = "real_live_wall-updater";

/// Budget for the small metadata request.
const CHECK_TIMEOUT: Duration = Duration::from_secs(8);

/// Everything the caller needs to tell the user about (and optionally
/// install) an available update.
#[derive(Clone, Debug)]
pub struct UpdateInfo {
    /// Release version with any leading `v`/`V` stripped, e.g. `"1.2.0"`.
    pub version: String,
    /// The raw tag name as published, e.g. `"v1.2.0"`.
    pub tag: String,
    /// Release notes (the release body; may be empty and may be long/Markdown).
    pub notes: String,
    /// Link to the release page on GitHub.
    pub html_url: String,
    /// `browser_download_url` of the Windows zip asset, if the release has one.
    pub asset_url: Option<String>,
    /// File name of that asset, e.g. `"real_live_wall-v1.2.0-windows-x64.zip"`.
    pub asset_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

/// Query GitHub for the latest release and return `Some(UpdateInfo)` only if
/// it is **strictly newer** than `current` (e.g. `env!("CARGO_PKG_VERSION")`).
///
/// Returns `None` on any failure — no network, non-2xx response, malformed
/// JSON, or simply "already up to date". Never panics.
pub fn check(current: &str) -> Option<UpdateInfo> {
    let release = fetch_latest_release()?;

    let version = normalize_version(&release.tag_name);
    if !is_newer(&version, current) {
        log::debug!("update check: {version} is not newer than {current}");
        return None;
    }

    let asset = pick_windows_asset(&release.assets);
    let (asset_url, asset_name) = match asset {
        Some(a) => (Some(a.browser_download_url.clone()), Some(a.name.clone())),
        None => (None, None),
    };

    log::info!("update check: found newer release {version} (current {current})");
    Some(UpdateInfo {
        version,
        tag: release.tag_name,
        notes: release.body.unwrap_or_default(),
        html_url: release.html_url,
        asset_url,
        asset_name,
    })
}

/// Run [`check`] on a background thread so the caller (typically an event
/// loop or GUI) never blocks on network I/O. `cb` runs exactly once, on that
/// background thread, with the result.
pub fn check_async(current: &'static str, cb: impl FnOnce(Option<UpdateInfo>) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("rlw-update-check".into())
        .spawn(move || cb(check(current)));

    if let Err(e) = spawned {
        log::warn!("update: failed to spawn background check thread: {e}");
    }
}

/// Download the release's Windows zip asset and **stage** a self-update.
///
/// This does not itself replace any files or exit the process. On success it
/// has: downloaded + extracted the asset under `%TEMP%\rlw_update\<version>`,
/// and spawned a detached helper batch script that waits for this process
/// (by PID) to exit, then mirrors the extracted files over the install
/// directory (the folder containing the current exe) and relaunches it. The
/// **caller is responsible for exiting the process afterward** — this
/// function only stages the swap.
///
/// Returns `Err` if the release has no Windows asset, the download/extract
/// fails, or (on non-Windows platforms) unconditionally, since there is no
/// swap implementation there.
pub fn download_and_apply(info: &UpdateInfo) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        win::download_and_apply(info)
    }
    #[cfg(not(windows))]
    {
        let _ = info;
        anyhow::bail!("auto-apply not supported on this platform")
    }
}

// ---------------------------------------------------------------------------
// Metadata fetch + version comparison (cross-platform; HTTP works everywhere)
// ---------------------------------------------------------------------------

fn fetch_latest_release() -> Option<GhRelease> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(CHECK_TIMEOUT))
        .build();
    let agent = ureq::Agent::new_with_config(config);

    let mut response = match agent
        .get(RELEASES_LATEST_URL)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(r) => r,
        Err(e) => {
            log::debug!("update check: request to {RELEASES_LATEST_URL} failed: {e}");
            return None;
        }
    };

    let body = match response.body_mut().read_to_string() {
        Ok(b) => b,
        Err(e) => {
            log::debug!("update check: could not read response body: {e}");
            return None;
        }
    };

    match serde_json::from_str::<GhRelease>(&body) {
        Ok(release) => Some(release),
        Err(e) => {
            log::debug!("update check: could not parse release JSON: {e}");
            None
        }
    }
}

/// Prefer an asset whose name mentions "windows"; fall back to any `.zip`.
fn pick_windows_asset(assets: &[GhAsset]) -> Option<&GhAsset> {
    assets
        .iter()
        .find(|a| a.name.to_lowercase().contains("windows"))
        .or_else(|| assets.iter().find(|a| a.name.to_lowercase().ends_with(".zip")))
}

/// Strip a leading `v`/`V` and surrounding whitespace from a tag/version string.
fn normalize_version(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

/// Split a dotted version string into numeric components, ignoring any
/// `-prerelease`/`+build` suffix. Missing or non-numeric components become 0
/// rather than causing a parse failure — this is deliberately forgiving since
/// it only ever runs on tags this project itself published.
fn parse_version(v: &str) -> Vec<u64> {
    let core = v.split(['-', '+']).next().unwrap_or(v);
    core.split('.')
        .map(|part| part.trim().parse::<u64>().unwrap_or(0))
        .collect()
}

/// True if `candidate` is strictly greater than `base` under numeric,
/// dot-separated comparison (shorter version strings are zero-padded).
fn is_newer(candidate: &str, base: &str) -> bool {
    let c = parse_version(candidate);
    let b = parse_version(base);
    for i in 0..c.len().max(b.len()) {
        let cv = c.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        if cv != bv {
            return cv > bv;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Windows implementation: download, extract, stage the swap
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use std::fs::File;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use anyhow::Context;

    use super::UpdateInfo;

    /// Release zips are tens of MB; give the download far more room than the
    /// metadata check.
    const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

    /// How many ~2s polls the helper script waits for this process to exit
    /// before giving up and applying the update anyway (best-effort safety
    /// valve against a caller that forgets to exit).
    const MAX_WAIT_TRIES: u32 = 150;

    pub fn download_and_apply(info: &UpdateInfo) -> anyhow::Result<()> {
        let asset_url = info
            .asset_url
            .as_deref()
            .context("release has no downloadable Windows asset")?;

        let current_exe =
            std::env::current_exe().context("could not resolve the current exe's path")?;
        let install_dir = current_exe
            .parent()
            .context("current exe path has no parent directory")?
            .to_path_buf();

        // Stage under %TEMP%\rlw_update\<version>\ so a retried/older attempt
        // never mixes files with this one.
        let stage_root = std::env::temp_dir().join("rlw_update").join(&info.version);
        if stage_root.exists() {
            let _ = std::fs::remove_dir_all(&stage_root);
        }
        let extract_dir = stage_root.join("extracted");
        std::fs::create_dir_all(&extract_dir)
            .with_context(|| format!("could not create staging dir {}", extract_dir.display()))?;

        let asset_name = info.asset_name.as_deref().unwrap_or("update.zip");
        let zip_path = stage_root.join(asset_name);
        download_to_file(asset_url, &zip_path)
            .with_context(|| format!("failed to download {asset_url}"))?;

        extract_zip(&zip_path, &extract_dir)
            .with_context(|| format!("failed to extract {}", zip_path.display()))?;

        // The release convention is a flat zip root (exe + shaders/ + docs),
        // but tolerate a single wrapping top-level folder defensively.
        let staged_exe = find_staged_exe(&extract_dir)
            .context("update archive did not contain real_live_wall.exe")?;
        let copy_source = staged_exe.parent().unwrap_or(&extract_dir).to_path_buf();

        let target_exe = install_dir.join("real_live_wall.exe");
        let pid = std::process::id();
        let script = build_batch_script(pid, &copy_source, &install_dir, &target_exe);

        let batch_path = stage_root.join("apply_update.bat");
        std::fs::write(&batch_path, script)
            .with_context(|| format!("could not write helper script {}", batch_path.display()))?;

        spawn_detached_batch(&batch_path).context("failed to launch the update helper script")?;

        log::info!(
            "update: staged {} -> {} (helper: {})",
            copy_source.display(),
            install_dir.display(),
            batch_path.display()
        );
        Ok(())
    }

    fn download_to_file(url: &str, dest: &Path) -> anyhow::Result<()> {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(DOWNLOAD_TIMEOUT))
            .build();
        let agent = ureq::Agent::new_with_config(config);

        let mut response = agent
            .get(url)
            .header("User-Agent", super::USER_AGENT)
            .call()
            .map_err(|e| anyhow::anyhow!("download request failed: {e}"))?;

        let mut file =
            File::create(dest).with_context(|| format!("could not create {}", dest.display()))?;
        let mut reader = response.body_mut().as_reader();
        std::io::copy(&mut reader, &mut file).context("could not write downloaded data to disk")?;
        file.flush().ok();
        Ok(())
    }

    fn extract_zip(zip_path: &Path, dest: &Path) -> anyhow::Result<()> {
        let file = File::open(zip_path)
            .with_context(|| format!("could not open {}", zip_path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .with_context(|| format!("{} is not a valid zip archive", zip_path.display()))?;
        archive
            .extract(dest)
            .with_context(|| format!("could not extract into {}", dest.display()))
    }

    /// Find `real_live_wall.exe` directly under `root`, or one level down if
    /// the archive wraps everything in a single top-level folder.
    fn find_staged_exe(root: &Path) -> Option<PathBuf> {
        let direct = root.join("real_live_wall.exe");
        if direct.is_file() {
            return Some(direct);
        }
        for entry in std::fs::read_dir(root).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let nested = path.join("real_live_wall.exe");
                if nested.is_file() {
                    return Some(nested);
                }
            }
        }
        None
    }

    /// Render the helper batch script.
    ///
    /// It: (1) polls for `pid` to disappear from `tasklist` (bounded — after
    /// [`MAX_WAIT_TRIES`] it proceeds anyway), (2) mirrors `src`'s contents
    /// over `dst` with `robocopy` — unlike `xcopy`, robocopy always copies a
    /// directory's *contents* into the destination and never nests `src` as a
    /// subdirectory when `dst` already exists (the classic xcopy gotcha,
    /// which would matter here since `dst` is the already-existing install
    /// directory), (3) relaunches `exe`, then (4) best-effort deletes the
    /// downloaded zip and extracted tree (the tiny helper script itself is
    /// left behind in the staging folder rather than trying to delete the
    /// batch file cmd.exe is currently executing).
    fn build_batch_script(pid: u32, src: &Path, dst: &Path, exe: &Path) -> String {
        let src = src.display();
        let dst = dst.display();
        let exe = exe.display();
        format!(
            "@echo off\r\n\
             setlocal EnableDelayedExpansion\r\n\
             set \"PID={pid}\"\r\n\
             set \"TRIES=0\"\r\n\
             \r\n\
             :wait\r\n\
             tasklist /FI \"PID eq %PID%\" 2>NUL | find /I \"%PID%\" >NUL\r\n\
             if not errorlevel 1 (\r\n\
             \tset /a TRIES+=1\r\n\
             \tif !TRIES! GEQ {max_tries} goto apply\r\n\
             \tping -n 2 127.0.0.1 >NUL\r\n\
             \tgoto wait\r\n\
             )\r\n\
             \r\n\
             :apply\r\n\
             robocopy \"{src}\" \"{dst}\" /E /R:5 /W:1 >NUL\r\n\
             start \"\" \"{exe}\"\r\n\
             \r\n\
             rmdir /S /Q \"{src}\" >NUL 2>&1\r\n",
            pid = pid,
            max_tries = MAX_WAIT_TRIES,
            src = src,
            dst = dst,
            exe = exe,
        )
    }

    fn spawn_detached_batch(batch_path: &Path) -> anyhow::Result<()> {
        use std::os::windows::process::CommandExt;

        // CREATE_NO_WINDOW: suppress the cmd.exe console flash. Debug builds
        // have a console (release builds are already windowed), so this
        // matters most for `cargo run` testing.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        std::process::Command::new("cmd")
            .args(["/C", &batch_path.to_string_lossy()])
            .current_dir(batch_path.parent().unwrap_or(batch_path))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .context("could not spawn cmd.exe")?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests for the pure version-comparison logic (no network access).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_leading_v() {
        assert_eq!(normalize_version("v1.2.0"), "1.2.0");
        assert_eq!(normalize_version("V1.2.0"), "1.2.0");
        assert_eq!(normalize_version(" 1.2.0 "), "1.2.0");
        assert_eq!(normalize_version("1.2.0"), "1.2.0");
    }

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("1.2.0", "1.1.0"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(is_newer("1.1.0", "1.0.9"));
        assert!(is_newer("1.2", "1.1.9"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(!is_newer("1.0.9", "1.1.0"));
        assert!(!is_newer("1.1.0", "1.1.0.1"));
    }

    #[test]
    fn ignores_prerelease_and_build_suffixes() {
        assert!(is_newer("1.2.0-beta.1", "1.1.0"));
        assert!(!is_newer("1.2.0-beta.1", "1.2.0"));
        assert_eq!(parse_version("1.2.0+build5"), vec![1, 2, 0]);
    }

    #[test]
    fn picks_windows_zip_asset() {
        let assets = vec![
            GhAsset {
                name: "source.tar.gz".into(),
                browser_download_url: "https://example.com/source.tar.gz".into(),
            },
            GhAsset {
                name: "real_live_wall-v1.2.0-windows-x64.zip".into(),
                browser_download_url: "https://example.com/win.zip".into(),
            },
        ];
        let picked = pick_windows_asset(&assets).expect("should find windows asset");
        assert_eq!(picked.name, "real_live_wall-v1.2.0-windows-x64.zip");
    }
}
