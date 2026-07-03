//! Real-time audio capture and FFT analysis.
//!
//! On Windows we default to *loopback* capture of the default output device, so
//! the wallpaper reacts to whatever is currently playing. Everything here is
//! best-effort: if no device/stream can be opened the engine keeps running with
//! a silent (all-zero) analysis rather than crashing.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustfft::{num_complex::Complex, Fft, FftPlanner};

use crate::uniforms::SPECTRUM_BINS;

/// Samples fed into each FFT frame.
const FFT_SIZE: usize = 2048;

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
}

impl Default for AudioFrame {
    fn default() -> Self {
        Self { bass: 0.0, mid: 0.0, treble: 0.0, volume: 0.0, spectrum: [0.0; SPECTRUM_BINS] }
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
        frame
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
