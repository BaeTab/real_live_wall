//! User settings persisted to %APPDATA%/real_live_wall/config.toml.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Settings that survive engine restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistConfig {
    /// Scene display name last selected; `None` = built-in default.
    pub scene: Option<String>,
    /// Audio capture source: `"auto"` | `"input"` | `"loopback"` | `"off"`.
    pub audio: String,
    /// Audio sensitivity multiplier applied to FFT magnitudes.
    pub gain: f32,
    /// Super-sampling factor (1.0 = off, 1.5–2.0 = crisper).
    pub ssaa: f32,
    /// Register the wallpaper engine in the Windows autostart registry key.
    pub autostart: bool,
    /// Auto-cycle through the scene list on a timer (playlist mode).
    #[serde(default)]
    pub playlist_enabled: bool,
    /// Seconds between automatic scene changes when the playlist is on.
    #[serde(default = "default_interval")]
    pub playlist_interval_secs: u64,
    /// Pick the next scene at random instead of in order.
    #[serde(default)]
    pub playlist_shuffle: bool,
}

/// Default playlist interval (5 minutes) — used for older config files that
/// predate the playlist fields.
fn default_interval() -> u64 {
    300
}

impl Default for PersistConfig {
    fn default() -> Self {
        Self {
            scene: None,
            audio: "auto".into(),
            gain: 6.0,
            ssaa: 1.5,
            autostart: false,
            playlist_enabled: false,
            playlist_interval_secs: default_interval(),
            playlist_shuffle: false,
        }
    }
}

impl PersistConfig {
    /// The resolved path to `config.toml`, or `None` if no suitable base dir
    /// could be found (extremely rare; cwd fallback almost always succeeds).
    pub fn path() -> Option<PathBuf> {
        // Prefer %APPDATA%, then %USERPROFILE%, then the current directory.
        let base = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .or_else(|| std::env::current_dir().ok())?;

        Some(base.join("real_live_wall").join("config.toml"))
    }

    /// Load from the resolved path.  Returns [`Default::default`] on any
    /// error (missing file, parse failure) without panicking.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            log::debug!("persist: no suitable base directory — using defaults");
            return Self::default();
        };

        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                log::debug!("persist: could not read {}: {e}", path.display());
                return Self::default();
            }
        };

        match toml::from_str::<Self>(&text) {
            Ok(cfg) => cfg,
            Err(e) => {
                log::info!("persist: parse error in {}: {e} — using defaults", path.display());
                Self::default()
            }
        }
    }

    /// Serialize to the resolved path (creating the directory tree if needed).
    /// Logs a warning on failure; never panics.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            log::warn!("persist: no suitable base directory — settings not saved");
            return;
        };

        // Ensure parent directory exists.
        if let Some(dir) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                log::warn!("persist: could not create {}: {e}", dir.display());
                return;
            }
        }

        let text = match toml::to_string_pretty(self) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("persist: serialization failed: {e}");
                return;
            }
        };

        if let Err(e) = std::fs::write(&path, text) {
            log::warn!("persist: could not write {}: {e}", path.display());
        }
    }
}
