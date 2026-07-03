//! real_live_wall — a reactive, cross-platform live wallpaper engine.
//!
//! Draws a full-screen shader (built-in WGSL scene or a Shadertoy GLSL file)
//! that reacts in real time to system audio, CPU/memory load and the clock,
//! either in a preview window (with an egui settings panel) or as the live
//! desktop wallpaper.

// Release builds are a GUI app: no console window pops up on double-click.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod config;
mod gpu;
mod persist;
mod platform;
mod postfx;
mod reactive;
mod renderer;
mod screenshot;
mod shader;
mod startup;
mod tray;
mod ui;
mod uniforms;

use clap::Parser;
use winit::event_loop::{ControlFlow, EventLoop};

use crate::app::AppEvent;
use crate::config::Mode;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = config::Config::parse();

    // `--screenshot`: render one frame offscreen, save a PNG and quit.
    if let Some(out) = config.screenshot.clone() {
        return screenshot::run(&config, &out);
    }

    // `--stop`: just signal any running wallpaper and quit — no window needed.
    if config.stop {
        if platform::signal_stop() {
            log::info!("stop signal sent to running wallpaper");
        } else {
            log::info!("no running wallpaper found");
        }
        return Ok(());
    }

    // Only one wallpaper process at a time (guards autostart + double-launch).
    if config.mode == Mode::Wallpaper && platform::wallpaper_running() {
        log::info!("a wallpaper is already running; exiting");
        return Ok(());
    }

    log::info!("real_live_wall starting ({:?} mode)", config.mode);
    let mode = config.mode;

    // A user-event loop so a background thread can wake us to exit cleanly when
    // another instance requests a wallpaper stop.
    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    // Continuous animation: keep pumping frames.
    event_loop.set_control_flow(ControlFlow::Poll);
    let proxy = event_loop.create_proxy();

    let mut app = app::App::new(config, proxy);
    event_loop.run_app(&mut app)?;

    // After the wallpaper's windows are gone, nudge the shell to repaint the
    // static desktop wallpaper so no black region is left behind.
    if mode == Mode::Wallpaper {
        platform::restore_desktop();
    }
    Ok(())
}
