//! The winit application: window lifecycle, the per-frame update/render loop,
//! input, audio/system sampling and optional GLSL hot-reload.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::audio::AudioEngine;
use crate::config::{Config, Mode};
use crate::gpu::Gpu;
use crate::reactive::Reactive;
use crate::renderer::{RenderOutcome, Renderer};
use crate::uniforms::Uniforms;

pub struct App {
    config: Config,
    state: Option<State>,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self { config, state: None }
    }
}

struct State {
    window: Arc<Window>,
    gpu: Gpu,
    renderer: Renderer,
    audio: Option<AudioEngine>,
    reactive: Reactive,
    uniforms: Uniforms,

    start: Instant,
    last_frame: Instant,
    frame: u32,
    fps: f32,
    mouse: [f32; 4],

    shader_path: Option<PathBuf>,
    _watcher: Option<RecommendedWatcher>,
    watch_rx: Option<Receiver<()>>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match build_state(&self.config, event_loop) {
            Ok(state) => self.state = Some(state),
            Err(e) => {
                log::error!("failed to initialise engine: {e:#}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.state.as_mut() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    && self.config.mode == Mode::Preview
                {
                    event_loop.exit();
                }
            }

            WindowEvent::Resized(size) => {
                state.gpu.resize(size.width, size.height);
                state.renderer.rebind(&state.gpu);
            }

            WindowEvent::CursorMoved { position, .. } => {
                let h = state.gpu.size.1 as f32;
                state.mouse[0] = position.x as f32;
                state.mouse[1] = h - position.y as f32;
            }

            WindowEvent::MouseInput { state: btn_state, button, .. } => {
                if button == MouseButton::Left && btn_state == ElementState::Pressed {
                    state.mouse[2] = state.mouse[0];
                    state.mouse[3] = state.mouse[1];
                }
            }

            WindowEvent::RedrawRequested => {
                state.render_frame();
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = self.state.as_ref() {
            state.window.request_redraw();
        }
    }
}

fn build_state(config: &Config, event_loop: &ActiveEventLoop) -> anyhow::Result<State> {
    // --- window -------------------------------------------------------------
    let mut attrs = Window::default_attributes().with_title("real_live_wall");
    match config.mode {
        Mode::Preview => {
            attrs = attrs
                .with_inner_size(PhysicalSize::new(config.width, config.height))
                .with_resizable(true);
            if config.top {
                attrs = attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
            }
        }
        Mode::Wallpaper => {
            let size = event_loop
                .primary_monitor()
                .map(|m| m.size())
                .unwrap_or(PhysicalSize::new(1920, 1080));
            attrs = attrs
                .with_inner_size(size)
                .with_position(PhysicalPosition::new(0, 0))
                .with_decorations(false)
                .with_resizable(false);
        }
    }
    let window = Arc::new(event_loop.create_window(attrs)?);

    // --- gpu + renderer -----------------------------------------------------
    let gpu = Gpu::new(window.clone())?;
    let mut renderer = Renderer::new(&gpu)?;

    // --- optional Shadertoy GLSL scene --------------------------------------
    let mut shader_path = None;
    if let Some(path) = &config.shader {
        match std::fs::read_to_string(path) {
            Ok(src) => match renderer.load_shadertoy_glsl(&gpu, &src) {
                Ok(()) => {
                    log::info!("loaded shader {}", path.display());
                    shader_path = Some(path.clone());
                }
                Err(e) => log::error!("shader {} failed: {e:#}; using default scene", path.display()),
            },
            Err(e) => log::error!("could not read shader {}: {e}", path.display()),
        }
    }

    // --- wallpaper attach ---------------------------------------------------
    if config.mode == Mode::Wallpaper {
        if let Err(e) = crate::platform::attach_to_desktop(&window) {
            log::error!("wallpaper attach failed: {e:#}");
        }
    }

    // --- audio --------------------------------------------------------------
    let audio = config
        .audio_source()
        .map(|src| AudioEngine::new(src, config.gain));

    // --- optional hot-reload watcher ----------------------------------------
    let (mut watcher, watch_rx) = (None, None);
    if config.watch {
        if let Some(path) = &shader_path {
            match make_watcher(path) {
                Ok((w, rx)) => {
                    watcher = Some(w);
                    log::info!("watching {} for changes", path.display());
                    return Ok(finish_state(window, gpu, renderer, audio, shader_path, watcher, Some(rx)));
                }
                Err(e) => log::warn!("watch disabled: {e}"),
            }
        }
    }
    let _ = &mut watcher;

    Ok(finish_state(window, gpu, renderer, audio, shader_path, watcher, watch_rx))
}

#[allow(clippy::too_many_arguments)]
fn finish_state(
    window: Arc<Window>,
    gpu: Gpu,
    renderer: Renderer,
    audio: Option<AudioEngine>,
    shader_path: Option<PathBuf>,
    watcher: Option<RecommendedWatcher>,
    watch_rx: Option<Receiver<()>>,
) -> State {
    let now = Instant::now();
    State {
        window,
        gpu,
        renderer,
        audio,
        reactive: Reactive::new(),
        uniforms: Uniforms::default(),
        start: now,
        last_frame: now,
        frame: 0,
        fps: 60.0,
        mouse: [0.0; 4],
        shader_path,
        _watcher: watcher,
        watch_rx,
    }
}

fn make_watcher(path: &PathBuf) -> anyhow::Result<(RecommendedWatcher, Receiver<()>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(path, RecursiveMode::NonRecursive)?;
    Ok((watcher, rx))
}

impl State {
    fn render_frame(&mut self) {
        // --- timing ---------------------------------------------------------
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().max(1e-5);
        self.last_frame = now;
        let t = (now - self.start).as_secs_f32();
        self.fps = self.fps * 0.9 + (1.0 / dt) * 0.1;
        self.frame = self.frame.wrapping_add(1);

        // --- hot reload -----------------------------------------------------
        if let (Some(rx), Some(path)) = (self.watch_rx.as_ref(), self.shader_path.as_ref()) {
            if rx.try_iter().count() > 0 {
                if let Ok(src) = std::fs::read_to_string(path) {
                    match self.renderer.load_shadertoy_glsl(&self.gpu, &src) {
                        Ok(()) => log::info!("reloaded {}", path.display()),
                        Err(e) => log::error!("reload failed: {e:#}"),
                    }
                }
            }
        }

        // --- reactive inputs ------------------------------------------------
        let audio_frame = self
            .audio
            .as_mut()
            .map(|a| a.analyze())
            .unwrap_or_default();
        let (cpu, mem) = self.reactive.poll();
        let sample_rate = self.audio.as_ref().map(|a| a.sample_rate()).unwrap_or(44_100.0);

        // --- fill uniforms --------------------------------------------------
        let (w, h) = self.gpu.size;
        let u = &mut self.uniforms;
        u.resolution = [w as f32, h as f32, 1.0, w as f32 / h.max(1) as f32];
        u.mouse = self.mouse;
        u.time = [t, dt, self.frame as f32, sample_rate];
        u.audio = [audio_frame.bass, audio_frame.mid, audio_frame.treble, audio_frame.volume];
        // beat = quick bass transient proxy
        u.sys = [cpu, mem, audio_frame.bass, self.fps];
        u.set_spectrum(&audio_frame.spectrum);

        self.renderer.update_uniforms(&self.gpu, u);

        // --- draw -----------------------------------------------------------
        match self.renderer.render(&self.gpu) {
            RenderOutcome::Presented | RenderOutcome::Skipped => {}
            RenderOutcome::NeedsReconfigure => self.gpu.reconfigure(),
        }
    }
}
