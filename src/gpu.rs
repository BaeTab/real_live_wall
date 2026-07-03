//! wgpu instance / adapter / device / surface bootstrap.
//!
//! [`GpuContext`] owns the process-wide instance/adapter/device/queue. Each
//! window then gets its own [`Gpu`] (surface + swapchain config) that shares
//! that one device/queue — this is what lets a single wallpaper process drive
//! one surface per monitor without spinning up a GPU device per screen.

use std::sync::Arc;

use anyhow::Context;
use winit::window::Window;

/// Process-wide GPU state shared by every surface.
pub struct GpuContext {
    // Kept alive for the lifetime of every surface created from it.
    _instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// A single window's surface + swapchain, backed by a shared [`GpuContext`].
pub struct Gpu {
    pub surface: wgpu::Surface<'static>,
    // Cheap handle clones of the shared context (Arc under the hood).
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub size: (u32, u32),
}

impl GpuContext {
    /// Bootstrap the shared GPU state around a first window, returning both the
    /// context and that window's [`Gpu`].
    pub fn new(window: Arc<Window>) -> anyhow::Result<(Self, Gpu)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });

        // Owned Arc<Window> gives us a 'static surface (no self-referential borrow).
        let surface = instance
            .create_surface(window.clone())
            .context("failed to create wgpu surface")?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
        }))
        .context("no suitable GPU adapter found")?;

        log::info!("gpu: using adapter {:?}", adapter.get_info().name);

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rlw-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .context("failed to acquire GPU device")?;

        // Log shader/surface errors instead of panicking (keeps the wallpaper alive
        // even if a user GLSL shader fails validation).
        device.on_uncaptured_error(std::sync::Arc::new(|err| log::error!("wgpu error: {err}")));

        let ctx = Self {
            _instance: instance,
            adapter,
            device,
            queue,
        };
        let gpu = ctx.configure_surface(surface, window.inner_size())?;
        Ok((ctx, gpu))
    }

    /// Create an additional [`Gpu`] for another window, reusing this device.
    pub fn create_gpu(&self, window: Arc<Window>) -> anyhow::Result<Gpu> {
        let surface = self
            ._instance
            .create_surface(window.clone())
            .context("failed to create wgpu surface")?;
        self.configure_surface(surface, window.inner_size())
    }

    fn configure_surface(
        &self,
        surface: wgpu::Surface<'static>,
        inner: winit::dpi::PhysicalSize<u32>,
    ) -> anyhow::Result<Gpu> {
        let (width, height) = (inner.width.max(1), inner.height.max(1));

        // A fully-populated config for this surface/adapter (fills new fields such
        // as color space automatically), then tweak the bits we care about.
        let mut config = surface
            .get_default_config(&self.adapter, width, height)
            .context("surface is not supported by this adapter")?;
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        // Prefer a non-sRGB format so shader output matches Shadertoy 1:1
        // (Shadertoy writes un-gamma-corrected values to an sRGB canvas).
        let caps = surface.get_capabilities(&self.adapter);
        if let Some(f) = caps.formats.iter().copied().find(|f| !f.is_srgb()) {
            config.format = f;
        }
        surface.configure(&self.device, &config);

        Ok(Gpu {
            surface,
            device: self.device.clone(),
            queue: self.queue.clone(),
            config,
            size: (width, height),
        })
    }
}

impl Gpu {
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.size = (width, height);
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Reconfigure the surface after a Lost/Outdated acquisition.
    pub fn reconfigure(&mut self) {
        self.surface.configure(&self.device, &self.config);
    }

    /// Acquire the next swapchain image, transparently reconfiguring on
    /// outdated/lost surfaces. `None` means "skip this frame".
    pub fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                Some(t)
            }
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Validation => {
                self.reconfigure();
                None
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => None,
        }
    }
}
