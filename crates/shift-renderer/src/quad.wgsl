// Instanced SDF quads: rounded rects, borders, circles, soft shadows/glows.
// Coordinates are logical pixels; Globals.screen is the logical surface size.

struct Globals {
    screen: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct Inst {
    @location(0) pos: vec2<f32>,          // top-left
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,        // fill (straight alpha, sRGB)
    @location(3) border_color: vec4<f32>,
    @location(4) radius: f32,             // corner radius
    @location(5) border_width: f32,       // inner ring width, 0 = none
    @location(6) softness: f32,           // blur sigma for shadow/glow, 0 = crisp
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,        // px, relative to quad center
    @location(1) half_size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) radius: f32,
    @location(5) border_width: f32,
    @location(6) softness: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Inst) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vi];
    // expand geometry so soft edges have room to falloff
    let margin = inst.softness * 3.0;
    let p = inst.pos - vec2<f32>(margin) + (inst.size + vec2<f32>(margin * 2.0)) * corner;

    var out: VsOut;
    out.clip = vec4<f32>(
        p.x / globals.screen.x * 2.0 - 1.0,
        1.0 - p.y / globals.screen.y * 2.0,
        0.0,
        1.0,
    );
    out.local = p - (inst.pos + inst.size * 0.5);
    out.half_size = inst.size * 0.5;
    out.color = inst.color;
    out.border_color = inst.border_color;
    out.radius = inst.radius;
    out.border_width = inst.border_width;
    out.softness = inst.softness;
    return out;
}

fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let r = min(in.radius, min(in.half_size.x, in.half_size.y));
    let d = sd_rounded_box(in.local, in.half_size, r);

    var col = in.color;
    var alpha: f32;
    if (in.softness > 0.0) {
        // gaussian-ish falloff for shadows / glows
        let t = d / (in.softness * 1.5);
        alpha = exp(-t * t) * step(0.0, d) + (1.0 - step(0.0, d));
    } else {
        let aa = max(fwidth(d), 0.0001);
        alpha = 1.0 - smoothstep(-aa * 0.5, aa * 0.5, d);
        if (in.border_width > 0.0) {
            let inner = 1.0 - smoothstep(-aa * 0.5, aa * 0.5, d + in.border_width);
            col = mix(in.border_color, in.color, inner);
        }
    }

    return vec4<f32>(col.rgb, col.a * alpha);
}
