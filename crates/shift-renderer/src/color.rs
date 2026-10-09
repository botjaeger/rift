use shift_core::Color;

/// Theme color → wgpu clear color.
///
/// The surface uses a non-sRGB format (see `gpu.rs`): sRGB values are written
/// verbatim and composited in sRGB space, matching CSS compositing in the
/// design mock. No linearization.
pub fn to_wgpu(c: Color) -> wgpu::Color {
    wgpu::Color {
        r: c.r as f64,
        g: c.g as f64,
        b: c.b as f64,
        a: c.a as f64,
    }
}

/// Theme color → shader vec4 (straight alpha, sRGB values).
pub fn to_vec4(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// Straight-alpha RGBA8 → premultiplied (sRGB values, no linearization).
///
/// Textures are premultiplied before upload so bilinear filtering never
/// mixes the color of fully transparent texels into their neighbours (dark
/// fringes); pipelines then blend with `PREMULTIPLIED_ALPHA_BLENDING`, which
/// is the same sRGB-space composite as straight alpha.
pub fn premultiply_rgba8(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::premultiply_rgba8;

    #[test]
    fn premultiply_scales_rgb_by_alpha() {
        let mut px = [200, 100, 50, 255, 200, 100, 50, 0, 255, 255, 255, 128];
        premultiply_rgba8(&mut px);
        assert_eq!(&px[0..4], &[200, 100, 50, 255]);
        assert_eq!(&px[4..8], &[0, 0, 0, 0]);
        assert_eq!(&px[8..12], &[128, 128, 128, 128]);
    }
}
