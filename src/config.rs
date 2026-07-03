//! Command-line configuration.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::audio::AudioSource;

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Show the engine in a normal resizable window (best for development).
    Preview,
    /// Render as the live desktop wallpaper (behind icons).
    Wallpaper,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum AudioArg {
    Auto,
    Input,
    Loopback,
    Off,
}

impl AudioArg {
    pub fn to_source(self) -> Option<AudioSource> {
        match self {
            AudioArg::Off => None,
            AudioArg::Auto => Some(AudioSource::Auto),
            AudioArg::Input => Some(AudioSource::Input),
            AudioArg::Loopback => Some(AudioSource::Loopback),
        }
    }

    /// The clap value name, for spawning a wallpaper child process.
    pub fn as_cli(self) -> &'static str {
        match self {
            AudioArg::Auto => "auto",
            AudioArg::Input => "input",
            AudioArg::Loopback => "loopback",
            AudioArg::Off => "off",
        }
    }
}

/// Reactive, cross-platform live wallpaper engine.
#[derive(Parser, Debug)]
#[command(name = "real_live_wall", version, about)]
pub struct Config {
    /// Run mode: `preview` window or live `wallpaper`.
    #[arg(long, value_enum, default_value_t = Mode::Preview)]
    pub mode: Mode,

    /// Path to a Shadertoy-style GLSL shader (defines `mainImage`). Omit for the
    /// built-in audio-reactive aurora scene.
    #[arg(long, short = 's')]
    pub shader: Option<PathBuf>,

    /// Audio capture source for reactivity.
    #[arg(long, value_enum, default_value_t = AudioArg::Auto)]
    pub audio: AudioArg,

    /// Audio sensitivity multiplier applied to the FFT magnitudes.
    #[arg(long, default_value_t = 6.0)]
    pub gain: f32,

    /// Hot-reload the GLSL shader file when it changes on disk.
    #[arg(long, default_value_t = false)]
    pub watch: bool,

    /// Keep the preview window always on top (handy for screen capture).
    #[arg(long, default_value_t = false)]
    pub top: bool,

    /// Preview window width (ignored in wallpaper mode).
    #[arg(long, default_value_t = 1280)]
    pub width: u32,

    /// Preview window height (ignored in wallpaper mode).
    #[arg(long, default_value_t = 720)]
    pub height: u32,
}

impl Config {
    pub fn audio_source(&self) -> Option<AudioSource> {
        self.audio.to_source()
    }
}
