//! The winit application: window lifecycle, the per-frame update/render loop,
//! input, audio/system sampling, the egui settings panel, GLSL hot-reload and
//! spawning a detached wallpaper process.
//!
//! In wallpaper mode one borderless window is created per monitor (each with its
//! own surface + post-processing chain, all sharing a single GPU device) so the
//! same scene is drawn full-frame on every screen. The whole fleet is driven from
//! a single frame loop keyed off the first window's redraw.

use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::audio::AudioEngine;
use crate::config::{AudioArg, Config, Mode};
use crate::gpu::{Gpu, GpuContext};
use crate::persist::PersistConfig;
use crate::platform::{self, MonitorRect};
use crate::postfx::PostFx;
use crate::reactive::Reactive;
use crate::renderer::Renderer;
use crate::startup;
use crate::tray::{self, TrayCommand};
use crate::ui::{Meters, Settings, Ui, UiAction};
use crate::uniforms::Uniforms;

/// Events delivered to the loop from outside the winit thread.
#[derive(Debug, Clone, Copy)]
pub enum AppEvent {
    /// Another instance asked this wallpaper process to exit.
    StopWallpaper,
    /// A tray-menu action (wallpaper process only).
    Tray(TrayCommand),
}

/// A selectable scene: the built-in WGSL aurora, or a Shadertoy GLSL file.
enum SceneSource {
    Builtin,
    Glsl(PathBuf),
}

struct Scene {
    name: String,
    source: SceneSource,
}

/// Locate the `shaders/` directory: next to the current dir (dev / `cargo run`),
/// else next to the executable (release zip, autostart from another cwd).
fn shaders_dir() -> Option<PathBuf> {
    let cwd = PathBuf::from("shaders");
    if cwd.is_dir() {
        return Some(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("shaders");
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

fn discover_scenes() -> Vec<Scene> {
    let mut scenes = vec![Scene {
        name: "기본 — 오로라 + 스펙트럼".to_string(),
        source: SceneSource::Builtin,
    }];
    if let Some(dir) = shaders_dir() {
        if let Ok(rd) = std::fs::read_dir(dir) {
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|x| x == "glsl").unwrap_or(false))
            .collect();
        files.sort();
        for f in files {
            let name = f
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "shader".to_string());
            scenes.push(Scene { name, source: SceneSource::Glsl(f) });
        }
        }
    }
    scenes
}

pub struct App {
    config: Config,
    proxy: EventLoopProxy<AppEvent>,
    state: Option<State>,
}

impl App {
    pub fn new(config: Config, proxy: EventLoopProxy<AppEvent>) -> Self {
        Self { config, proxy, state: None }
    }
}

/// One monitor's render surface: its own window, swapchain and post-processing
/// chain. The scene [`Renderer`] (pipeline + uniform buffer) is shared across all
/// targets; uniforms are re-written per target just before its submit.
struct MonitorTarget {
    window: Arc<Window>,
    gpu: Gpu,
    postfx: PostFx,
}

impl MonitorTarget {
    fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
        self.postfx.resize(&self.gpu, width, height);
    }

    /// Draw the scene (no GUI) for this monitor. `uniforms` must already be
    /// filled for this monitor's scene resolution.
    fn render(&mut self, renderer: &Renderer, uniforms: &Uniforms) {
        renderer.update_uniforms(&self.gpu, uniforms);
        let Some(frame) = self.gpu.acquire() else {
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("rlw-encoder") });
        renderer.draw_scene(&self.gpu, self.postfx.scene_view(), &mut encoder);
        self.postfx.render(&mut encoder, &view);
        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
    }
}

struct State {
    // Kept alive for the lifetime of every surface it created.
    _ctx: GpuContext,
    monitors: Vec<MonitorTarget>,
    renderer: Renderer,
    audio: Option<AudioEngine>,
    reactive: Reactive,
    uniforms: Uniforms,

    mode: Mode,
    ssaa: f32,
    ui: Option<Ui>,
    settings: Settings,
    scenes: Vec<Scene>,
    scene_names: Vec<String>,
    wallpaper_child: Option<Child>,
    // Keeps the wallpaper stop-watcher thread's ownership marker alive.
    _stop_guard: Option<platform::StopGuard>,
    // Keeps the system-tray thread alive (wallpaper process only).
    _tray: Option<tray::TrayHandle>,

