//! Textured background image pipeline (Warp-style theme `background_image`).
//!
//! Draws one full-viewport triangle sampling a decoded image with "cover" fit:
//! the image is scaled uniformly until it fills the viewport, centered, and the
//! overflow is cropped. The crop is computed on the CPU into a uv
//! offset + scale so the shader stays a single `textureSample`.
//!
//! The texture format is `Rgba8Unorm`, not `Rgba8UnormSrgb`: Rift composites
//! in sRGB space on a non-sRGB surface (see `gpu.rs`), so image bytes are
//! written verbatim and alpha-blended the same way every quad is.

use bytemuck::{Pod, Zeroable};
use std::path::Path;

/// Largest background image texture we upload (RGBA bytes).
const MAX_BG_BYTES: u64 = 256 * 1024 * 1024;

/// Reject images the GPU cannot hold: empty, wider/taller than the device's
/// 2D texture limit, or more than `MAX_BG_BYTES` of RGBA.
fn check_size(width: u32, height: u32, limit: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("empty image".into());
    }
    if width > limit || height > limit {
        return Err(format!(
            "{width}x{height} exceeds the GPU texture limit of {limit}px"
        ));
    }
    if u64::from(width) * u64::from(height) * 4 > MAX_BG_BYTES {
        return Err(format!("{width}x{height} is too large (max 256 MiB RGBA)"));
    }
    Ok(())
}

/// Per-frame shader uniforms. 32 bytes, 16-byte aligned.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    uv_offset: [f32; 2],
    uv_scale: [f32; 2],
    opacity: f32,
    _pad: [f32; 3],
}

/// "Cover" fit: the uv sub-rectangle of `image` that fills `viewport` without
/// distortion, centered. Returns `(offset, scale)` in uv space, so a fragment
/// at viewport uv `t` (0..1) samples `offset + t * scale`.
///
/// Exactly one axis is cropped; the other spans the full 0..1 range.
pub fn cover_uv(viewport: [f32; 2], image: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    // guard degenerate sizes rather than emitting NaN uvs
    if !(viewport[0] > 0.0 && viewport[1] > 0.0 && image[0] > 0.0 && image[1] > 0.0) {
        return ([0.0, 0.0], [1.0, 1.0]);
    }
    let viewport_aspect = viewport[0] / viewport[1];
    let image_aspect = image[0] / image[1];

    // crop the axis on which the image is relatively longer
    let scale = [
        (viewport_aspect / image_aspect).min(1.0),
        (image_aspect / viewport_aspect).min(1.0),
    ];
    let offset = [(1.0 - scale[0]) * 0.5, (1.0 - scale[1]) * 0.5];
    (offset, scale)
}

struct Loaded {
    bind_group: wgpu::BindGroup,
    size: [f32; 2],
}

pub struct ImagePipeline {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buf: wgpu::Buffer,
    loaded: Option<Loaded>,
}

