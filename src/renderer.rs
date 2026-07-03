//! The full-screen shader renderer: owns the uniform buffer, bind group and the
//! active render pipeline, and can hot-swap the fragment shader (WGSL default or
//! a Shadertoy GLSL scene) at runtime.

use crate::gpu::Gpu;
use crate::shader;
use crate::uniforms::Uniforms;

/// What happened when we tried to draw a frame.
pub enum RenderOutcome {
    Presented,
    NeedsReconfigure,
    Skipped,
}

pub struct Renderer {
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    #[allow(dead_code)]
    bind_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    vs_module: wgpu::ShaderModule,
    pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
}

impl Renderer {
    pub fn new(gpu: &Gpu) -> anyhow::Result<Self> {
        let device = &gpu.device;

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rlw-uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rlw-bind-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rlw-bind-group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rlw-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });

        let vs_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rlw-fullscreen-vs"),
            source: wgpu::ShaderSource::Wgsl(shader::FULLSCREEN_VS.into()),
        });

        let fs_module = wgsl_module(device, shader::DEFAULT_WGSL_FS);
        let pipeline =
            build_pipeline(device, &pipeline_layout, &vs_module, &fs_module, gpu.config.format);

        Ok(Self {
            uniform_buf,
            bind_group,
            bind_layout,
            pipeline_layout,
            vs_module,
            pipeline,
            format: gpu.config.format,
        })
    }

    /// Swap to a Shadertoy GLSL scene.
    pub fn load_shadertoy_glsl(&mut self, gpu: &Gpu, user_source: &str) -> anyhow::Result<()> {
        let wrapped = shader::wrap_shadertoy_glsl(user_source);
        let fs_module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rlw-shadertoy-fs"),
            source: wgpu::ShaderSource::Glsl {
                shader: wrapped.into(),
                stage: wgpu::naga::ShaderStage::Fragment,
                defines: &[],
            },
        });
        self.pipeline = build_pipeline(
            &gpu.device,
            &self.pipeline_layout,
            &self.vs_module,
            &fs_module,
            self.format,
        );
        Ok(())
    }

    /// Reset to the built-in WGSL scene.
    #[allow(dead_code)]
    pub fn load_default(&mut self, gpu: &Gpu) {
        let fs_module = wgsl_module(&gpu.device, shader::DEFAULT_WGSL_FS);
        self.pipeline = build_pipeline(
            &gpu.device,
            &self.pipeline_layout,
            &self.vs_module,
            &fs_module,
            self.format,
        );
    }

    pub fn update_uniforms(&self, gpu: &Gpu, uniforms: &Uniforms) {
        gpu.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(uniforms));
    }

    /// Hook for future bind-group invalidation on resize (currently a no-op).
    pub fn rebind(&mut self, _gpu: &Gpu) {}

    pub fn render(&self, gpu: &Gpu) -> RenderOutcome {
        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Validation => return RenderOutcome::NeedsReconfigure,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return RenderOutcome::Skipped
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rlw-encoder"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rlw-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, Some(&self.bind_group), &[]);
            pass.draw(0..3, 0..1);
        }

        gpu.queue.submit(std::iter::once(encoder.finish()));
        // wgpu 30: presentation moved from `SurfaceTexture::present` to the queue.
        gpu.queue.present(frame);
        RenderOutcome::Presented
    }
}

fn wgsl_module(device: &wgpu::Device, src: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("rlw-wgsl-fs"),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    })
}

fn build_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    vs: &wgpu::ShaderModule,
    fs: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("rlw-pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: vs,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: fs,
            // WGSL uses `fs_main`; the wrapped GLSL uses `main`. Both modules have
            // a single fragment entry point, so `None` selects it either way.
            entry_point: None,
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
