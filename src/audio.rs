//! Real-time audio capture and FFT analysis.
//!
//! On Windows we default to *loopback* capture of the default output device, so
//! the wallpaper reacts to whatever is currently playing. Everything here is
//! best-effort: if no device/stream can be opened the engine keeps running with
//! a silent (all-zero) analysis rather than crashing.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustfft::{num_complex::Complex, Fft, FftPlanner};

use crate::uniforms::SPECTRUM_BINS;

/// Samples fed into each FFT frame.
const FFT_SIZE: usize = 2048;

/// Length of the short-term (per-`analyze()`-call) onset history ring used to
/// derive an adaptive beat-detection threshold. Sized in "calls", not seconds,
/// since it only needs to track the recent noise floor / dynamic range.
const ONSET_RING_LEN: usize = 128;

/// Virtual sample rate (Hz) that the long-term onset envelope is resampled
/// onto before autocorrelation, so BPM estimation is robust to `analyze()`
/// being called at an uneven cadence.
const ENV_RATE: f32 = 50.0;

/// Capacity of the resampled onset envelope ring (~4s of history at
/// `ENV_RATE`), long enough to contain several beat periods even at slow
/// tempos.
const ENV_CAP: usize = 200;

/// Tempo range considered when searching the autocorrelation for a peak.
const BPM_MIN: f32 = 60.0;
const BPM_MAX: f32 = 180.0;

/// Minimum time between two accepted beat events (seconds); guards against
/// double-triggering on a single smeared transient (caps detectable rate at
/// ~500 BPM, well above anything musically meaningful).
const MIN_BEAT_INTERVAL: f32 = 0.12;

/// Where to pull audio from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AudioSource {
    /// Windows: loopback of the default output; elsewhere the default input.
    Auto,
    /// Default input device (microphone / line-in).
    Input,
    /// Loopback of the default output device (what you hear).
    Loopback,
}

/// One frame of analysis handed to the shader uniforms.
#[derive(Clone)]
pub struct AudioFrame {
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub volume: f32,
    pub spectrum: [f32; SPECTRUM_BINS],
    /// Onset/beat pulse: jumps to 1.0 when an onset event is detected and
    /// decays every `analyze()` call. Good for driving a "kick" pulse.
    pub beat: f32,
    /// Estimated tempo in beats per minute. `0.0` while unconfident.
    pub bpm: f32,
    /// Confidence (0..1) in the current `bpm` estimate.
    pub beat_confidence: f32,
    /// Raw per-frame onset strength (spectral flux), normalised to 0..1.
    pub onset: f32,
}

impl Default for AudioFrame {
    fn default() -> Self {
        Self {
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            volume: 0.0,
            spectrum: [0.0; SPECTRUM_BINS],
            beat: 0.0,
            bpm: 0.0,
            beat_confidence: 0.0,
            onset: 0.0,
        }
    }
}

/// A lock-guarded ring of the most recent mono samples.
struct Ring {
    buf: Vec<f32>,
    pos: usize,
}

impl Ring {
    fn new(n: usize) -> Self {
        Self { buf: vec![0.0; n], pos: 0 }
    }
    fn push(&mut self, s: f32) {
        let n = self.buf.len();
        self.buf[self.pos] = s;
        self.pos = (self.pos + 1) % n;
    }
    /// Copy oldest→newest into `out`.
    fn snapshot(&self, out: &mut [f32]) {
        let n = self.buf.len();
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.buf[(self.pos + i) % n];
        }
    }
}

pub struct AudioEngine {
    // Kept alive for the lifetime of the engine; dropping it stops capture.
    _stream: Option<cpal::Stream>,
    ring: Arc<Mutex<Ring>>,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    scratch: Vec<f32>,
    complex: Vec<Complex<f32>>,
    smoothed: [f32; SPECTRUM_BINS],
    sample_rate: f32,
    gain: f32,
    active: bool,

