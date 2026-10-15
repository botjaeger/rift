//! Offscreen frame capture for the script driver (`shot <path>`).
//!
//! Re-renders the already-prepared scene into a texture with `COPY_SRC`,
//! reads it back and writes a PNG. Used only by `--script`; never on the
//! normal frame path.

use std::path::Path;

use rift_renderer::{Gpu, ImagePipeline, QuadPipeline, SpritePipeline, TextEngine, to_wgpu};

#[allow(clippy::too_many_arguments)]
pub fn capture_frame(
    gpu: &Gpu,
    quads: &QuadPipeline,
    text: &TextEngine,
    sprites: &SpritePipeline,
    bg_image: Option<&ImagePipeline>,
    ranges: &[std::ops::Range<u32>],
    clear: rift_core::Color,
    path: &Path,
) {
    let (w, h) = (gpu.config.width, gpu.config.height);
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("capture"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: gpu.config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let bytes_per_row = (w * 4).div_ceil(256) * 256;
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("capture readback"),
        size: (bytes_per_row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("capture"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("capture pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(to_wgpu(clear)),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(img) = bg_image {
            img.render(&mut pass);
        }
        for (i, range) in ranges.iter().cloned().enumerate() {
            quads.render_range(&mut pass, range);
            sprites.render_layer(i, true, &mut pass);
            text.render_layer(i, &mut pass);
            sprites.render_layer(i, false, &mut pass);
        }
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);

    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    if let Err(e) = gpu.device.poll(wgpu::PollType::wait_indefinitely()) {
        log::warn!("capture: poll failed: {e:?}");
        return;
    }
    match rx.recv() {
        Ok(Ok(())) => {}
        other => {
            log::warn!("capture: map failed: {other:?}");
            return;
        }
    }
    let data = slice.get_mapped_range();
    let bgra = matches!(
        gpu.config.format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h {
        let start = (row * bytes_per_row) as usize;
        let line = &data[start..start + (w * 4) as usize];
        for px in line.as_chunks::<4>().0 {
            if bgra {
                rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            } else {
                rgba.extend_from_slice(&[px[0], px[1], px[2], px[3]]);
            }
        }
    }
    drop(data);
    buf.unmap();

    let write = || -> Result<(), Box<dyn std::error::Error>> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let file = std::fs::File::create(path)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header()?;
        writer.write_image_data(&rgba)?;
        Ok(())
    };
    match write() {
        Ok(()) => log::info!("capture: wrote {} ({w}x{h})", path.display()),
        Err(e) => log::warn!("capture: {}: {e}", path.display()),
    }
}
