//! HDR post-processing: render the scene to a super-sampled float target, then
//! bright-pass → ping-pong Gaussian bloom → ACES tonemap + vignette composite to
//! the swapchain. This is the main "premium look" lever (glow + smooth AA).

use crate::gpu::Gpu;

/// The HDR format the scene and bloom targets render into.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniforms {
    /// xy = bloom texel (1/half-res), z = threshold, w = knee.
    params0: [f32; 4],
    /// x = bloom intensity, y = exposure, z = vignette, w = saturation.
    params1: [f32; 4],
}

pub struct PostFx {
    ssaa: f32,
    scene: TexView,
    bloom0: TexView,
    bloom1: TexView,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,

    layout_single: wgpu::BindGroupLayout,
    layout_dual: wgpu::BindGroupLayout,

    bright: wgpu::RenderPipeline,
    blur_h: wgpu::RenderPipeline,
    blur_v: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,

    bg_bright: wgpu::BindGroup,
    bg_blur_h: wgpu::BindGroup,
    bg_blur_v: wgpu::BindGroup,
    bg_composite: wgpu::BindGroup,

    scene_size: (u32, u32),
}

struct TexView {
    #[allow(dead_code)]
    tex: wgpu::Texture,
    view: wgpu::TextureView,
}

fn make_target(device: &wgpu::Device, label: &str, w: u32, h: u32) -> TexView {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HDR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    TexView { tex, view }
}

impl PostFx {
    pub fn new(gpu: &Gpu, ssaa: f32) -> Self {
        let device = &gpu.device;

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("postfx-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("postfx-uniform"),
            size: std::mem::size_of::<PostUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout_single = device.create_bind_group_layout(&single_layout_desc());
        let layout_dual = device.create_bind_group_layout(&dual_layout_desc());

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("postfx-shader"),
            source: wgpu::ShaderSource::Wgsl(POST_WGSL.into()),
        });

        let pl_single = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("postfx-pl-single"),
            bind_group_layouts: &[Some(&layout_single)],
            immediate_size: 0,
        });
        let pl_dual = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("postfx-pl-dual"),
            bind_group_layouts: &[Some(&layout_dual)],
            immediate_size: 0,
        });

        let bright = make_pipeline(device, &pl_single, &module, "fs_bright", HDR_FORMAT);
        let blur_h = make_pipeline(device, &pl_single, &module, "fs_blur_h", HDR_FORMAT);
        let blur_v = make_pipeline(device, &pl_single, &module, "fs_blur_v", HDR_FORMAT);
        let composite = make_pipeline(device, &pl_dual, &module, "fs_composite", gpu.config.format);

        let (w, h) = gpu.size;
        let scene_size = scaled(w, h, ssaa);
        let (bw, bh) = (w / 2, h / 2);
        let scene = make_target(device, "postfx-scene", scene_size.0, scene_size.1);
        let bloom0 = make_target(device, "postfx-bloom0", bw, bh);
        let bloom1 = make_target(device, "postfx-bloom1", bw, bh);

        let bg_bright = single_bg(device, &layout_single, &uniform, &scene.view, &sampler);
        let bg_blur_h = single_bg(device, &layout_single, &uniform, &bloom0.view, &sampler);
        let bg_blur_v = single_bg(device, &layout_single, &uniform, &bloom1.view, &sampler);
        let bg_composite =
            dual_bg(device, &layout_dual, &uniform, &scene.view, &sampler, &bloom0.view);

        let me = Self {
            ssaa,
            scene,
            bloom0,
            bloom1,
            sampler,
            uniform,
            layout_single,
            layout_dual,
            bright,
            blur_h,
            blur_v,
            composite,
            bg_bright,
            bg_blur_h,
            bg_blur_v,
            bg_composite,
            scene_size,
        };
        me.write_uniform(gpu);
        me
    }

    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene.view
    }
    pub fn scene_size(&self) -> (u32, u32) {
        self.scene_size
    }

    pub fn resize(&mut self, gpu: &Gpu, w: u32, h: u32) {
        let device = &gpu.device;
        self.scene_size = scaled(w, h, self.ssaa);
        let (bw, bh) = (w / 2, h / 2);
        self.scene = make_target(device, "postfx-scene", self.scene_size.0, self.scene_size.1);
        self.bloom0 = make_target(device, "postfx-bloom0", bw, bh);
        self.bloom1 = make_target(device, "postfx-bloom1", bw, bh);

        self.bg_bright =
            single_bg(device, &self.layout_single, &self.uniform, &self.scene.view, &self.sampler);
        self.bg_blur_h =
            single_bg(device, &self.layout_single, &self.uniform, &self.bloom0.view, &self.sampler);
        self.bg_blur_v =
            single_bg(device, &self.layout_single, &self.uniform, &self.bloom1.view, &self.sampler);
        self.bg_composite = dual_bg(
            device,
            &self.layout_dual,
            &self.uniform,
            &self.scene.view,
            &self.sampler,
            &self.bloom0.view,
        );
        self.write_uniform(gpu);
    }

    fn write_uniform(&self, gpu: &Gpu) {
        let (w, h) = gpu.size;
        let texel = [2.0 / w.max(1) as f32, 2.0 / h.max(1) as f32];
        let u = PostUniforms {
            // threshold 0.75, soft knee 0.35
            params0: [texel[0], texel[1], 0.75, 0.35],
            // bloom intensity, exposure, vignette, saturation
            params1: [0.85, 1.05, 0.32, 1.12],
        };
        gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
    }

    /// Run bright-pass → 2× ping-pong blur → composite into `final_view`.
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, final_view: &wgpu::TextureView) {
        // bright-pass: scene → bloom0
        pass(encoder, &self.bloom0.view, &self.bright, &self.bg_bright);
        // two blur iterations (H,V) ping-ponging bloom0<->bloom1
        pass(encoder, &self.bloom1.view, &self.blur_h, &self.bg_blur_h);
        pass(encoder, &self.bloom0.view, &self.blur_v, &self.bg_blur_v);
        pass(encoder, &self.bloom1.view, &self.blur_h, &self.bg_blur_h);
        pass(encoder, &self.bloom0.view, &self.blur_v, &self.bg_blur_v);
        // composite scene + bloom0 → swapchain
        pass(encoder, final_view, &self.composite, &self.bg_composite);
    }
}

