//! GPU-facing uniform block, shared byte-for-byte between the Rust side and both
//! the WGSL and GLSL shaders. Laid out to be std140-compatible: every field is a
//! 16-byte aligned `vec4`, and the spectrum is an array of `vec4` (64 bins).

use bytemuck::{Pod, Zeroable};

/// Number of FFT spectrum bins exposed to shaders.
pub const SPECTRUM_BINS: usize = 64;
/// Spectrum packed as `vec4`s (4 bins each).
pub const SPECTRUM_VEC4S: usize = SPECTRUM_BINS / 4;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Uniforms {
    /// x,y = pixel resolution, z = 1.0, w = aspect ratio (w/h).
    pub resolution: [f32; 4],
    /// x,y = current mouse (pixels, origin bottom-left), z,w = last click.
    pub mouse: [f32; 4],
    /// x = iTime (s), y = iTimeDelta (s), z = iFrame, w = sample rate.
    pub time: [f32; 4],
    /// x = bass, y = mid, z = treble, w = overall volume. All ~0..1.
    pub audio: [f32; 4],
    /// x = cpu load (0..1), y = memory used (0..1), z = beat pulse, w = fps.
    pub sys: [f32; 4],
    /// x = year, y = month, z = day, w = seconds-in-day (Shadertoy iDate).
    pub date: [f32; 4],
    /// 64 FFT magnitude bins (0..1), packed 4 per vec4.
    pub spectrum: [[f32; 4]; SPECTRUM_VEC4S],
}

impl Default for Uniforms {
    fn default() -> Self {
        Self {
            resolution: [1.0, 1.0, 1.0, 1.0],
            mouse: [0.0; 4],
            time: [0.0, 0.0, 0.0, 44100.0],
            audio: [0.0; 4],
            sys: [0.0; 4],
            date: [0.0; 4],
            spectrum: [[0.0; 4]; SPECTRUM_VEC4S],
        }
    }
}

impl Uniforms {
    /// Copy a 64-bin spectrum into the packed vec4 layout.
    pub fn set_spectrum(&mut self, bins: &[f32; SPECTRUM_BINS]) {
        for (i, chunk) in bins.chunks_exact(4).enumerate() {
            self.spectrum[i] = [chunk[0], chunk[1], chunk[2], chunk[3]];
        }
    }
}
