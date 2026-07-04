//! egui-based settings panel composited on top of the live shader.
//!
//! The panel (toggle with F1) lets you pick a scene/shader, choose the audio
//! source and gain, watch live audio/CPU meters, and push the current scene to
//! the actual desktop as a wallpaper. It only exists in preview/window mode.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use winit::window::Window;

use crate::config::AudioArg;
use crate::gpu::Gpu;

/// Aurora-teal accent used for headers, selections and highlights.
const ACCENT: egui::Color32 = egui::Color32::from_rgb(52, 214, 178);
/// Primary text colour (soft off-white).
const TEXT: egui::Color32 = egui::Color32::from_rgb(228, 233, 242);
/// Secondary/weak text colour.
const TEXT_WEAK: egui::Color32 = egui::Color32::from_rgb(150, 160, 178);

/// User-adjustable settings mutated directly by the egui widgets.
pub struct Settings {
    pub scene: usize,
    pub audio: AudioArg,
    pub gain: f32,
    pub panel_open: bool,
    pub autostart: bool,
    /// Auto-cycle scenes on a timer (playlist mode).
    pub playlist_enabled: bool,
    /// Seconds between automatic scene changes.
    pub playlist_interval_secs: u64,
    /// Random next scene instead of sequential.
    pub playlist_shuffle: bool,
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
    pub bpm: f32,
    pub cpu: f32,
    pub mem: f32,
    pub wallpaper_running: bool,
    // Now-playing (SMTC): shown when music is detected.
    pub has_music: bool,
    pub music_title: String,
    pub music_artist: String,
    pub palette: [[f32; 3]; 4],
    /// A newer GitHub release version (e.g. "1.2.0"), if the update check found one.
    pub update_version: Option<String>,
}

/// Actions the UI asks the app to perform (applied after the frame is drawn).
#[derive(Clone, Copy, Debug)]
pub enum UiAction {
    SelectScene(usize),
    SetAudio(AudioArg),
    ToggleWallpaper,
    SetAutostart(bool),
    Reload,
    /// Playlist settings changed — persist and reset the auto-cycle timer.
    PlaylistChanged,
    /// Download and install the available GitHub-release update.
    ApplyUpdate,
    Quit,
}

pub struct Ui {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    /// Lazily-loaded scene thumbnails, keyed by thumbnail key (None = tried,
    /// no file). Textures live for the process lifetime.
    thumbs: HashMap<String, Option<egui::TextureHandle>>,
    thumbs_dir: Option<PathBuf>,
}

