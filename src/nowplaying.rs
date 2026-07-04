//! Windows SMTC (System Media Transport Controls) now-playing info.
//!
//! Polls the current system media session roughly once a second on a
//! dedicated background thread and exposes a cheap-to-clone snapshot
//! ([`NowPlaying`]) that the reactive/render layer can read every frame
//! without ever touching WinRT directly.
//!
//! Also extracts a 4-colour palette from the current track's album art (when
//! present) via a coarse histogram over sparsely-sampled pixels — no k-means,
//! just bucket-count + saturation-weighted ranking.
//!
//! Everything here is best-effort: no SMTC session, no media properties, no
//! thumbnail, or a decode failure all just fall back to sane defaults. The
//! poll thread never panics and never tears itself down on error.

#![allow(dead_code)]

/// A snapshot of the current SMTC now-playing state.
#[derive(Clone)]
pub struct NowPlaying {
    /// Whether an SMTC session currently exists (some app is registered as a
    /// media source), regardless of play/pause state.
    pub has_music: bool,
    /// Whether that session is actively playing (not paused/stopped).
    pub is_playing: bool,
    /// True only for the single poll cycle in which the current track (by
    /// title+artist) was first observed to differ from the previous poll's
    /// track. It is reset to `false` again on the very next poll regardless
    /// of how many times [`NowPlayingHandle::latest`] is called in between —
    /// see the poll-thread loop in `win::spawn` for the exact semantics.
    pub track_changed: bool,
    /// Up to 4 representative colours from the album art (sRGB, 0..1),
    /// `[0]` being the most prominent. Falls back to a fixed, app-themed
    /// palette when there is no music or no usable thumbnail.
    pub palette: [[f32; 3]; 4],
    pub title: String,
    pub artist: String,
}

/// Fallback palette used whenever there's no session, no thumbnail, or the
/// thumbnail failed to decode. Chosen to blend with the app's own look:
/// deep teal, aurora green, violet, soft blue.
const DEFAULT_PALETTE: [[f32; 3]; 4] = [
    [0.086, 0.325, 0.318],
    [0.204, 0.584, 0.431],
    [0.408, 0.310, 0.612],
    [0.294, 0.478, 0.702],
];

impl Default for NowPlaying {
    fn default() -> Self {
        Self {
            has_music: false,
            is_playing: false,
            track_changed: false,
            palette: DEFAULT_PALETTE,
            title: String::new(),
            artist: String::new(),
        }
    }
}

/// Handle to the background SMTC poll thread. Dropping it does not stop the
/// thread (it's a daemon-style loop for the process lifetime); it just marks
/// ownership of the shared snapshot.
pub struct NowPlayingHandle {
    #[cfg(windows)]
    latest: std::sync::Arc<std::sync::Mutex<NowPlaying>>,
}

impl NowPlayingHandle {
    /// Spawn the background polling thread. Never panics; if the thread
    /// itself fails to spawn, `latest()` will just keep returning the
    /// default (no-music) snapshot forever.
    pub fn spawn() -> Self {
        #[cfg(windows)]
        {
            win::spawn()
        }
        #[cfg(not(windows))]
        {
            Self {}
        }
    }

    /// Cheaply clone the latest snapshot seen by the poll thread.
    pub fn latest(&self) -> NowPlaying {
        #[cfg(windows)]
        {
            self.latest.lock().map(|g| g.clone()).unwrap_or_default()
        }
        #[cfg(not(windows))]
        {
            NowPlaying::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
        ExifOrientationMode,
    };
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager as SessionManager,
        GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProperties,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
    };

    use super::{NowPlaying, DEFAULT_PALETTE};

    // -----------------------------------------------------------------------
    // spawn / poll loop
    // -----------------------------------------------------------------------

    const POLL_INTERVAL: Duration = Duration::from_secs(1);

