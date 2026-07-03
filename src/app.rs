//! The winit application: window lifecycle, the per-frame update/render loop,
//! input, audio/system sampling, the egui settings panel, GLSL hot-reload and
//! spawning a detached wallpaper process.

use std::path::{Path, PathBuf};
use std::process::Child;
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
use crate::postfx::PostFx;
use crate::reactive::Reactive;
use crate::renderer::Renderer;
use crate::ui::{Meters, Settings, Ui, UiAction};
use crate::uniforms::Uniforms;

/// A selectable scene: the built-in WGSL aurora, or a Shadertoy GLSL file.
enum SceneSource {
    Builtin,
    Glsl(PathBuf),
}

struct Scene {
    name: String,
    source: SceneSource,
}

fn discover_scenes() -> Vec<Scene> {
    let mut scenes = vec![Scene {
        name: "기본 — 오로라 + 스펙트럼".to_string(),
        source: SceneSource::Builtin,
    }];
    if let Ok(rd) = std::fs::read_dir("shaders") {
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
    scenes
}

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
    postfx: PostFx,
    audio: Option<AudioEngine>,
    reactive: Reactive,
    uniforms: Uniforms,

    ui: Option<Ui>,
    settings: Settings,
    scenes: Vec<Scene>,
    scene_names: Vec<String>,
    wallpaper_child: Option<Child>,

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

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };

        // Let egui see the event first (it may want exclusive use of it).
        let egui_consumed = if let Some(ui) = state.ui.as_mut() {
            ui.on_window_event(state.window.as_ref(), &event)
        } else {
            false
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::F1) => {
                            state.settings.panel_open = !state.settings.panel_open;
                        }
                        PhysicalKey::Code(KeyCode::Escape) if self.config.mode == Mode::Preview => {
                            event_loop.exit();
                        }
                        _ => {}
                    }
                }
            }

            WindowEvent::Resized(size) => {
                state.gpu.resize(size.width, size.height);
                state.postfx.resize(&state.gpu, size.width, size.height);
                state.renderer.rebind(&state.gpu);
            }

            WindowEvent::CursorMoved { position, .. } if !egui_consumed => {
                let h = state.gpu.size.1 as f32;
                state.mouse[0] = position.x as f32;
                state.mouse[1] = h - position.y as f32;
            }

            WindowEvent::MouseInput { state: btn, button, .. }
                if !egui_consumed && button == MouseButton::Left && btn == ElementState::Pressed =>
            {
                state.mouse[2] = state.mouse[0];
                state.mouse[3] = state.mouse[1];
            }

            WindowEvent::RedrawRequested => {
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

    // --- gpu + renderer + post-processing -----------------------------------
    let gpu = Gpu::new(window.clone())?;
    let mut renderer = Renderer::new(&gpu)?;
    let postfx = PostFx::new(&gpu, config.ssaa.clamp(1.0, 2.0));

    // --- scenes + initial selection -----------------------------------------
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
    }
    let shader_path = load_scene(&gpu, &mut renderer, &scenes[scene_index]);
    let scene_names = scenes.iter().map(|s| s.name.clone()).collect();

    // --- wallpaper attach (only when this instance IS the wallpaper) --------
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

    // --- egui settings panel (window/preview mode only) ---------------------
    let ui = match config.mode {
        Mode::Preview => Some(Ui::new(&gpu, window.as_ref())),
        Mode::Wallpaper => None,
    };

    let now = Instant::now();
    Ok(State {
        window,
        gpu,
        renderer,
        postfx,
        audio,
        reactive: Reactive::new(),
        uniforms: Uniforms::default(),
        ui,
        settings: Settings {
            scene: scene_index,
            audio: config.audio,
            gain: config.gain,
            panel_open: config.mode == Mode::Preview,
        },
        scenes,
        scene_names,
        wallpaper_child: None,
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
                    match self.renderer.load_shadertoy_glsl(&self.gpu, &src) {
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

        // --- reactive inputs ------------------------------------------------
        if let Some(a) = self.audio.as_mut() {
            a.set_gain(self.settings.gain);
        }
        let audio_frame = self.audio.as_mut().map(|a| a.analyze()).unwrap_or_default();
        let audio_active = self.audio.as_ref().map(|a| a.is_active()).unwrap_or(false);
        let (cpu, mem) = self.reactive.poll();
        let sample_rate = self.audio.as_ref().map(|a| a.sample_rate()).unwrap_or(44_100.0);

        // --- fill uniforms (scene renders at super-sampled resolution) ------
        let (w, h) = self.gpu.size;
        let (sw, sh) = self.postfx.scene_size();
        let mx = sw as f32 / w.max(1) as f32;
        let my = sh as f32 / h.max(1) as f32;
        {
            let u = &mut self.uniforms;
            u.resolution = [sw as f32, sh as f32, 1.0, sw as f32 / sh.max(1) as f32];
            u.mouse = [self.mouse[0] * mx, self.mouse[1] * my, self.mouse[2] * mx, self.mouse[3] * my];
            u.time = [t, dt, self.frame as f32, sample_rate];
            u.audio = [audio_frame.bass, audio_frame.mid, audio_frame.treble, audio_frame.volume];
            u.sys = [cpu, mem, audio_frame.bass, self.fps];
            u.set_spectrum(&audio_frame.spectrum);
        }
        self.renderer.update_uniforms(&self.gpu, &self.uniforms);

        // --- acquire + draw -------------------------------------------------
        let Some(frame) = self.gpu.acquire() else {
            return Vec::new();
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("rlw-encoder") });

        // Scene → HDR super-sampled target, then bloom/tonemap → swapchain.
        self.renderer.draw_scene(&self.gpu, self.postfx.scene_view(), &mut encoder);
        self.postfx.render(&mut encoder, &view);

        let mut actions = Vec::new();
        let mut user_bufs = Vec::new();
        if let Some(ui) = self.ui.as_mut() {
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
                wallpaper_running: self.wallpaper_child.is_some(),
            };
            let (a, b) = ui.draw(
                &self.gpu,
                self.window.as_ref(),
                &view,
                &mut encoder,
                &mut self.settings,
                &self.scene_names,
                &meters,
            );
            actions = a;
            user_bufs = b;
        }

        self.gpu
            .queue
            .submit(user_bufs.into_iter().chain(std::iter::once(encoder.finish())));
        frame.present();
        actions
    }

    fn apply_action(&mut self, action: UiAction, event_loop: &ActiveEventLoop) {
        match action {
            UiAction::SelectScene(i) => {
                if let Some(scene) = self.scenes.get(i) {
                    self.shader_path = load_scene(&self.gpu, &mut self.renderer, scene);
                }
            }
            UiAction::Reload => {
                if let Some(scene) = self.scenes.get(self.settings.scene) {
                    self.shader_path = load_scene(&self.gpu, &mut self.renderer, scene);
                }
            }
            UiAction::SetAudio(a) => {
                self.settings.audio = a;
                self.audio = a.to_source().map(|src| AudioEngine::new(src, self.settings.gain));
            }
            UiAction::ToggleWallpaper => self.toggle_wallpaper(),
            UiAction::Quit => event_loop.exit(),
        }
    }

    fn toggle_wallpaper(&mut self) {
        if let Some(mut child) = self.wallpaper_child.take() {
            let _ = child.kill();
            let _ = child.wait();
            log::info!("wallpaper stopped");
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
