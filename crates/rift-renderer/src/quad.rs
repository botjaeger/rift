use bytemuck::{Pod, Zeroable};

/// One SDF quad instance. Positions/sizes in logical px.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Quad {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub color: [f32; 4],
    pub border_color: [f32; 4],
    pub radius: f32,
    pub border_width: f32,
    pub softness: f32,
    /// 0 = rounded box, 1 = line segment (`line` holds its end points),
    /// 2 = rounded box filled with a two-stop gradient from `color` to
    /// `border_color` along the axis `line.xy`.
    pub kind: f32,
    /// Segment end points `[ax, ay, bx, by]` (kind 1).
    pub line: [f32; 4],
    /// Rounded clip `[x, y, w, h]`; `w <= 0` = unclipped (CSS
    /// `overflow: hidden` on a rounded parent).
    pub clip: [f32; 4],
    pub clip_radius: f32,
    pub _pad: [f32; 3],
}

impl Quad {
    pub fn rect(pos: [f32; 2], size: [f32; 2], color: [f32; 4], radius: f32) -> Self {
        Quad {
            pos,
            size,
            color,
            radius,
            ..Zeroable::zeroed()
        }
    }

    pub fn bordered(
        pos: [f32; 2],
        size: [f32; 2],
        fill: [f32; 4],
        border: [f32; 4],
        radius: f32,
        border_width: f32,
    ) -> Self {
        Quad {
            border_color: border,
            border_width,
            ..Quad::rect(pos, size, fill, radius)
        }
    }

    pub fn circle(center: [f32; 2], diameter: f32, color: [f32; 4]) -> Self {
        Quad::rect(
            [center[0] - diameter * 0.5, center[1] - diameter * 0.5],
            [diameter, diameter],
            color,
            diameter, // clamped to half-size in shader → circle
        )
    }

    /// Soft shadow / glow behind other quads.
    pub fn shadow(pos: [f32; 2], size: [f32; 2], color: [f32; 4], radius: f32, sigma: f32) -> Self {
        Quad {
            softness: sigma,
            ..Quad::rect(pos, size, color, radius)
        }
    }

    /// A rounded box filled with a linear gradient from `from` (top or
    /// left edge) to `to` (bottom or right edge).
    pub fn gradient(
        pos: [f32; 2],
        size: [f32; 2],
        from: [f32; 4],
        to: [f32; 4],
        vertical: bool,
        radius: f32,
    ) -> Self {
        Quad {
            border_color: to,
            kind: 2.0,
            line: if vertical {
                [0.0, 1.0, 0.0, 0.0]
            } else {
                [1.0, 0.0, 0.0, 0.0]
            },
            ..Quad::rect(pos, size, from, radius)
        }
    }

    /// Anti-aliased line from `a` to `b`, `width` px thick with round caps
    /// (icon strokes: ×, chevrons, checkmarks).
    pub fn line(a: [f32; 2], b: [f32; 2], width: f32, color: [f32; 4]) -> Self {
        let pad = width * 0.5 + 1.0;
        let (x0, y0) = (a[0].min(b[0]) - pad, a[1].min(b[1]) - pad);
        let (x1, y1) = (a[0].max(b[0]) + pad, a[1].max(b[1]) + pad);
        Quad {
            border_width: width,
            kind: 1.0,
            line: [a[0], a[1], b[0], b[1]],
            ..Quad::rect([x0, y0], [x1 - x0, y1 - y0], color, 0.0)
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen: [f32; 2],
    _pad: [f32; 2],
}

pub struct QuadPipeline {
    pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buf: wgpu::Buffer,
    capacity: usize,
    count: u32,
}

impl QuadPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("quad.wgsl"));

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quad bgl"),
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quad bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Quad>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x2, // pos
                1 => Float32x2, // size
                2 => Float32x4, // color
                3 => Float32x4, // border_color
                4 => Float32,   // radius
                5 => Float32,   // border_width
                6 => Float32,   // softness
                7 => Float32,   // kind
                8 => Float32x4, // line
                9 => Float32x4, // clip
                10 => Float32,  // clip_radius
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quad pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[instance_layout],
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

        let instance_buf = Self::make_instance_buf(device, 512);

        QuadPipeline {
            pipeline,
            globals_buf,
            bind_group,
            instance_buf,
            capacity: 512,
            count: 0,
        }
    }

    fn make_instance_buf(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad instances"),
            size: (capacity * std::mem::size_of::<Quad>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Upload instances for this frame (all layers concatenated).
    /// `screen` is the logical surface size.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: [f32; 2],
        quads: &[Quad],
    ) {
        queue.write_buffer(
            &self.globals_buf,
            0,
            bytemuck::bytes_of(&Globals {
                screen,
                _pad: [0.0; 2],
            }),
        );

        if quads.len() > self.capacity {
            self.capacity = quads.len().next_power_of_two();
            self.instance_buf = Self::make_instance_buf(device, self.capacity);
        }
        if !quads.is_empty() {
            queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(quads));
        }
        self.count = quads.len() as u32;
    }

    pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.render_range(pass, 0..self.count);
    }

    /// Draw a sub-range of the uploaded instances (one scene layer).
    pub fn render_range(&self, pass: &mut wgpu::RenderPass<'_>, range: std::ops::Range<u32>) {
        if range.is_empty() || range.end > self.count {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buf.slice(..));
        pass.draw(0..6, range);
    }
}