    start: Instant,
    last_frame: Instant,
    frame: u32,
    fps: f32,
    mouse: [f32; 4],

    shader_path: Option<PathBuf>,
    _watcher: Option<RecommendedWatcher>,
    watch_rx: Option<Receiver<()>>,
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match build_state(&self.config, event_loop, &self.proxy) {
            Ok(state) => self.state = Some(state),
            Err(e) => {
                log::error!("failed to initialise engine: {e:#}");
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::StopWallpaper => {
                log::info!("exiting on stop request");
                event_loop.exit();
            }
            AppEvent::Tray(cmd) => {
                if let Some(state) = self.state.as_mut() {
                    state.on_tray(cmd, event_loop);
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };

        let is_primary = state
            .monitors
            .first()
            .map(|m| m.window.id() == id)
            .unwrap_or(false);

        // Let egui (preview-only, on the primary window) see the event first.
        let egui_consumed = if is_primary {
            if let Some(ui) = state.ui.as_mut() {
                ui.on_window_event(state.monitors[0].window.as_ref(), &event)
            } else {
                false
            }
        } else {
            false
        };

        match event {
            WindowEvent::CloseRequested => {
                state.save_prefs();
                event_loop.exit();
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::F1) => {
                            state.settings.panel_open = !state.settings.panel_open;
                        }
                        PhysicalKey::Code(KeyCode::Escape) if self.config.mode == Mode::Preview => {
                            state.save_prefs();
                            event_loop.exit();
                        }
                        _ => {}
                    }
                }
            }

            WindowEvent::Resized(size) => {
                if let Some(i) = state.index_of(id) {
                    state.monitors[i].resize(size.width, size.height);
                    state.renderer.rebind(&state.monitors[i].gpu);
                }
            }

            WindowEvent::CursorMoved { position, .. } if is_primary && !egui_consumed => {
                let h = state.monitors[0].gpu.size.1 as f32;
                state.mouse[0] = position.x as f32;
                state.mouse[1] = h - position.y as f32;
            }

            WindowEvent::MouseInput { state: btn, button, .. }
                if is_primary && !egui_consumed && button == MouseButton::Left && btn == ElementState::Pressed =>
            {
                state.mouse[2] = state.mouse[0];
                state.mouse[3] = state.mouse[1];
            }

            // The frame loop is driven off the primary window's redraw; it renders
            // every monitor. Ignore redraws for the others.
            WindowEvent::RedrawRequested if is_primary => {
                let actions = state.render_frame();
                for action in actions {
                    state.apply_action(action, event_loop);
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = self.state.as_ref() {
            if let Some(primary) = state.monitors.first() {
                primary.window.request_redraw();
            }
        }
    }
}

/// Describes one window to create, plus (in wallpaper mode) the monitor rect it
/// must be pinned to inside the desktop WorkerW layer.
struct WinSpec {
    window: Arc<Window>,
    rect: Option<MonitorRect>,
}

fn build_state(
    config: &Config,
    event_loop: &ActiveEventLoop,
    proxy: &EventLoopProxy<AppEvent>,
) -> anyhow::Result<State> {
    // --- windows ------------------------------------------------------------
    let specs = create_windows(config, event_loop)?;

    // --- shared gpu + scene renderer (from the first window) ----------------
    let (ctx, gpu0) = GpuContext::new(specs[0].window.clone())?;
    let mut renderer = Renderer::new(&gpu0)?;

    // --- merge persisted preferences (preview only) with CLI overrides ------
    let persisted = if config.mode == Mode::Preview {
        PersistConfig::load()
    } else {
        PersistConfig::default()
    };
    // Explicit CLI values (i.e. not the clap default) win; else use persisted.
    let gain = if config.gain != 6.0 { config.gain } else { persisted.gain };
    let audio_arg = if config.audio != AudioArg::Auto {
        config.audio
    } else {
        AudioArg::from_cli(&persisted.audio)
    };
    let ssaa = (if config.ssaa != 1.5 { config.ssaa } else { persisted.ssaa }).clamp(1.0, 2.0);

    // --- scenes + initial selection -----------------------------------------
    let preferred = if config.shader.is_none() {
        persisted.scene.as_deref()
    } else {
        None
    };
    let (scenes, scene_index) = select_scenes(config, preferred);
    let shader_path = load_scene(&gpu0, &mut renderer, &scenes[scene_index]);
    let scene_names = scenes.iter().map(|s| s.name.clone()).collect();

    // --- one render target per window ---------------------------------------
    let mut monitors = Vec::with_capacity(specs.len());
    let postfx0 = PostFx::new(&gpu0, ssaa);
    monitors.push(MonitorTarget { window: specs[0].window.clone(), gpu: gpu0, postfx: postfx0 });
    for spec in &specs[1..] {
        let gpu = ctx.create_gpu(spec.window.clone())?;
        let postfx = PostFx::new(&gpu, ssaa);
        monitors.push(MonitorTarget { window: spec.window.clone(), gpu, postfx });
    }

    // --- wallpaper attach + remote-stop watcher + system tray ---------------
    let mut stop_guard = None;
    let mut tray_handle = None;
    if config.mode == Mode::Wallpaper {
        for (spec, target) in specs.iter().zip(monitors.iter()) {
            if let Some(rect) = spec.rect {
                if let Err(e) = platform::attach_to_desktop(target.window.as_ref(), rect) {
                    log::error!("wallpaper attach failed: {e:#}");
                }
            }
        }
        let proxy_stop = proxy.clone();
        stop_guard = platform::watch_for_stop(move || {
            let _ = proxy_stop.send_event(AppEvent::StopWallpaper);
        });
        // Tray icon lives in the (persistent) wallpaper process: control it
        // without keeping a settings window open.
        let proxy_tray = proxy.clone();
        match tray::spawn(
            startup::autostart_enabled(),
            Box::new(move |cmd| {
                let _ = proxy_tray.send_event(AppEvent::Tray(cmd));
            }),
        ) {
            Ok(h) => tray_handle = Some(h),
            Err(e) => log::warn!("tray disabled: {e:#}"),
        }
    }

    // --- audio --------------------------------------------------------------
    let audio = audio_arg
        .to_source()
        .map(|src| AudioEngine::new(src, gain));

    // --- optional hot-reload watcher ----------------------------------------
    let (mut watcher, mut watch_rx) = (None, None);
    if config.watch {
        if let Some(path) = &shader_path {
            match make_watcher(path) {
                Ok((w, rx)) => {
                    watcher = Some(w);
                    watch_rx = Some(rx);
                    log::info!("watching {} for changes", path.display());
                }
                Err(e) => log::warn!("watch disabled: {e}"),
            }
        }
    }

    // --- egui settings panel (preview mode only, on the primary window) -----
    let ui = match config.mode {
        Mode::Preview => Some(Ui::new(&monitors[0].gpu, monitors[0].window.as_ref())),
        Mode::Wallpaper => None,
    };

    let now = Instant::now();
    Ok(State {
        _ctx: ctx,
        monitors,
        renderer,
        audio,
        reactive: Reactive::new(),
        uniforms: Uniforms::default(),
        mode: config.mode,
        ssaa,
        ui,
        settings: Settings {
            scene: scene_index,
            audio: audio_arg,
            gain,
            panel_open: config.mode == Mode::Preview,
            autostart: startup::autostart_enabled(),
        },
        scenes,
        scene_names,
        wallpaper_child: None,
        _stop_guard: stop_guard,
        _tray: tray_handle,
        start: now,
        last_frame: now,
        frame: 0,
        fps: 60.0,
        mouse: [0.0; 4],
        shader_path,
        _watcher: watcher,
        watch_rx,
    })
}

/// Create the platform windows: one resizable preview window, or one borderless
/// window per monitor in wallpaper mode.
fn create_windows(config: &Config, event_loop: &ActiveEventLoop) -> anyhow::Result<Vec<WinSpec>> {
    let mut specs = Vec::new();
    match config.mode {
        Mode::Preview => {
            let mut attrs = Window::default_attributes()
                .with_title("real_live_wall")
                .with_inner_size(PhysicalSize::new(config.width, config.height))
                .with_resizable(true);
            if config.top {
                attrs = attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
            }
            let window = Arc::new(event_loop.create_window(attrs)?);
            specs.push(WinSpec { window, rect: None });
        }
        Mode::Wallpaper => {
            let monitors: Vec<_> = event_loop.available_monitors().collect();
            for m in monitors {
                let pos = m.position();
                let size = m.size();
                let rect = MonitorRect {
                    x: pos.x,
                    y: pos.y,
                    w: size.width.max(1) as i32,
                    h: size.height.max(1) as i32,
                };
                let attrs = Window::default_attributes()
                    .with_title("real_live_wall")
                    .with_inner_size(size)
                    .with_position(pos)
                    .with_decorations(false)
                    .with_resizable(false);
                let window = Arc::new(event_loop.create_window(attrs)?);
                specs.push(WinSpec { window, rect: Some(rect) });
            }
            // Fallback: no enumerable monitors — one primary-sized window at (0,0).
            if specs.is_empty() {
                let size = event_loop
                    .primary_monitor()
                    .map(|m| m.size())
                    .unwrap_or(PhysicalSize::new(1920, 1080));
                let attrs = Window::default_attributes()
                    .with_title("real_live_wall")
                    .with_inner_size(size)
                    .with_position(PhysicalPosition::new(0, 0))
                    .with_decorations(false)
                    .with_resizable(false);
                let window = Arc::new(event_loop.create_window(attrs)?);
                let rect = MonitorRect { x: 0, y: 0, w: size.width.max(1) as i32, h: size.height.max(1) as i32 };
                specs.push(WinSpec { window, rect: Some(rect) });
            }
        }
    }
    Ok(specs)
}

/// Discover scenes and resolve the initial selection: `--shader` wins, then a
/// `preferred` scene name (e.g. the persisted last selection), else the default.
fn select_scenes(config: &Config, preferred: Option<&str>) -> (Vec<Scene>, usize) {
    let mut scenes = discover_scenes();
    let mut scene_index = 0usize;
    if let Some(path) = &config.shader {
        let canon = std::fs::canonicalize(path).ok();
        if let Some(pos) = scenes.iter().position(|s| match &s.source {
            SceneSource::Glsl(p) => std::fs::canonicalize(p).ok() == canon,
            SceneSource::Builtin => false,
        }) {
            scene_index = pos;
        } else {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "custom".to_string());
            scenes.push(Scene { name, source: SceneSource::Glsl(path.clone()) });
            scene_index = scenes.len() - 1;
        }
    } else if let Some(name) = preferred {
        if let Some(pos) = scenes.iter().position(|s| s.name == name) {
            scene_index = pos;
        }
    }
    (scenes, scene_index)
}

/// Load a scene into the renderer, returning the GLSL path (for hot-reload).
fn load_scene(gpu: &Gpu, renderer: &mut Renderer, scene: &Scene) -> Option<PathBuf> {
    match &scene.source {
        SceneSource::Builtin => {
            renderer.load_default(gpu);
            None
        }
        SceneSource::Glsl(path) => {
            match std::fs::read_to_string(path) {
                Ok(src) => match renderer.load_shadertoy_glsl(gpu, &src) {
                    Ok(()) => log::info!("loaded shader {}", path.display()),
                    Err(e) => log::error!("shader {} failed: {e:#}", path.display()),
                },
                Err(e) => log::error!("could not read shader {}: {e}", path.display()),
            }
            Some(path.clone())
        }
    }
}

fn make_watcher(path: &Path) -> anyhow::Result<(RecommendedWatcher, Receiver<()>)> {
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
    fn index_of(&self, id: WindowId) -> Option<usize> {
        self.monitors.iter().position(|m| m.window.id() == id)
    }

    /// Is a wallpaper currently live (our own child, or any process advertising
    /// the stop event)?
    fn wallpaper_active(&self) -> bool {
        self.wallpaper_child.is_some() || platform::wallpaper_running()
    }

    fn render_frame(&mut self) -> Vec<UiAction> {
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
                    match self.renderer.load_shadertoy_glsl(&self.monitors[0].gpu, &src) {
                        Ok(()) => log::info!("reloaded {}", path.display()),
                        Err(e) => log::error!("reload failed: {e:#}"),
                    }
                }
            }
        }

        // --- detect a wallpaper child that has exited -----------------------
        if let Some(child) = self.wallpaper_child.as_mut() {
            if matches!(child.try_wait(), Ok(Some(_))) {
                self.wallpaper_child = None;
            }
        }

        // --- reactive inputs (sampled once for all monitors) ----------------
        if let Some(a) = self.audio.as_mut() {
            a.set_gain(self.settings.gain);
        }
        let audio_frame = self.audio.as_mut().map(|a| a.analyze()).unwrap_or_default();
        let audio_active = self.audio.as_ref().map(|a| a.is_active()).unwrap_or(false);
        let (cpu, mem) = self.reactive.poll();
        let sample_rate = self.audio.as_ref().map(|a| a.sample_rate()).unwrap_or(44_100.0);

        // --- draw every monitor ---------------------------------------------
        let mut actions = Vec::new();
        for i in 0..self.monitors.len() {
            let (sw, sh) = self.monitors[i].postfx.scene_size();
            let (w, h) = self.monitors[i].gpu.size;
            let mx = sw as f32 / w.max(1) as f32;
            let my = sh as f32 / h.max(1) as f32;
            {
                let u = &mut self.uniforms;
                u.resolution = [sw as f32, sh as f32, 1.0, sw as f32 / sh.max(1) as f32];
                u.mouse =
                    [self.mouse[0] * mx, self.mouse[1] * my, self.mouse[2] * mx, self.mouse[3] * my];
                u.time = [t, dt, self.frame as f32, sample_rate];
                u.audio =
                    [audio_frame.bass, audio_frame.mid, audio_frame.treble, audio_frame.volume];
                u.sys = [cpu, mem, audio_frame.bass, self.fps];
                u.set_spectrum(&audio_frame.spectrum);
            }

            if i == 0 && self.ui.is_some() {
                let meters = Meters {
                    fps: self.fps,
                    resolution: [w, h],
                    audio_active,
                    bass: audio_frame.bass,
                    mid: audio_frame.mid,
                    treble: audio_frame.treble,
                    volume: audio_frame.volume,
                    cpu,
                    mem,
                    wallpaper_running: self.wallpaper_active(),
                };
                actions = self.render_primary(meters);
            } else {
                let renderer = &self.renderer;
                let uniforms = &self.uniforms;
                self.monitors[i].render(renderer, uniforms);
            }
        }
        actions
    }

    /// Render monitor 0 with the egui settings panel composited on top.
    fn render_primary(&mut self, meters: Meters) -> Vec<UiAction> {
        // Split-borrow distinct fields so the GUI, renderer and target coexist.
        let renderer = &self.renderer;
        let uniforms = &self.uniforms;
        let settings = &mut self.settings;
        let scene_names = &self.scene_names;
        let Some(ui) = self.ui.as_mut() else {
            return Vec::new();
        };
        let target = &mut self.monitors[0];

        renderer.update_uniforms(&target.gpu, uniforms);
        let Some(frame) = target.gpu.acquire() else {
            return Vec::new();
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = target
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("rlw-encoder") });

        renderer.draw_scene(&target.gpu, target.postfx.scene_view(), &mut encoder);
        target.postfx.render(&mut encoder, &view);

        let (actions, user_bufs) = ui.draw(
            &target.gpu,
            target.window.as_ref(),
            &view,
            &mut encoder,
            settings,
            scene_names,
            &meters,
        );

        target
            .gpu
            .queue
            .submit(user_bufs.into_iter().chain(std::iter::once(encoder.finish())));
        frame.present();
        actions
    }

    fn apply_action(&mut self, action: UiAction, event_loop: &ActiveEventLoop) {
        match action {
            UiAction::SelectScene(i) => {
                if let Some(scene) = self.scenes.get(i) {
                    self.shader_path = load_scene(&self.monitors[0].gpu, &mut self.renderer, scene);
                }
                self.save_prefs();
            }
            UiAction::Reload => {
                if let Some(scene) = self.scenes.get(self.settings.scene) {
                    self.shader_path = load_scene(&self.monitors[0].gpu, &mut self.renderer, scene);
                }
            }
            UiAction::SetAudio(a) => {
                self.settings.audio = a;
                self.audio = a.to_source().map(|src| AudioEngine::new(src, self.settings.gain));
                self.save_prefs();
            }
            UiAction::ToggleWallpaper => self.toggle_wallpaper(),
            UiAction::SetAutostart(on) => {
                let cmd = self.autostart_command();
                match startup::set_autostart(on, &cmd) {
                    Ok(()) => self.settings.autostart = on,
                    Err(e) => log::error!("autostart toggle failed: {e:#}"),
                }
                self.save_prefs();
            }
            UiAction::Quit => {
                self.save_prefs();
                event_loop.exit();
            }
        }
    }

    /// Route a tray-menu command (wallpaper process).
    fn on_tray(&mut self, cmd: TrayCommand, event_loop: &ActiveEventLoop) {
        match cmd {
            TrayCommand::OpenSettings => self.open_settings(),
            TrayCommand::NextScene => self.cycle_scene(),
            TrayCommand::ToggleAutostart => self.toggle_autostart(),
            TrayCommand::Quit => event_loop.exit(),
        }
    }

    /// Spawn a fresh preview/settings window process.
    fn open_settings(&self) {
        if let Ok(exe) = std::env::current_exe() {
            match std::process::Command::new(exe).arg("--mode").arg("preview").spawn() {
                Ok(_) => log::info!("opened settings window"),
                Err(e) => log::error!("failed to open settings: {e}"),
            }
        }
    }

    /// Advance to the next scene (used by the tray in wallpaper mode). The scene
    /// renderer is shared, so one load applies to every monitor.
    fn cycle_scene(&mut self) {
        if self.scenes.is_empty() {
            return;
        }
        self.settings.scene = (self.settings.scene + 1) % self.scenes.len();
        if let Some(scene) = self.scenes.get(self.settings.scene) {
            self.shader_path = load_scene(&self.monitors[0].gpu, &mut self.renderer, scene);
            log::info!("tray: scene → {}", self.scene_names.get(self.settings.scene).map(|s| s.as_str()).unwrap_or("?"));
        }
    }

    /// The command line that autostart should run to reproduce the current scene.
    fn autostart_command(&self) -> String {
        let exe = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "real_live_wall.exe".to_string());
        let mut cmd = format!("\"{exe}\" --mode wallpaper");
        if let Some(scene) = self.scenes.get(self.settings.scene) {
            if let SceneSource::Glsl(p) = &scene.source {
                let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                cmd.push_str(&format!(" --shader \"{}\"", abs.display()));
            }
        }
        cmd.push_str(&format!(
            " --audio {} --gain {} --ssaa {}",
            self.settings.audio.as_cli(),
            self.settings.gain,
            self.ssaa
        ));
        cmd
    }

    fn toggle_autostart(&mut self) {
        let enable = !startup::autostart_enabled();
        let cmd = self.autostart_command();
        match startup::set_autostart(enable, &cmd) {
            Ok(()) => {
                self.settings.autostart = enable;
                log::info!("autostart {}", if enable { "enabled" } else { "disabled" });
            }
            Err(e) => log::error!("autostart toggle failed: {e:#}"),
        }
        self.save_prefs();
    }

    /// Persist the current preferences (preview process only — the wallpaper
    /// process is launched with explicit args and must not overwrite them).
    fn save_prefs(&self) {
        if self.mode != Mode::Preview {
            return;
        }
        let scene = self.scenes.get(self.settings.scene).and_then(|s| match s.source {
            SceneSource::Builtin => None,
            SceneSource::Glsl(_) => Some(s.name.clone()),
        });
        PersistConfig {
            scene,
            audio: self.settings.audio.as_cli().to_string(),
            gain: self.settings.gain,
            ssaa: self.ssaa,
            autostart: startup::autostart_enabled(),
        }
        .save();
    }

    fn toggle_wallpaper(&mut self) {
        // Stop whatever is running — our own child and/or any other instance.
        if self.wallpaper_active() {
            if platform::signal_stop() {
                log::info!("wallpaper stop requested");
            }
            // Detach our child handle; it exits on its own from the stop signal.
            let _ = self.wallpaper_child.take();
            return;
        }
        match self.spawn_wallpaper() {
            Ok(child) => {
                self.wallpaper_child = Some(child);
                log::info!("wallpaper started");
            }
            Err(e) => log::error!("failed to start wallpaper: {e:#}"),
        }
    }

    fn spawn_wallpaper(&self) -> anyhow::Result<Child> {
        let exe = std::env::current_exe()?;
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--mode").arg("wallpaper");
        if let Some(scene) = self.scenes.get(self.settings.scene) {
            if let SceneSource::Glsl(p) = &scene.source {
                cmd.arg("--shader").arg(p);
            }
        }
        cmd.arg("--audio").arg(self.settings.audio.as_cli());
        cmd.arg("--gain").arg(format!("{}", self.settings.gain));
        Ok(cmd.spawn()?)
    }
}