    // --- Onset / beat / BPM state -----------------------------------------
    /// Per-bin magnitude from the previous `analyze()` call (not time-smoothed)
    /// used to compute spectral flux.
    prev_bins: [f32; SPECTRUM_BINS],
    /// Slowly-decaying peak tracker used to normalise flux into 0..1.
    flux_norm_max: f32,
    /// Short-term per-call onset history for adaptive thresholding.
    onset_ring: [f32; ONSET_RING_LEN],
    onset_ring_pos: usize,
    /// Whether the previous call's onset was below threshold (for edge
    /// detection so a sustained loud passage doesn't retrigger every call).
    onset_was_below: bool,
    /// Current beat pulse value (decays every call, jumps to 1.0 on an event).
    beat_pulse: f32,
    /// Seconds elapsed since the last accepted beat event (refractory timer).
    time_since_beat: f32,
    /// Wall-clock time of the previous `analyze()` call, for computing dt.
    last_analyze_instant: Option<Instant>,
    /// Onset envelope resampled onto a fixed `ENV_RATE` grid, for
    /// autocorrelation-based BPM estimation.
    env_ring: [f32; ENV_CAP],
    env_ring_pos: usize,
    /// Accumulated time (seconds) not yet consumed by an `ENV_RATE` slot.
    env_time_acc: f32,
    /// Accumulated time (seconds) since the autocorrelation was last recomputed.
    env_update_acc: f32,
    /// Reusable scratch buffer for the (detrended) linear-order envelope.
    bpm_scratch: Vec<f32>,
    bpm_smoothed: f32,
    bpm_conf_smoothed: f32,
}

