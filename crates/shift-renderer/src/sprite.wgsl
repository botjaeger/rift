// Inline terminal images: one textured quad per instance.
//
// Rects arrive in logical px and are mapped to clip space with the viewport
// uniform. Textures are `Rgba8Unorm` and composited in sRGB space like every
// other Shift surface (see gpu.rs / image.wgsl).

struct Viewport {
    size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> vp: Viewport;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct Inst {
    @location(0) rect: vec4<f32>,  // x, y, w, h (logical px)
    @location(1) uv: vec4<f32>,    // u0, v0, u1, v1
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Inst) -> VsOut {
    // two triangles: (0,0) (1,0) (0,1) / (0,1) (1,0) (1,1)
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi];
    let px = inst.rect.xy + c * inst.rect.zw;
    var out: VsOut;
    out.clip = vec4<f32>(px.x / vp.size.x * 2.0 - 1.0, 1.0 - px.y / vp.size.y * 2.0, 0.0, 1.0);
    out.uv = mix(inst.uv.xy, inst.uv.zw, c);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
