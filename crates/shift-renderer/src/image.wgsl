// Full-viewport background image. One triangle, no vertex buffer.
//
// The texture is `Rgba8Unorm` (NOT sRGB): like every other Shift color the
// image bytes are treated as sRGB values and composited in sRGB space, which
// matches how the design mock composites in CSS. See gpu.rs.

struct Uniforms {
    uv_offset: vec2<f32>,   // cover-fit crop origin, in uv space
    uv_scale: vec2<f32>,    // cover-fit crop extent, in uv space
    opacity: f32,
    // three scalars, not a vec3: a vec3 would align to 16 and push the struct
    // to 48 bytes, mismatching the 32-byte Rust `Uniforms`
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,   // 0..1 across the viewport, origin top-left
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // (0,0) (2,0) (0,2) — a single oversized triangle covering the viewport
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));

    var out: VsOut;
    out.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = u.uv_offset + in.uv * u.uv_scale;
    let c = textureSample(tex, samp, uv);
    return vec4<f32>(c.rgb * u.opacity, c.a * u.opacity);
}
