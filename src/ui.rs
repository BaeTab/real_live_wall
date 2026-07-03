//! egui-based settings panel composited on top of the live shader.
//!
//! The panel (toggle with F1) lets you pick a scene/shader, choose the audio
//! source and gain, watch live audio/CPU meters, and push the current scene to
//! the actual desktop as a wallpaper. It only exists in preview/window mode.

use winit::window::Window;

use crate::config::AudioArg;
use crate::gpu::Gpu;

/// User-adjustable settings mutated directly by the egui widgets.
pub struct Settings {
    pub scene: usize,
    pub audio: AudioArg,
    pub gain: f32,
    pub panel_open: bool,
    pub autostart: bool,
}

/// Live read-only values shown as meters.
#[derive(Default)]
pub struct Meters {
    pub fps: f32,
    pub resolution: [u32; 2],
    pub audio_active: bool,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub volume: f32,
    pub cpu: f32,
    pub mem: f32,
    pub wallpaper_running: bool,
}

/// Actions the UI asks the app to perform (applied after the frame is drawn).
#[derive(Clone, Copy, Debug)]
pub enum UiAction {
    SelectScene(usize),
    SetAudio(AudioArg),
    ToggleWallpaper,
    SetAutostart(bool),
    Reload,
    Quit,
}

pub struct Ui {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

impl Ui {
    pub fn new(gpu: &Gpu, window: &Window) -> Self {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        install_cjk_font(&ctx);
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let renderer =
            egui_wgpu::Renderer::new(&gpu.device, gpu.config.format, egui_wgpu::RendererOptions::default());
        Self { ctx, state, renderer }
    }