    pub fn spawn() -> super::NowPlayingHandle {
        let latest = Arc::new(Mutex::new(NowPlaying::default()));
        let shared = latest.clone();

        let spawned = std::thread::Builder::new()
            .name("rlw-nowplaying".into())
            .spawn(move || {
                // Hash of the last track (title+artist) we observed, so we
                // can flag the single poll cycle where it changes.
                let mut last_track_hash: Option<u64> = None;

                loop {
                    match poll_once() {
                        Ok(mut snapshot) => {
                            let hash = track_hash(&snapshot.title, &snapshot.artist);
                            let current = if snapshot.has_music { Some(hash) } else { None };
                            snapshot.track_changed = snapshot.has_music && current != last_track_hash;
                            last_track_hash = current;

                            if let Ok(mut guard) = shared.lock() {
                                *guard = snapshot;
                            }
                        }
                        Err(e) => {
                            // Best-effort: log and keep whatever snapshot we
                            // already had (do not touch `shared`, do not
                            // reset `last_track_hash`).
                            log::warn!("nowplaying: poll failed ({e}); keeping previous state");
                        }
                    }

                    std::thread::sleep(POLL_INTERVAL);
                }
            });

        if let Err(e) = spawned {
            log::warn!("nowplaying: failed to spawn poll thread: {e}; now-playing disabled");
        }

        super::NowPlayingHandle { latest }
    }

    /// One poll cycle: talk to SMTC, return a fresh snapshot (with
    /// `track_changed` left at `false` — the caller fills that in).
    fn poll_once() -> anyhow::Result<NowPlaying> {
        let manager = SessionManager::RequestAsync()
            .map_err(|e| anyhow::anyhow!("RequestAsync failed: {e}"))?
            .join()
            .map_err(|e| anyhow::anyhow!("RequestAsync.join failed: {e}"))?;

        // No current session at all (nothing registered as a media source)
        // is a perfectly normal, expected state — not a warning-worthy error.
        let session = match manager.GetCurrentSession() {
            Ok(s) => s,
            Err(_) => return Ok(NowPlaying { has_music: false, ..NowPlaying::default() }),
        };

        let is_playing = session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .map(|status| status == PlaybackStatus::Playing)
            .unwrap_or(false);

        let (title, artist, palette) = match session
            .TryGetMediaPropertiesAsync()
            .and_then(|op| op.join())
        {
            Ok(props) => {
                let title = props.Title().map(|h| h.to_string_lossy()).unwrap_or_default();
                let artist = props.Artist().map(|h| h.to_string_lossy()).unwrap_or_default();
                let palette = extract_palette(&props).unwrap_or(DEFAULT_PALETTE);
                (title, artist, palette)
            }
            Err(e) => {
                log::warn!("nowplaying: TryGetMediaPropertiesAsync failed: {e}");
                (String::new(), String::new(), DEFAULT_PALETTE)
            }
        };

        Ok(NowPlaying {
            has_music: true,
            is_playing,
            track_changed: false,
            palette,
            title,
            artist,
        })
    }

    // -----------------------------------------------------------------------
    // Album art -> palette
    // -----------------------------------------------------------------------

    /// Decode the track's thumbnail (if any) to raw RGBA8 and reduce it to a
    /// 4-colour palette. Returns `None` on any failure (no thumbnail, decode
    /// error, empty image, ...) so the caller can fall back to the default.
    fn extract_palette(props: &MediaProperties) -> Option<[[f32; 3]; 4]> {
        let thumb_ref = props.Thumbnail().ok()?;
        let stream = thumb_ref.OpenReadAsync().ok()?.join().ok()?;
        let decoder = BitmapDecoder::CreateAsync(&stream).ok()?.join().ok()?;

        let width = decoder.PixelWidth().ok()? as usize;
        let height = decoder.PixelHeight().ok()? as usize;
        if width == 0 || height == 0 {
            return None;
        }

        // Force a known, simple byte layout (straight, non-premultiplied
        // RGBA8, EXIF orientation ignored, colour-managed to sRGB) so the
        // sampling code below never has to special-case the source format.
        let transform = BitmapTransform::new().ok()?;
        let provider = decoder
            .GetPixelDataTransformedAsync(
                BitmapPixelFormat::Rgba8,
                BitmapAlphaMode::Straight,
                &transform,
                ExifOrientationMode::IgnoreExifOrientation,
                ColorManagementMode::ColorManageToSRgb,
            )
            .ok()?
            .join()
            .ok()?;
        let pixels = provider.DetachPixelData().ok()?;

        Some(palette_from_rgba(pixels.as_slice(), width, height))
    }

