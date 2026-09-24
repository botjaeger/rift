//! Inline terminal images (kitty graphics / iTerm2) as textured quads.
//!
//! Each decoded image is uploaded once into its own texture, keyed by the
//! image's process-unique key, and reused across frames. Textures unused for
//! `IDLE_FRAMES` frames are dropped, and the cache never holds more than
//! [`MAX_GPU_BYTES`]: when a new upload would exceed it, the least recently
//! drawn textures go first (a sprite that still cannot fit is skipped).

use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::sync::Arc;

pub const MAX_GPU_BYTES: usize = 256 << 20;
const IDLE_FRAMES: u64 = 600;

/// One image drawn this frame (logical px).
#[derive(Clone)]
pub struct Sprite {
    pub key: u64,
    pub rgba: Arc<Vec<u8>>,
    pub size: [u32; 2],
    /// Destination rect: x, y, w, h.
    pub rect: [f32; 4],
    /// Source rect in image pixels: x, y, w, h.
    pub src: [f32; 4],
    /// Visible region (pane content area): x, y, w, h.
    pub clip: [f32; 4],
    /// Drawn before the layer's text (kitty z < 0).
    pub under_text: bool,
}

/// Clip `rect` to `clip`, shrinking the uv range in proportion. None when
/// nothing is visible.
pub fn clip_uv(rect: [f32; 4], uv: [f32; 4], clip: [f32; 4]) -> Option<([f32; 4], [f32; 4])> {
    let x0 = rect[0].max(clip[0]);
    let y0 = rect[1].max(clip[1]);
    let x1 = (rect[0] + rect[2]).min(clip[0] + clip[2]);
    let y1 = (rect[1] + rect[3]).min(clip[1] + clip[3]);
    if x1 <= x0 || y1 <= y0 || rect[2] <= 0.0 || rect[3] <= 0.0 {
        return None;
    }
    let fx = |x: f32| uv[0] + (uv[2] - uv[0]) * (x - rect[0]) / rect[2];
    let fy = |y: f32| uv[1] + (uv[3] - uv[1]) * (y - rect[1]) / rect[3];
    Some(([x0, y0, x1 - x0, y1 - y0], [fx(x0), fy(y0), fx(x1), fy(y1)]))
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Inst {
    rect: [f32; 4],
    uv: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Viewport {
    size: [f32; 2],
    _pad: [f32; 2],
}

struct Tex {
    bind_group: wgpu::BindGroup,
    bytes: usize,
    last_frame: u64,
}

/// Per layer: (under-text draws, over-text draws), each (instance, key).
type LayerDraws = (Vec<(u32, u64)>, Vec<(u32, u64)>);

pub struct SpritePipeline {
    pipeline: wgpu::RenderPipeline,
    tex_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    vp_buf: wgpu::Buffer,
    vp_group: wgpu::BindGroup,
    inst_buf: wgpu::Buffer,
    capacity: usize,
    textures: HashMap<u64, Tex>,
    gpu_bytes: usize,
    frame: u64,
    layers: Vec<LayerDraws>,
}

impl SpritePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("sprite.wgsl"));
        let vp_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite vp bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite tex bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let vp_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite viewport"),
            size: std::mem::size_of::<Viewport>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vp_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprite vp"),
            layout: &vp_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: vp_buf.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite layout"),
            bind_group_layouts: &[Some(&vp_bgl), Some(&tex_bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Inst>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            label: Some("sprite sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let capacity = 16;
        SpritePipeline {
            pipeline,
            tex_bgl,
            sampler,
            vp_buf,
            vp_group,
            inst_buf: Self::make_buf(device, capacity),
            capacity,
            textures: HashMap::new(),
            gpu_bytes: 0,
            frame: 0,
            layers: Vec::new(),
        }
    }

    fn make_buf(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite instances"),
            size: (capacity * std::mem::size_of::<Inst>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Bytes of image textures currently resident on the GPU.
    pub fn gpu_bytes(&self) -> usize {
        self.gpu_bytes
    }

    fn evict_until(&mut self, need: usize, keep: &[u64]) {
        while self.gpu_bytes + need > MAX_GPU_BYTES {
            let victim = self
                .textures
                .iter()
                .filter(|(k, _)| !keep.contains(k))
                .min_by_key(|(_, t)| t.last_frame)
                .map(|(k, _)| *k);
            let Some(k) = victim else { break };
            if let Some(t) = self.textures.remove(&k) {
                self.gpu_bytes -= t.bytes;
            }
        }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, s: &Sprite) -> bool {
        let bytes = s.rgba.len();
        let expected = s.size[0] as usize * s.size[1] as usize * 4;
        if bytes != expected || bytes == 0 || bytes > MAX_GPU_BYTES {
            return false;
        }
        let limit = device.limits().max_texture_dimension_2d;
        if s.size[0] > limit || s.size[1] > limit {
            return false;
        }
        self.evict_until(bytes, &[]);
        if self.gpu_bytes + bytes > MAX_GPU_BYTES {
            return false;
        }
        let extent = wgpu::Extent3d {
            width: s.size[0],
            height: s.size[1],
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("inline image"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
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
            &s.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(s.size[0] * 4),
                rows_per_image: Some(s.size[1]),
            },
            extent,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("inline image bg"),
            layout: &self.tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.gpu_bytes += bytes;
        self.textures.insert(
            s.key,
            Tex {
                bind_group,
                bytes,
                last_frame: self.frame,
            },
        );
        true
    }

    /// Upload this frame's sprites (`layers[i]` = sprites of scene layer i).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: [f32; 2],
        layers: &[&[Sprite]],
    ) {
        self.frame += 1;
        queue.write_buffer(
            &self.vp_buf,
            0,
            bytemuck::bytes_of(&Viewport {
                size: screen,
                _pad: [0.0; 2],
            }),
        );
        let mut insts: Vec<Inst> = Vec::new();
        self.layers.clear();
        for sprites in layers {
            let mut draws: LayerDraws = (Vec::new(), Vec::new());
            for s in *sprites {
                let uv = [
                    s.src[0] / s.size[0].max(1) as f32,
                    s.src[1] / s.size[1].max(1) as f32,
                    (s.src[0] + s.src[2]) / s.size[0].max(1) as f32,
                    (s.src[1] + s.src[3]) / s.size[1].max(1) as f32,
                ];
                let Some((rect, uv)) = clip_uv(s.rect, uv, s.clip) else {
                    continue;
                };
                if !self.textures.contains_key(&s.key) && !self.upload(device, queue, s) {
                    continue;
                }
                if let Some(t) = self.textures.get_mut(&s.key) {
                    t.last_frame = self.frame;
                }
                let i = insts.len() as u32;
                insts.push(Inst { rect, uv });
                if s.under_text {
                    draws.0.push((i, s.key));
                } else {
                    draws.1.push((i, s.key));
                }
            }
            self.layers.push(draws);
        }
        // idle textures go
        let frame = self.frame;
        let mut freed = 0;
        self.textures.retain(|_, t| {
            let keep = frame - t.last_frame < IDLE_FRAMES;
            if !keep {
                freed += t.bytes;
            }
            keep
        });
        self.gpu_bytes -= freed;
        if insts.len() > self.capacity {
            self.capacity = insts.len().next_power_of_two();
            self.inst_buf = Self::make_buf(device, self.capacity);
        }
        if !insts.is_empty() {
            queue.write_buffer(&self.inst_buf, 0, bytemuck::cast_slice(&insts));
        }
    }

    /// Draw layer `i`'s sprites that go under (`under = true`) or over text.
    pub fn render_layer(&self, i: usize, under: bool, pass: &mut wgpu::RenderPass<'_>) {
        let Some(draws) = self.layers.get(i) else {
            return;
        };
        let list = if under { &draws.0 } else { &draws.1 };
        if list.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.vp_group, &[]);
        pass.set_vertex_buffer(0, self.inst_buf.slice(..));
        for (inst, key) in list {
            if let Some(t) = self.textures.get(key) {
                pass.set_bind_group(1, &t.bind_group, &[]);
                pass.draw(0..6, *inst..*inst + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::clip_uv;

    #[test]
    fn clipping_shrinks_uvs_proportionally() {
        // 100x100 rect, clip away the top half and left quarter
        let (r, uv) = clip_uv(
            [0.0, 0.0, 100.0, 100.0],
            [0.0, 0.0, 1.0, 1.0],
            [25.0, 50.0, 1000.0, 1000.0],
        )
        .unwrap();
        assert_eq!(r, [25.0, 50.0, 75.0, 50.0]);
        assert_eq!(uv, [0.25, 0.5, 1.0, 1.0]);
        // fully outside
        assert!(
            clip_uv(
                [0.0, 0.0, 10.0, 10.0],
                [0.0, 0.0, 1.0, 1.0],
                [20.0, 0.0, 5.0, 5.0]
            )
            .is_none()
        );
    }
}