impl Ui {
    pub fn new(gpu: &Gpu, window: &Window) -> Self {
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| *s = premium_style());
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
        Self { ctx, state, renderer, thumbs: HashMap::new(), thumbs_dir: thumbnails_dir() }
    }

    /// Get (loading + caching on first use) the thumbnail texture for a key.
    fn thumbnail(&mut self, key: &str) -> Option<egui::TextureHandle> {
        if let Some(cached) = self.thumbs.get(key) {
            return cached.clone();
        }
        let handle = self.thumbs_dir.as_ref().and_then(|d| load_thumb(&self.ctx, d, key));
        self.thumbs.insert(key.to_string(), handle.clone());
        handle
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
        thumb_keys: &[String],
        meters: &Meters,
    ) -> (Vec<UiAction>, Vec<wgpu::CommandBuffer>) {
        let mut actions = Vec::new();
        // Load (once) the thumbnail texture for every scene before building the
        // panel, so the closure below can borrow them by reference.
        let thumbs: Vec<Option<egui::TextureHandle>> =
            thumb_keys.iter().map(|k| self.thumbnail(k)).collect();
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
                .default_size(352.0)
                .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    // --- header ----------------------------------------------
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("real_live_wall").color(TEXT).size(21.0).strong());
                    ui.label(
                        egui::RichText::new(format!(
                            "리액티브 라이브 월페이퍼 · v{}",
                            env!("CARGO_PKG_VERSION")
                        ))
                        .color(TEXT_WEAK)
                        .size(11.0),
                    );
                    ui.add_space(10.0);

                    // --- update banner (GitHub release) ----------------------
                    if let Some(ver) = &meters.update_version {
                        card(ui, |ui| {
                            ui.colored_label(ACCENT, format!("⬆  새 버전 v{ver} 사용 가능"));
                            ui.add_space(6.0);
                            let w = ui.available_width();
                            if ui
                                .add_sized(
                                    [w, 30.0],
                                    egui::Button::new(egui::RichText::new("다운로드 & 설치").strong())
                                        .fill(egui::Color32::from_rgb(46, 160, 120)),
                                )
                                .clicked()
                            {
                                actions.push(UiAction::ApplyUpdate);
                            }
                        });
                        ui.add_space(10.0);
                    }

                    // --- scene gallery ---------------------------------------
                    card(ui, |ui| {
                        section(ui, "씬 / 셰이더");
                        ui.label(
                            egui::RichText::new(
                                scene_names.get(settings.scene).map(|s| s.as_str()).unwrap_or("?"),
                            )
                            .color(TEXT)
                            .size(12.5),
                        );
                        ui.add_space(6.0);
                        // Size cells so exactly two columns fit the card width.
                        let inner = ui.available_width();
                        let cw = ((inner - 7.0) / 2.0 - 20.0).clamp(96.0, 156.0);
                        let cell = egui::vec2(cw, cw * 0.5625);
                        egui::Grid::new("scene_thumbs").spacing(egui::vec2(7.0, 7.0)).show(ui, |ui| {
                            for i in 0..scene_names.len() {
                                let selected = i == settings.scene;
                                let clicked = if let Some(Some(tex)) = thumbs.get(i) {
                                    let img = egui::Image::from_texture(
                                        egui::load::SizedTexture::new(tex.id(), cell),
                                    );
                                    ui.add(egui::Button::image(img).selected(selected))
                                        .on_hover_text(scene_names[i].as_str())
                                        .clicked()
                                } else {
                                    ui.selectable_label(selected, scene_names[i].as_str()).clicked()
                                };
                                if clicked && i != settings.scene {
                                    settings.scene = i;
                                    actions.push(UiAction::SelectScene(i));
                                }
                                if i % 2 == 1 {
                                    ui.end_row();
                                }
                            }
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button("↻ 리로드").clicked() {
                                actions.push(UiAction::Reload);
                            }
                            let mut pl = settings.playlist_enabled;
                            if ui.checkbox(&mut pl, "자동 순환").changed() {
                                settings.playlist_enabled = pl;
                                actions.push(UiAction::PlaylistChanged);
                            }
                        });
                        if settings.playlist_enabled {
                            let mut mins = (settings.playlist_interval_secs as f32 / 60.0).max(0.5);
                            if ui
                                .add(egui::Slider::new(&mut mins, 0.5..=60.0).suffix(" 분"))
                                .changed()
                            {
                                settings.playlist_interval_secs =
                                    (mins * 60.0).round().max(30.0) as u64;
                                actions.push(UiAction::PlaylistChanged);
                            }
                            let mut sh = settings.playlist_shuffle;
                            if ui.checkbox(&mut sh, "셔플").changed() {
                                settings.playlist_shuffle = sh;
                                actions.push(UiAction::PlaylistChanged);
                            }
                        }
                    });
                    ui.add_space(10.0);

                    // --- audio -----------------------------------------------
                    card(ui, |ui| {
                        section(ui, "오디오");
                        let prev_audio = settings.audio;
                        let w = ui.available_width();
                        egui::ComboBox::from_id_salt("audio")
                            .width(w)
                            .selected_text(audio_label(settings.audio))
                            .show_ui(ui, |ui| {
                                for a in
                                    [AudioArg::Auto, AudioArg::Loopback, AudioArg::Input, AudioArg::Off]
                                {
                                    ui.selectable_value(&mut settings.audio, a, audio_label(a));
                                }
                            });
                        if settings.audio != prev_audio {
                            actions.push(UiAction::SetAudio(settings.audio));
                        }
                        ui.add(egui::Slider::new(&mut settings.gain, 0.5..=20.0).text("게인"));
                        ui.add_space(4.0);
                        if meters.audio_active {
                            meter(ui, "Bass", meters.bass, egui::Color32::from_rgb(80, 140, 255));
                            meter(ui, "Mid", meters.mid, egui::Color32::from_rgb(120, 220, 160));
                            meter(ui, "Treble", meters.treble, egui::Color32::from_rgb(255, 120, 160));
                            meter(ui, "Vol", meters.volume, egui::Color32::from_rgb(230, 230, 120));
                            if meters.bpm > 0.0 {
                                ui.colored_label(ACCENT, format!("♩ {:.0} BPM · 비트 감지", meters.bpm));
                            }
                        } else {
                            ui.weak("오디오 비활성 — 소리 재생 중 loopback 선택");
                        }
                    });
                    ui.add_space(10.0);

                    // --- now playing (SMTC) + album-art palette --------------
                    if meters.has_music {
                        card(ui, |ui| {
                            section(ui, "♪ 지금 재생 중");
                            if !meters.music_title.is_empty() {
                                ui.label(
                                    egui::RichText::new(meters.music_title.as_str())
                                        .color(TEXT)
                                        .size(13.0)
                                        .strong(),
                                );
                            }
                            if !meters.music_artist.is_empty() {
                                ui.label(
                                    egui::RichText::new(meters.music_artist.as_str())
                                        .color(TEXT_WEAK)
                                        .size(11.5),
                                );
                            }
                            ui.add_space(6.0);
                            let sw = (ui.available_width() - 3.0 * 6.0) / 4.0;
                            ui.horizontal(|ui| {
                                for c in &meters.palette {
                                    let col = egui::Color32::from_rgb(
                                        (c[0] * 255.0).clamp(0.0, 255.0) as u8,
                                        (c[1] * 255.0).clamp(0.0, 255.0) as u8,
                                        (c[2] * 255.0).clamp(0.0, 255.0) as u8,
                                    );
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(sw.max(20.0), 22.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(rect, egui::CornerRadius::same(4), col);
                                }
                            });
                        });
                        ui.add_space(10.0);
                    }

                    // --- system ----------------------------------------------
                    card(ui, |ui| {
                        section(ui, "시스템");
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{:.0}", meters.fps))
                                    .color(ACCENT)
                                    .size(15.0)
                                    .strong(),
                            );
                            ui.label(
                                egui::RichText::new(format!(
                                    "FPS · {}×{}",
                                    meters.resolution[0], meters.resolution[1]
                                ))
                                .color(TEXT_WEAK)
                                .size(11.5),
                            );
                        });
                        ui.add_space(2.0);
                        meter(ui, "CPU", meters.cpu, egui::Color32::from_rgb(255, 170, 90));
                        meter(ui, "Mem", meters.mem, egui::Color32::from_rgb(180, 160, 255));
                    });
                    ui.add_space(12.0);

                    // --- primary action + toggles ----------------------------
                    let (label, color) = if meters.wallpaper_running {
                        ("■  월페이퍼 중지", egui::Color32::from_rgb(196, 76, 84))
                    } else {
                        ("▶  바탕화면에 적용", egui::Color32::from_rgb(46, 160, 120))
                    };
                    let w = ui.available_width();
                    if ui
                        .add_sized(
                            [w, 34.0],
                            egui::Button::new(egui::RichText::new(label).size(14.0).strong())
                                .fill(color),
                        )
                        .clicked()
                    {
                        actions.push(UiAction::ToggleWallpaper);
                    }
                    if meters.wallpaper_running {
                        ui.add_space(2.0);
                        ui.weak("데스크톱 배경으로 실행 중 · 트레이에서 제어");
                    }
                    ui.add_space(8.0);
                    let mut autostart = settings.autostart;
                    if ui.checkbox(&mut autostart, "로그인 시 자동 시작").changed() {
                        settings.autostart = autostart;
                        actions.push(UiAction::SetAutostart(autostart));
                    }
                    ui.add_space(6.0);
                    let w = ui.available_width();
                    if ui.add_sized([w, 26.0], egui::Button::new("종료")).clicked() {
                        actions.push(UiAction::Quit);
                    }
                    ui.add_space(8.0);
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