    /// Coarse histogram-based palette extraction (no k-means):
    /// sparsely sample the image, quantize each sample to a coarse RGB
    /// bucket, count buckets, then rank by `count * saturation weight` with
    /// penalties for near-black/near-white/near-transparent samples. The top
    /// 4 buckets (by score) become the palette, most-prominent first.
    fn palette_from_rgba(pixels: &[u8], width: usize, height: usize) -> [[f32; 3]; 4] {
        if width == 0 || height == 0 || pixels.len() < width * height * 4 {
            return DEFAULT_PALETTE;
        }

        // 4 bits/channel -> 16^3 = 4096 possible buckets, coarse enough to
        // group similar colours but fine enough to keep hue separated.
        const QUANT_SHIFT: u32 = 4;
        const MAX_SAMPLES_PER_AXIS: usize = 40;

        let step_x = (width / MAX_SAMPLES_PER_AXIS).max(1);
        let step_y = (height / MAX_SAMPLES_PER_AXIS).max(1);

        // bucket key -> (sample count, summed raw r/g/b for centroid average)
        let mut buckets: HashMap<u16, (u32, [u64; 3])> = HashMap::new();

        let mut y = 0;
        while y < height {
            let row = y * width;
            let mut x = 0;
            while x < width {
                let idx = (row + x) * 4;
                let r = pixels[idx];
                let g = pixels[idx + 1];
                let b = pixels[idx + 2];
                let a = pixels[idx + 3];
                if a >= 16 {
                    let qr = (r >> QUANT_SHIFT) as u16;
                    let qg = (g >> QUANT_SHIFT) as u16;
                    let qb = (b >> QUANT_SHIFT) as u16;
                    let key = (qr << 8) | (qg << 4) | qb;
                    let entry = buckets.entry(key).or_insert((0, [0u64; 3]));
                    entry.0 += 1;
                    entry.1[0] += r as u64;
                    entry.1[1] += g as u64;
                    entry.1[2] += b as u64;
                }
                x += step_x;
            }
            y += step_y;
        }

        if buckets.is_empty() {
            return DEFAULT_PALETTE;
        }

        // Score + centroid colour for every bucket.
        let mut scored: Vec<(f32, [f32; 3])> = buckets
            .into_iter()
            .map(|(_, (count, sum))| {
                let n = count as f32;
                let r = sum[0] as f32 / count as f32 / 255.0;
                let g = sum[1] as f32 / count as f32 / 255.0;
                let b = sum[2] as f32 / count as f32 / 255.0;

                let max_c = r.max(g).max(b);
                let min_c = r.min(g).min(b);
                let lightness = (max_c + min_c) * 0.5;
                let sat = if (max_c - min_c).abs() < 1e-6 {
                    0.0
                } else {
                    (max_c - min_c) / (1.0 - (2.0 * lightness - 1.0).abs()).max(1e-4)
                };

                let dark_penalty = if lightness < 0.08 { 0.15 } else { 1.0 };
                let light_penalty = if lightness > 0.95 { 0.3 } else { 1.0 };
                // Never fully zero out low-saturation buckets — greys/whites
                // still matter for e.g. monochrome cover art.
                let sat_weight = 0.35 + 0.65 * sat;

                let score = n * sat_weight * dark_penalty * light_penalty;
                (score, [r, g, b])
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut palette = DEFAULT_PALETTE;
        for (slot, (_, rgb)) in palette.iter_mut().zip(scored.iter()) {
            *slot = *rgb;
        }
        palette
    }

    // -----------------------------------------------------------------------
    // Track identity
    // -----------------------------------------------------------------------

    /// Stable FNV-1a 64-bit hash of `title` + `artist`, used to detect track
    /// changes between polls without keeping the strings around twice.
    fn track_hash(title: &str, artist: &str) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

        let mut hash = FNV_OFFSET;
        for byte in title.bytes().chain(std::iter::once(0u8)).chain(artist.bytes()) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash
    }
}
