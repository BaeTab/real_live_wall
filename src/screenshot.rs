//! Headless scene screenshot (`--screenshot out.png`): render a scene offscreen
//! through the full HDR post pipeline and save a PNG, then exit. No window, no
//! swapchain — it works even while the desktop is locked, which makes it the
//! visual-QA path for the built-in scenes (and a future CI hook).

use std::path::Path;

use anyhow::Context;

use crate::config::Config;
use crate::gpu::GpuContext;
use crate::postfx::PostFx;
use crate::renderer::Renderer;
use crate::uniforms::Uniforms;

pub fn run(config: &Config, out: &Path) -> anyhow::Result<()> {
    let (w, h) = (config.width.max(16), config.height.max(16));
    let (_ctx, gpu) = GpuContext::new_headless(w, h)?;

    let mut renderer = Renderer::new(&gpu)?;
    if let Some(path) = &config.shader {
        let src = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read shader {}", path.display()))?;
        renderer.load_shadertoy_glsl(&gpu, &src)?;
    }
    let postfx = PostFx::new(&gpu, config.ssaa);

    // A settled sim time and silent audio: every scene must stand on its own
    // without music, so that is exactly what the screenshot shows.
    let (sw, sh) = postfx.scene_size();
    let mut u = Uniforms::default();
    u.resolution = [sw as f32, sh as f32, 1.0, sw as f32 / sh.max(1) as f32];
    u.time = [config.sim_time, 1.0 / 60.0, config.sim_time * 60.0, 44100.0];
    u.date = [2026.0, 7.0, 3.0, 12.0 * 3600.0];
    renderer.update_uniforms(&gpu, &u);

    let final_tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shot-final"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: gpu.config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let final_view = final_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("shot-encoder") });
    renderer.draw_scene(&gpu, postfx.scene_view(), &mut encoder);
    postfx.render(&mut encoder, &final_view);

    // Read back the swapchain-format frame with 256-byte-aligned rows.
    let bpr = (w * 4).div_ceil(256) * 256;
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shot-readback"),
        size: (bpr * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &final_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bpr),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    gpu.queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
        .context("device poll failed")?;
    rx.recv()
        .context("map_async callback dropped")?
        .context("failed to map readback buffer")?;

    // Strip the row padding into a tight RGBA image.
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h {
        let start = (row * bpr) as usize;
        pixels.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    drop(data);
    readback.unmap();

    let file = std::fs::File::create(out)
        .with_context(|| format!("failed to create {}", out.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&pixels)?;

    log::info!("screenshot saved to {} ({w}x{h}, t={})", out.display(), config.sim_time);
    Ok(())
}
