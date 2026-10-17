/// wgpu bootstrap: instance, surface, device, swapchain config.
///
/// The surface format is deliberately non-sRGB when available: all Rift
/// colors are authored in sRGB (design tokens) and alpha compositing must
/// happen in sRGB space to match how the design mock composites in CSS.
pub struct Gpu {
    /// Kept for [`Gpu::for_window`]: every window draws with one device.
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    /// The compositor reads the surface as premultiplied alpha (Metal always
    /// does, whatever `alpha_mode` says; Vulkan when `PreMultiplied`).
    premultiplied: bool,
    /// Last `Suboptimal` acquire: reconfigure before the next one.
    reconfigure: bool,
}

/// Preferred surface format: 8-bit non-sRGB (the capture path decodes only
/// 8-bit BGRA/RGBA), then any non-sRGB, then whatever comes first.
fn pick_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    use wgpu::TextureFormat as F;
    [F::Bgra8Unorm, F::Rgba8Unorm]
        .into_iter()
        .find(|f| formats.contains(f))
        .or_else(|| formats.iter().copied().find(|f| !f.is_srgb()))
        .or_else(|| formats.first().copied())
}

impl Gpu {
    /// Like [`Gpu::try_new`], but a machine without a usable GPU prints one
    /// line and exits with status 1 instead of panicking.
    pub fn new(target: impl Into<wgpu::SurfaceTarget<'static>>, width: u32, height: u32) -> Gpu {
        match Gpu::try_new(target, width, height) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("rift: {e}");
                std::process::exit(1);
            }
        }
    }

    pub fn try_new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Gpu, String> {
        // no display handle: Metal (macOS) and Vulkan (Linux) don't need one
        let desc = wgpu::InstanceDescriptor::new_without_display_handle().with_env();
        let instance = wgpu::Instance::new(desc);
        let surface = instance
            .create_surface(target)
            .map_err(|e| format!("cannot create a GPU surface: {e}"))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|e| format!("no compatible GPU adapter: {e}"))?;

        log::info!("wgpu adapter = {:?}", adapter.get_info().name);

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rift device"),
            ..Default::default()
        }))
        .map_err(|e| format!("cannot open the GPU device: {e}"))?;
        Gpu::with_device(instance, adapter, device, queue, surface, width, height)
    }

    /// Another window's surface on this one's device and queue: windows
    /// share the device, so pipelines are cheap and the glyph atlas and
    /// fonts are shared.
    pub fn for_window(
        &self,
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Gpu, String> {
        let surface = self
            .instance
            .create_surface(target)
            .map_err(|e| format!("cannot create a GPU surface: {e}"))?;
        Gpu::with_device(
            self.instance.clone(),
            self.adapter.clone(),
            self.device.clone(),
            self.queue.clone(),
            surface,
            width,
            height,
        )
    }

    fn with_device(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Gpu, String> {
        let caps = surface.get_capabilities(&adapter);
        let format = pick_format(&caps.formats)
            .ok_or_else(|| "the GPU surface reports no formats".to_string())?;

        // translucent window grounds (config `window_opacity`) need the
        // compositor to honor the surface alpha. Metal treats PostMultiplied
        // as premultiplied anyway (see `premultiplied`).
        let alpha_mode = [
            wgpu::CompositeAlphaMode::PostMultiplied,
            wgpu::CompositeAlphaMode::PreMultiplied,
        ]
        .into_iter()
        .find(|m| caps.alpha_modes.contains(m))
        .unwrap_or(wgpu::CompositeAlphaMode::Auto);
        let premultiplied =
            cfg!(target_os = "macos") || alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied;
        let max_dim = device.limits().max_texture_dimension_2d;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.clamp(1, max_dim),
            height: height.clamp(1, max_dim),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        Ok(Gpu {
            instance,
            adapter,
            surface,
            device,
            queue,
            config,
            premultiplied,
            reconfigure: false,
        })
    }

    /// Theme color -> clear color for the window surface. A translucent
    /// ground must be premultiplied when the compositor reads the surface as
    /// premultiplied alpha, else it renders brighter than `window_opacity`.
    pub fn clear_color(&self, c: rift_core::Color) -> wgpu::Color {
        let mut w = crate::color::to_wgpu(c);
        if self.premultiplied {
            w.r *= w.a;
            w.g *= w.a;
            w.b *= w.a;
        }
        w
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        // configure() over the device's texture limit is a fatal error
        let max_dim = self.device.limits().max_texture_dimension_2d;
        self.config.width = width.min(max_dim);
        self.config.height = height.min(max_dim);
        self.surface.configure(&self.device, &self.config);
        self.reconfigure = false;
    }

    /// Acquire the next frame, reconfiguring on Lost/Outdated/Suboptimal.
    pub fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        use wgpu::CurrentSurfaceTexture as Cst;
        if self.reconfigure {
            self.reconfigure = false;
            self.surface.configure(&self.device, &self.config);
        }
        match self.surface.get_current_texture() {
            Cst::Success(t) => Some(t),
            Cst::Suboptimal(t) => {
                // usable this frame; reconfigure before the next acquire
                self.reconfigure = true;
                Some(t)
            }
            Cst::Outdated | Cst::Lost => {
                self.surface.configure(&self.device, &self.config);
                match self.surface.get_current_texture() {
                    Cst::Success(t) | Cst::Suboptimal(t) => Some(t),
                    _ => None,
                }
            }
            Cst::Timeout | Cst::Occluded | Cst::Validation => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pick_format;
    use wgpu::TextureFormat as F;

    #[test]
    fn prefers_8bit_non_srgb() {
        assert_eq!(
            pick_format(&[
                F::Bgra8UnormSrgb,
                F::Rgba16Float,
                F::Rgba8Unorm,
                F::Bgra8Unorm
            ]),
            Some(F::Bgra8Unorm)
        );
        assert_eq!(
            pick_format(&[F::Rgba8UnormSrgb, F::Rgba16Float]),
            Some(F::Rgba16Float)
        );
        assert_eq!(pick_format(&[F::Bgra8UnormSrgb]), Some(F::Bgra8UnormSrgb));
        assert_eq!(pick_format(&[]), None);
    }
}