fn scaled(w: u32, h: u32, s: f32) -> (u32, u32) {
    (((w as f32 * s) as u32).max(1), ((h as f32 * s) as u32).max(1))
}

fn pass(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
) {
    let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("postfx-pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.set_pipeline(pipeline);
    rp.set_bind_group(0, Some(bind), &[]);
    rp.draw(0..3, 0..1);
}

fn make_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    fs: &str,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("postfx-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_post"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn single_layout_desc() -> wgpu::BindGroupLayoutDescriptor<'static> {
    wgpu::BindGroupLayoutDescriptor {
        label: Some("postfx-single"),
        entries: SINGLE_ENTRIES,
    }
}
fn dual_layout_desc() -> wgpu::BindGroupLayoutDescriptor<'static> {
    wgpu::BindGroupLayoutDescriptor {
        label: Some("postfx-dual"),
        entries: DUAL_ENTRIES,
    }
}

const fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
const fn uniform_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
const fn sampler_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}
const SINGLE_ENTRIES: &[wgpu::BindGroupLayoutEntry] =
    &[uniform_entry(), tex_entry(1), sampler_entry()];
const DUAL_ENTRIES: &[wgpu::BindGroupLayoutEntry] =
    &[uniform_entry(), tex_entry(1), sampler_entry(), tex_entry(3)];

fn single_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    tex: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("postfx-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(tex) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}
fn dual_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    tex0: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    tex1: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("postfx-bg-dual"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(tex0) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(tex1) },
        ],
    })
}

const POST_WGSL: &str = r#"
struct Post {
    params0: vec4<f32>,   // texel.xy, threshold, knee
    params1: vec4<f32>,   // bloom, exposure, vignette, saturation
};
@group(0) @binding(0) var<uniform> u: Post;
@group(0) @binding(1) var tex0: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_post(@builtin(vertex_index) vid: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let xy = p[vid];
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>((xy.x + 1.0) * 0.5, 1.0 - (xy.y + 1.0) * 0.5);
    return out;
}

// bright-pass with soft knee
@fragment
fn fs_bright(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex0, samp, in.uv).rgb;
    let br = max(c.r, max(c.g, c.b));
    let threshold = u.params0.z;
    let knee = u.params0.w + 1e-4;
    let soft = clamp(br - threshold + knee, 0.0, 2.0 * knee);
    let softc = soft * soft / (4.0 * knee);
    let contrib = max(softc, br - threshold) / max(br, 1e-4);
    return vec4<f32>(c * max(contrib, 0.0), 1.0);
}

fn blur(uv: vec2<f32>, dir: vec2<f32>) -> vec3<f32> {
    // 5-tap gaussian using linear-sampling offsets
    var col = textureSample(tex0, samp, uv).rgb * 0.227027;
    let o1 = dir * 1.3846153846;
    let o2 = dir * 3.2307692308;
    col += textureSample(tex0, samp, uv + o1).rgb * 0.3162162162;
    col += textureSample(tex0, samp, uv - o1).rgb * 0.3162162162;
    col += textureSample(tex0, samp, uv + o2).rgb * 0.0702702703;
    col += textureSample(tex0, samp, uv - o2).rgb * 0.0702702703;
    return col;
}

@fragment
fn fs_blur_h(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur(in.uv, vec2<f32>(u.params0.x, 0.0)), 1.0);
}
@fragment
fn fs_blur_v(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur(in.uv, vec2<f32>(0.0, u.params0.y)), 1.0);
}

@group(0) @binding(3) var tex1: texture_2d<f32>;

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51; let b = 0.03; let c = 2.43; let d = 0.59; let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4<f32> {
    let scene = textureSample(tex0, samp, in.uv).rgb;
    let bloom = textureSample(tex1, samp, in.uv).rgb;
    var col = scene + bloom * u.params1.x;
    col = col * u.params1.y;                 // exposure
    col = aces(col);                          // tonemap
    // saturation
    let luma = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
    col = mix(vec3<f32>(luma), col, u.params1.w);
    // vignette
    let d = distance(in.uv, vec2<f32>(0.5));
    let vig = 1.0 - u.params1.z * smoothstep(0.35, 0.85, d);
    col = col * vig;
    return vec4<f32>(col, 1.0);
}
"#;