impl AudioEngine {
    /// Try to start capture. Never fails hard — falls back to silent analysis.
    pub fn new(source: AudioSource, gain: f32) -> Self {
        let ring = Arc::new(Mutex::new(Ring::new(FFT_SIZE)));
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        // Hann window.
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let x = i as f32 / (FFT_SIZE as f32 - 1.0);
                0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
            })
            .collect();

        let mut engine = Self {
            _stream: None,
            ring: ring.clone(),
            fft,
            window,
            scratch: vec![0.0; FFT_SIZE],
            complex: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            smoothed: [0.0; SPECTRUM_BINS],
            sample_rate: 44_100.0,
            gain,
            active: false,

            prev_bins: [0.0; SPECTRUM_BINS],
            flux_norm_max: 0.05,
            onset_ring: [0.0; ONSET_RING_LEN],
            onset_ring_pos: 0,
            onset_was_below: true,
            beat_pulse: 0.0,
            time_since_beat: MIN_BEAT_INTERVAL + 1.0,
            last_analyze_instant: None,
            env_ring: [0.0; ENV_CAP],
            env_ring_pos: 0,
            env_time_acc: 0.0,
            env_update_acc: 0.0,
            bpm_scratch: vec![0.0; ENV_CAP],
            bpm_smoothed: 0.0,
            bpm_conf_smoothed: 0.0,
        };

        match open_stream(source, ring) {
            Ok((stream, rate)) => {
                if let Err(e) = stream.play() {
                    log::warn!("audio: failed to start stream: {e}; running silent");
                } else {
                    engine._stream = Some(stream);
                    engine.sample_rate = rate;
                    engine.active = true;
                    log::info!("audio: capturing at {rate} Hz");
                }
            }
            Err(e) => log::warn!("audio: no capture ({e}); wallpaper will run without audio reactivity"),
        }
        engine
    }

    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.active
    }
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    /// Run one FFT over the latest samples and return smoothed analysis.
    pub fn analyze(&mut self) -> AudioFrame {
        if !self.active {
            return AudioFrame::default();
        }

        // Wall-clock delta since the previous call; used for the beat
        // refractory timer and to resample the onset envelope onto a fixed
        // time grid regardless of how often `analyze()` happens to be called.
        let now = Instant::now();
        let dt = match self.last_analyze_instant {
            Some(prev) => (now - prev).as_secs_f32().clamp(0.0, 0.25),
            None => 1.0 / 60.0,
        };
        self.last_analyze_instant = Some(now);
        self.time_since_beat += dt;

        if let Ok(ring) = self.ring.lock() {
            ring.snapshot(&mut self.scratch);
        }

        // RMS volume from the raw window.
        let mut sq = 0.0f32;
        for &s in &self.scratch {
            sq += s * s;
        }
        let volume = (sq / FFT_SIZE as f32).sqrt();

        // Windowed FFT.
        for i in 0..FFT_SIZE {
            self.complex[i] = Complex::new(self.scratch[i] * self.window[i], 0.0);
        }
        self.fft.process(&mut self.complex);

        // Magnitude → 64 log-spaced bins.
        let half = FFT_SIZE / 2;
        let mut frame = AudioFrame::default();
        let mut flux_sum = 0.0f32;
        for b in 0..SPECTRUM_BINS {
            let f0 = (b as f32 / SPECTRUM_BINS as f32).powf(2.2);
            let f1 = ((b + 1) as f32 / SPECTRUM_BINS as f32).powf(2.2);
            let i0 = (f0 * half as f32) as usize;
            let i1 = ((f1 * half as f32) as usize).max(i0 + 1).min(half);
            let mut m = 0.0f32;
            for c in &self.complex[i0..i1] {
                m = m.max(c.norm());
            }
            // Normalise, log-compress and smooth with a fast attack / slow decay.
            let v = (m / FFT_SIZE as f32) * self.gain;
            let v = (v.max(0.0)).powf(0.7).min(1.0);

            // Spectral flux: positive-only increase over the *previous raw*
            // (not time-smoothed) bin value. This is what drives onset
            // detection below.
            let diff = v - self.prev_bins[b];
            if diff > 0.0 {
                flux_sum += diff;
            }
            self.prev_bins[b] = v;

            let prev = self.smoothed[b];
            let s = if v > prev { v } else { prev * 0.85 + v * 0.15 };
            self.smoothed[b] = s;
            frame.spectrum[b] = s;
        }

        // Band aggregates (bass / mid / treble) from the smoothed spectrum.
        frame.bass = avg(&frame.spectrum[0..8]);
        frame.mid = avg(&frame.spectrum[8..28]);
        frame.treble = avg(&frame.spectrum[28..SPECTRUM_BINS]);
        frame.volume = (volume * self.gain * 2.0).min(1.0);

        // --- Onset detection -------------------------------------------------
        // Adaptive peak-hold normaliser so `onset` reads relative to the
        // recent dynamic range instead of an arbitrary absolute scale.
        self.flux_norm_max = (self.flux_norm_max * 0.995).max(flux_sum).max(0.05);
        let onset_now = (flux_sum / self.flux_norm_max).clamp(0.0, 1.0);
        frame.onset = onset_now;

        // Short-term ring feeds an adaptive threshold (mean + k*std) with an
        // absolute floor so residual noise during silence never crosses it.
        self.onset_ring[self.onset_ring_pos] = onset_now;
        self.onset_ring_pos = (self.onset_ring_pos + 1) % ONSET_RING_LEN;
        let mut mean = 0.0f32;
        for &x in &self.onset_ring {
            mean += x;
        }
        mean /= ONSET_RING_LEN as f32;
        let mut var = 0.0f32;
        for &x in &self.onset_ring {
            let d = x - mean;
            var += d * d;
        }
        var /= ONSET_RING_LEN as f32;
        let threshold = (mean + 1.5 * var.sqrt()).max(0.08);

        let has_signal = frame.volume > 0.01;
        let rising_edge = onset_now > threshold && self.onset_was_below;
        let can_fire = self.time_since_beat > MIN_BEAT_INTERVAL;
        self.onset_was_below = onset_now <= threshold;

        if has_signal && rising_edge && can_fire {
            self.beat_pulse = 1.0;
            self.time_since_beat = 0.0;
        } else {
            self.beat_pulse *= 0.85;
        }
        frame.beat = self.beat_pulse;

        // --- BPM estimation ---------------------------------------------------
        // Resample the onset envelope onto a fixed-rate grid (zero-order
        // hold) so autocorrelation is meaningful even though `analyze()` may
        // be called at an uneven cadence.
        self.env_time_acc += dt;
        let slot_dt = 1.0 / ENV_RATE;
        let mut pushes = 0;
        while self.env_time_acc >= slot_dt && pushes < 8 {
            self.env_ring[self.env_ring_pos] = onset_now;
            self.env_ring_pos = (self.env_ring_pos + 1) % ENV_CAP;
            self.env_time_acc -= slot_dt;
            pushes += 1;
        }

        // Recompute the (relatively expensive) autocorrelation only a few
        // times a second rather than every call.
        self.env_update_acc += dt;
        if self.env_update_acc >= 0.15 {
            self.env_update_acc = 0.0;
            self.recompute_bpm();
        }

        frame.bpm = self.bpm_smoothed;
        frame.beat_confidence = self.bpm_conf_smoothed.clamp(0.0, 1.0);

        frame
    }

    /// Autocorrelate the resampled onset envelope over the 60-180 BPM lag
    /// range and update the smoothed tempo/confidence estimate. Graceful on
    /// silence: confidence (and eventually bpm) decays to zero rather than
    /// reporting a stale or spurious tempo.
    fn recompute_bpm(&mut self) {
        let n = ENV_CAP;
        for i in 0..n {
            self.bpm_scratch[i] = self.env_ring[(self.env_ring_pos + i) % n];
        }
        let mean = self.bpm_scratch.iter().sum::<f32>() / n as f32;
        for x in self.bpm_scratch.iter_mut() {
            *x -= mean;
        }

        let energy: f32 = self.bpm_scratch.iter().map(|x| x * x).sum();
        if energy < 1e-4 {
            // No usable signal in the window: fade out gracefully.
            self.bpm_conf_smoothed *= 0.9;
            if self.bpm_conf_smoothed < 0.02 {
                self.bpm_conf_smoothed = 0.0;
                self.bpm_smoothed = 0.0;
            }
            return;
        }

        let lag_min = ((ENV_RATE * 60.0) / BPM_MAX).floor().max(1.0) as usize;
        let lag_max = (((ENV_RATE * 60.0) / BPM_MIN).ceil() as usize).min(n - 1);

        let mut best_lag = lag_min;
        let mut best_corr = f32::MIN;
        for lag in lag_min..=lag_max {
            let mut c = 0.0f32;
            for i in 0..(n - lag) {
                c += self.bpm_scratch[i] * self.bpm_scratch[i + lag];
            }
            if c > best_corr {
                best_corr = c;
                best_lag = lag;
            }
        }

        // Fraction of the window's energy explained by periodicity at the
        // winning lag — a simple, bounded proxy for confidence.
        let raw_conf = (best_corr / energy).clamp(0.0, 1.0);
        let candidate_bpm = 60.0 * ENV_RATE / best_lag as f32;

        self.bpm_conf_smoothed = self.bpm_conf_smoothed * 0.7 + raw_conf * 0.3;
        if self.bpm_conf_smoothed > 0.12 {
            if self.bpm_smoothed < 1.0 {
                self.bpm_smoothed = candidate_bpm;
            } else {
                self.bpm_smoothed = self.bpm_smoothed * 0.7 + candidate_bpm * 0.3;
            }
        } else if self.bpm_conf_smoothed < 0.05 {
            self.bpm_smoothed *= 0.9;
            if self.bpm_smoothed < 1.0 {
                self.bpm_smoothed = 0.0;
            }
        }
    }
}