/// A bespoke dark theme: deep translucent panel (the scene glows through),
/// aurora-teal accents, rounded widgets and roomy spacing.
fn premium_style() -> egui::Style {
    use egui::{Color32, CornerRadius, Stroke};
    let mut style = egui::Style::default();
    let r = CornerRadius::same(7);

    let v = &mut style.visuals;
    v.dark_mode = true;
    v.override_text_color = Some(TEXT);
    v.panel_fill = Color32::from_rgba_unmultiplied(14, 16, 24, 236);
    v.window_fill = Color32::from_rgba_unmultiplied(14, 16, 24, 236);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 20));
    v.window_corner_radius = CornerRadius::same(10);
    v.faint_bg_color = Color32::from_rgba_unmultiplied(255, 255, 255, 8);
    v.extreme_bg_color = Color32::from_rgb(9, 11, 17);
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(52, 214, 178, 96);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = Color32::TRANSPARENT;
    w.noninteractive.weak_bg_fill = Color32::TRANSPARENT;
    w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 14));
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_WEAK);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = Color32::from_rgb(32, 37, 51);
    w.inactive.weak_bg_fill = Color32::from_rgb(30, 35, 48);
    w.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 16));
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = r;
    w.hovered.bg_fill = Color32::from_rgb(44, 51, 70);
    w.hovered.weak_bg_fill = Color32::from_rgb(44, 51, 70);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(52, 214, 178, 150));
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 1.0;
    w.active.bg_fill = ACCENT;
    w.active.weak_bg_fill = Color32::from_rgba_unmultiplied(52, 214, 178, 70);
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5, Color32::from_rgb(10, 14, 18));
    w.active.corner_radius = r;
    w.active.expansion = 1.0;
    w.open.bg_fill = Color32::from_rgb(32, 37, 51);
    w.open.weak_bg_fill = Color32::from_rgb(32, 37, 51);
    w.open.bg_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(52, 214, 178, 130));
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = r;

    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 8.0);
    s.button_padding = egui::vec2(10.0, 6.0);
    s.interact_size.y = 26.0;
    s.slider_width = 176.0;
    s.indent = 14.0;
    style
}

/// A rounded translucent "card" grouping one section of the panel.
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .fill(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 10))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 18)))
        .corner_radius(egui::CornerRadius::same(9))
        .inner_margin(egui::Margin::same(11))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A small accent-coloured section header.
fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(ACCENT).size(12.0).strong());
    ui.add_space(4.0);
}

/// Locate the `assets/thumbnails/` directory (cwd first, then next to the exe),
/// mirroring how the engine finds `shaders/`.
fn thumbnails_dir() -> Option<PathBuf> {
    let cwd = PathBuf::from("assets/thumbnails");
    if cwd.is_dir() {
        return Some(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("assets").join("thumbnails");
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

/// Decode `<dir>/<key>.png` and upload it as an egui texture (RGBA/RGB only).
fn load_thumb(ctx: &egui::Context, dir: &Path, key: &str) -> Option<egui::TextureHandle> {
    let path = dir.join(format!("{key}.png"));
    let bytes = std::fs::read(&path).ok()?;
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let src = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => src.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(src.len() / 3 * 4);
            for px in src.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        _ => return None,
    };
    let size = [info.width as usize, info.height as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
    Some(ctx.load_texture(format!("thumb_{key}"), image, egui::TextureOptions::LINEAR))
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
