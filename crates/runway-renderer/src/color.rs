use runway_core::Color;

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