fn avg(xs: &[f32]) -> f32 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f32>() / xs.len() as f32
    }
}

/// Open a capture stream according to `source`, with graceful fallbacks.
fn open_stream(source: AudioSource, ring: Arc<Mutex<Ring>>) -> anyhow::Result<(cpal::Stream, f32)> {
    let host = cpal::default_host();

    let want_loopback = match source {
        AudioSource::Loopback => true,
        AudioSource::Input => false,
        AudioSource::Auto => cfg!(windows),
    };

    if want_loopback {
        if let Some(device) = host.default_output_device() {
            match build_input_from_config(&device, device.default_output_config().ok(), ring.clone()) {
                Ok(v) => return Ok(v),
                Err(e) => log::warn!("audio: loopback failed ({e}); trying input device"),
            }
        }
    }

    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("no input device available"))?;
    build_input_from_config(&device, device.default_input_config().ok(), ring)
}

fn build_input_from_config(
    device: &cpal::Device,
    supported: Option<cpal::SupportedStreamConfig>,
    ring: Arc<Mutex<Ring>>,
) -> anyhow::Result<(cpal::Stream, f32)> {
    let supported = supported.ok_or_else(|| anyhow::anyhow!("no supported stream config"))?;
    let sample_format = supported.sample_format();
    let sample_rate = supported.sample_rate() as f32;
    let channels = supported.channels() as usize;
    let config: cpal::StreamConfig = supported.config();

    // Downmix interleaved frames to mono and push into the ring.
    macro_rules! build {
        ($t:ty, $to_f32:expr) => {{
            let ring = ring.clone();
            device.build_input_stream(
                config.clone(),
                move |data: &[$t], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut r) = ring.lock() {
                        for frame in data.chunks(channels) {
                            let mut acc = 0.0f32;
                            for &s in frame {
                                acc += $to_f32(s);
                            }
                            r.push(acc / channels as f32);
                        }
                    }
                },
                |e| log::warn!("audio: stream error: {e}"),
                None,
            )?
        }};
    }

    let stream = match sample_format {
        cpal::SampleFormat::F32 => build!(f32, |s: f32| s),
        cpal::SampleFormat::I16 => build!(i16, |s: i16| s as f32 / i16::MAX as f32),
        cpal::SampleFormat::U16 => build!(u16, |s: u16| (s as f32 / u16::MAX as f32) * 2.0 - 1.0),
        other => anyhow::bail!("unsupported sample format: {other:?}"),
    };

    Ok((stream, sample_rate))
}