    /// Feed a window event to egui. Returns true if egui consumed it.
    pub fn on_window_event(&mut self, window: &Window, event: &winit::event::WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// Build the panel and record its draw calls on top of the scene. Returns
    /// the requested actions plus any command buffers egui produced.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        window: &Window,
        view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        settings: &mut Settings,
        scene_names: &[String],
        meters: &Meters,
    ) -> (Vec<UiAction>, Vec<wgpu::CommandBuffer>) {
        let mut actions = Vec::new();
        let raw_input = self.state.take_egui_input(window);

        let full = self.ctx.run_ui(raw_input, |ui| {
            if !settings.panel_open {
                egui::Area::new(egui::Id::new("hint"))
                    .fixed_pos(egui::pos2(12.0, 12.0))
                    .show(ui.ctx(), |ui| {
                        ui.label(
                            egui::RichText::new("F1 — 설정")
                                .color(egui::Color32::from_white_alpha(150))
                                .size(13.0),
                        );
                    });
                return;
            }

            egui::Panel::right(egui::Id::new("settings"))
                .resizable(false)
                .default_size(320.0)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    ui.heading("real_live_wall");
                    ui.label(
                        egui::RichText::new("리액티브 라이브 월페이퍼 엔진")
                            .weak()
                            .size(12.0),
                    );
                    ui.separator();

                    // --- scene ---------------------------------------------
                    ui.strong("씬 / 셰이더");
                    let prev_scene = settings.scene;
                    egui::ComboBox::from_id_salt("scene")
                        .width(260.0)
                        .selected_text(scene_names.get(settings.scene).map(|s| s.as_str()).unwrap_or("?"))
                        .show_ui(ui, |ui| {
                            for (i, name) in scene_names.iter().enumerate() {
                                ui.selectable_value(&mut settings.scene, i, name);
                            }
                        });
                    if settings.scene != prev_scene {
                        actions.push(UiAction::SelectScene(settings.scene));
                    }
                    if ui.button("↻ 리로드").clicked() {
                        actions.push(UiAction::Reload);
                    }

                    ui.add_space(8.0);
                    ui.separator();

                    // --- audio ---------------------------------------------
                    ui.strong("오디오");
                    let prev_audio = settings.audio;
                    egui::ComboBox::from_id_salt("audio")
                        .width(260.0)
                        .selected_text(audio_label(settings.audio))
                        .show_ui(ui, |ui| {
                            for a in [AudioArg::Auto, AudioArg::Loopback, AudioArg::Input, AudioArg::Off] {
                                ui.selectable_value(&mut settings.audio, a, audio_label(a));
                            }
                        });
                    if settings.audio != prev_audio {
                        actions.push(UiAction::SetAudio(settings.audio));
                    }
                    ui.add(egui::Slider::new(&mut settings.gain, 0.5..=20.0).text("게인"));

                    ui.add_space(8.0);
                    ui.separator();

                    // --- live meters ---------------------------------------
                    ui.strong("실시간");
                    ui.label(format!(
                        "FPS {:.0}   ·   {}×{}",
                        meters.fps, meters.resolution[0], meters.resolution[1]
                    ));
                    if meters.audio_active {
                        meter(ui, "Bass", meters.bass, egui::Color32::from_rgb(80, 140, 255));
                        meter(ui, "Mid", meters.mid, egui::Color32::from_rgb(120, 220, 160));
                        meter(ui, "Treble", meters.treble, egui::Color32::from_rgb(255, 120, 160));
                        meter(ui, "Vol", meters.volume, egui::Color32::from_rgb(230, 230, 120));
                    } else {
                        ui.weak("오디오 비활성 (소리 재생 중 loopback 선택)");
                    }
                    meter(ui, "CPU", meters.cpu, egui::Color32::from_rgb(255, 170, 90));
                    meter(ui, "Mem", meters.mem, egui::Color32::from_rgb(180, 160, 255));

                    ui.add_space(10.0);
                    ui.separator();

                    // --- wallpaper apply -----------------------------------
                    let (label, color) = if meters.wallpaper_running {
                        ("■ 월페이퍼 중지", egui::Color32::from_rgb(200, 80, 80))
                    } else {
                        ("▶ 바탕화면에 적용", egui::Color32::from_rgb(70, 150, 90))
                    };
                    if ui
                        .add_sized([260.0, 30.0], egui::Button::new(label).fill(color))
                        .clicked()
                    {
                        actions.push(UiAction::ToggleWallpaper);
                    }
                    if meters.wallpaper_running {
                        ui.weak("현재 씬이 데스크톱 배경으로 실행 중입니다. (트레이 아이콘에서 제어)");
                    }

                    ui.add_space(6.0);
                    let mut autostart = settings.autostart;
                    if ui.checkbox(&mut autostart, "로그인 시 자동 시작").changed() {
                        settings.autostart = autostart;
                        actions.push(UiAction::SetAutostart(autostart));
                    }

                    ui.add_space(6.0);
                    if ui.button("종료").clicked() {
                        actions.push(UiAction::Quit);
                    }

                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        ui.add_space(4.0);
                        ui.weak("F1 로 이 패널을 숨깁니다");
                    });
                });
        });

        self.state
            .handle_platform_output(window, full.platform_output);
        let tris = self.ctx.tessellate(full.shapes, full.pixels_per_point);
        for (id, delta) in &full.textures_delta.set {
            self.renderer.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [gpu.size.0, gpu.size.1],
            pixels_per_point: full.pixels_per_point,
        };
        let user_bufs = self
            .renderer
            .update_buffers(&gpu.device, &gpu.queue, encoder, &tris, &screen);

        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // keep the shader underneath
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &tris, &screen);
        }

        for id in &full.textures_delta.free {
            self.renderer.free_texture(id);
        }

        (actions, user_bufs)
    }
}

/// egui's built-in font has no Korean/CJK glyphs, so load a system font as a
/// fallback. Tries common Windows/macOS/Linux Korean fonts; silently does
/// nothing (Latin still works) if none are found.
fn install_cjk_font(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        "C:/Windows/Fonts/malgun.ttf",              // Malgun Gothic (Windows)
        "C:/Windows/Fonts/NanumGothic.ttf",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc", // macOS
        "/usr/share/fonts/truetype/nanum/NanumGothic.ttf", // Linux
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    ];
    for path in CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("cjk".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push("cjk".to_owned());
            }
            ctx.set_fonts(fonts);
            log::info!("ui: loaded CJK font {path}");
            return;
        }
    }
    log::warn!("ui: no CJK font found; Korean labels may not render");
}

fn audio_label(a: AudioArg) -> &'static str {
    match a {
        AudioArg::Auto => "자동 (Windows=루프백)",
        AudioArg::Loopback => "루프백 (재생 소리)",
        AudioArg::Input => "입력 (마이크)",
        AudioArg::Off => "끄기",
    }
}

fn meter(ui: &mut egui::Ui, label: &str, v: f32, color: egui::Color32) {
    ui.add(
        egui::ProgressBar::new(v.clamp(0.0, 1.0))
            .desired_width(260.0)
            .fill(color)
            .text(egui::RichText::new(label).size(11.0)),
    );
}