impl ImagePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("image.wgsl"));

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bg image uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bg image bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bg image layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bg image pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // premultiplied textures (no filter fringes); still an sRGB-space composite
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bg image sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        ImagePipeline {
            pipeline,
            bgl,
            sampler,
            uniform_buf,
            loaded: None,
        }
    }

    /// Decode `path` and upload it as the background texture, replacing any
    /// previous image. Returns `Err` with a human-readable reason on failure;
    /// the previous image is dropped either way.
    pub fn load(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &Path,
    ) -> Result<(), String> {
        self.loaded = None;

        // check the header size before decoding: wgpu's default uncaptured
        // error handler panics on an oversized texture
        let limit = device.limits().max_texture_dimension_2d;
        let (hw, hh) = ::image::image_dimensions(path)
            .map_err(|e| format!("background_image {}: {e}", path.display()))?;
        check_size(hw, hh, limit)
            .map_err(|e| format!("background_image {}: {e}", path.display()))?;

        let mut decoded = ::image::open(path)
            .map_err(|e| format!("background_image {}: {e}", path.display()))?
            .to_rgba8();
        let (width, height) = decoded.dimensions();
        check_size(width, height, limit)
            .map_err(|e| format!("background_image {}: {e}", path.display()))?;
        crate::color::premultiply_rgba8(&mut decoded);

        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bg image"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // NOT sRGB: bytes are sRGB values composited in sRGB space
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &decoded,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            extent,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg image bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        self.loaded = Some(Loaded {
            bind_group,
            size: [width as f32, height as f32],
        });
        Ok(())
    }

    /// Drop the current image; `render` becomes a no-op.
    pub fn clear(&mut self) {
        self.loaded = None;
    }

    pub fn has_image(&self) -> bool {
        self.loaded.is_some()
    }

    /// Upload this frame's uniforms. `viewport` is the physical surface size in
    /// px; `opacity` is clamped to 0..1. No-op without an image.
    pub fn prepare(&mut self, queue: &wgpu::Queue, viewport: [f32; 2], opacity: f32) {
        let Some(loaded) = &self.loaded else {
            return;
        };
        let (uv_offset, uv_scale) = cover_uv(viewport, loaded.size);
        queue.write_buffer(
            &self.uniform_buf,
            0,
            bytemuck::bytes_of(&Uniforms {
                uv_offset,
                uv_scale,
                opacity: opacity.clamp(0.0, 1.0),
                _pad: [0.0; 3],
            }),
        );
    }

    /// Draw the full-viewport image. No-op without an image.
    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(loaded) = &self.loaded else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &loaded.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_size_rejects_oversized_and_empty() {
        assert!(check_size(1920, 1080, 16384).is_ok());
        assert!(check_size(0, 10, 16384).is_err());
        assert!(check_size(20000, 10, 16384).is_err());
        assert!(check_size(10, 20000, 16384).is_err());
        // within the dimension limit but over the byte cap
        assert!(check_size(16384, 16384, 16384).is_err());
    }

    /// uv span of the sampled rectangle on each axis.
    fn span(offset: [f32; 2], scale: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        (
            [offset[0], offset[0] + scale[0]],
            [offset[1], offset[1] + scale[1]],
        )
    }

    #[test]
    fn cover_wide_image_on_tall_viewport_crops_horizontally() {
        // 400x100 image (aspect 4) in a 100x200 viewport (aspect 0.5)
        let (offset, scale) = cover_uv([100.0, 200.0], [400.0, 100.0]);

        // v spans the whole image, u is cropped to viewport/image aspect
        assert!((scale[1] - 1.0).abs() < 1e-6, "v span {}", scale[1]);
        assert!((offset[1]).abs() < 1e-6, "v offset {}", offset[1]);
        assert!((scale[0] - 0.5 / 4.0).abs() < 1e-6, "u span {}", scale[0]);

        // centered: equal crop either side
        let (u, _) = span(offset, scale);
        assert!((offset[0] - (1.0 - scale[0]) / 2.0).abs() < 1e-6);
        assert!(((u[0] + u[1]) / 2.0 - 0.5).abs() < 1e-6, "u center {u:?}");

        // the visible region has exactly the viewport aspect
        let visible_aspect = (scale[0] * 400.0) / (scale[1] * 100.0);
        assert!((visible_aspect - 0.5).abs() < 1e-5, "{visible_aspect}");
    }

    #[test]
    fn cover_tall_image_on_wide_viewport_crops_vertically() {
        // 100x400 image (aspect 0.25) in a 800x200 viewport (aspect 4)
        let (offset, scale) = cover_uv([800.0, 200.0], [100.0, 400.0]);

        assert!((scale[0] - 1.0).abs() < 1e-6, "u span {}", scale[0]);
        assert!((offset[0]).abs() < 1e-6, "u offset {}", offset[0]);
        assert!((scale[1] - 0.25 / 4.0).abs() < 1e-6, "v span {}", scale[1]);

        let (_, v) = span(offset, scale);
        assert!((offset[1] - (1.0 - scale[1]) / 2.0).abs() < 1e-6);
        assert!(((v[0] + v[1]) / 2.0 - 0.5).abs() < 1e-6, "v center {v:?}");

        let visible_aspect = (scale[0] * 100.0) / (scale[1] * 400.0);
        assert!((visible_aspect - 4.0).abs() < 1e-4, "{visible_aspect}");
    }

    #[test]
    fn cover_matching_aspect_uses_whole_image() {
        let (offset, scale) = cover_uv([1600.0, 900.0], [800.0, 450.0]);
        assert_eq!(offset, [0.0, 0.0]);
        assert!((scale[0] - 1.0).abs() < 1e-6 && (scale[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cover_degenerate_sizes_do_not_produce_nan() {
        for (vp, img) in [
            ([0.0, 100.0], [10.0, 10.0]),
            ([100.0, 0.0], [10.0, 10.0]),
            ([100.0, 100.0], [0.0, 10.0]),
            ([100.0, 100.0], [10.0, 0.0]),
        ] {
            let (offset, scale) = cover_uv(vp, img);
            assert_eq!((offset, scale), ([0.0, 0.0], [1.0, 1.0]));
        }
    }

    #[test]
    fn png_round_trips_through_the_image_crate() {
        // no GPU needed: this pins that the decode path `load` uses works for
        // the formats the theme accepts
        let dir = std::env::temp_dir().join(format!(
            "rift-image-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("bg.png");

        let written = ::image::RgbaImage::from_fn(4, 3, |x, y| {
            ::image::Rgba([(x * 60) as u8, (y * 80) as u8, 12, 255])
        });
        written.save(&path).expect("write png");

        let read = ::image::open(&path).expect("decode png").to_rgba8();
        assert_eq!(read.dimensions(), (4, 3));
        assert_eq!(read.as_raw(), written.as_raw());

        // a non-image file is an Err, not a panic
        let bad = dir.join("bad.png");
        std::fs::write(&bad, b"not a png").expect("write bad file");
        assert!(::image::open(&bad).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }
}
