// Instanced SDF quads: rounded rects, borders, circles, line segments, soft
// shadows/glows, each optionally clipped to a rounded rect.
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
    @location(5) border_width: f32,       // inner ring width (line: stroke width), 0 = none
    @location(6) softness: f32,           // blur sigma for shadow/glow, 0 = crisp
    @location(7) kind: f32,               // 0 = rounded box, 1 = line segment
    @location(8) line: vec4<f32>,         // segment end points (a.xy, b.xy)
    @location(9) clip: vec4<f32>,         // rounded clip rect, w <= 0 = none
    @location(10) clip_radius: f32,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) local: vec2<f32>,        // px, relative to quad center
    @location(1) half_size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) radius: f32,
    @location(5) border_width: f32,
    @location(6) softness: f32,
    @location(7) kind: f32,
    @location(8) line: vec4<f32>,
    @location(9) clip: vec4<f32>,
    @location(10) clip_radius: f32,
    @location(11) world: vec2<f32>,       // px, absolute
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
    out.clip_pos = vec4<f32>(
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
    out.kind = inst.kind;
    out.line = inst.line;
    out.clip = inst.clip;
    out.clip_radius = inst.clip_radius;
    out.world = p;
    return out;
}

fn sd_rounded_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// erf, max error 5e-4 (Abramowitz & Stegun 7.1.27)
fn erf(x: f32) -> f32 {
    let a = abs(x);
    var r = 1.0 + (0.278393 + (0.230389 + 0.078108 * a * a) * a) * a;
    r = r * r;
    return sign(x) * (1.0 - 1.0 / (r * r));
}

// Coverage of a box (half size `b`, centred on 0) blurred by sigma `s`.
fn box_blur(p: vec2<f32>, b: vec2<f32>, s: f32) -> f32 {
    let k = 1.0 / (s * sqrt(2.0));
    let lo = (p + b) * k;
    let hi = (p - b) * k;
    let x = 0.5 * (erf(lo.x) - erf(hi.x));
    let y = 0.5 * (erf(lo.y) - erf(hi.y));
    return x * y;
}

fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 0.0001), 0.0, 1.0);
    return length(pa - ba * h);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var col = in.color;
    var alpha: f32;
    if (in.kind > 0.5) {
        // round-capped stroke; coverage over one pixel for crisp edges
        let d = sd_segment(in.world, in.line.xy, in.line.zw) - in.border_width * 0.5;
        alpha = clamp(0.5 - d, 0.0, 1.0);
    } else {
        let r = min(in.radius, min(in.half_size.x, in.half_size.y));
        let d = sd_rounded_box(in.local, in.half_size, r);
        if (in.softness > 0.0) {
            // shadows / glows: the box blurred by a gaussian (CSS
            // box-shadow). Outside a corner it fades to a quarter of the
            // edge's darkness, so a rounded card on top keeps round corners;
            // a shape solid up to its edge showed square ones.
            alpha = box_blur(in.local, in.half_size, in.softness);
        } else {
            let aa = max(fwidth(d), 0.0001);
            alpha = 1.0 - smoothstep(-aa * 0.5, aa * 0.5, d);
            if (in.border_width > 0.0) {
                let inner = 1.0 - smoothstep(-aa * 0.5, aa * 0.5, d + in.border_width);
                // the ring composited over the fill (CSS: the border paints
                // over its background). Mixing the straight-alpha colors
                // instead pulled an antialiased pixel's rgb to a translucent
                // border's white at near-full alpha: light streaks along
                // corner arcs and fractional edges
                let ring = (1.0 - inner) * in.border_color.a;
                let a = ring + in.color.a * (1.0 - ring);
                let rgb = in.border_color.rgb * ring + in.color.rgb * in.color.a * (1.0 - ring);
                col = vec4<f32>(rgb / max(a, 0.0001), a);
            }
        }
    }

    if (in.clip.z > 0.0) {
        let half = in.clip.zw * 0.5;
        let cr = min(in.clip_radius, min(half.x, half.y));
        let cd = sd_rounded_box(in.world - (in.clip.xy + half), half, cr);
        let aa = max(fwidth(cd), 0.0001);
        alpha = alpha * (1.0 - smoothstep(-aa * 0.5, aa * 0.5, cd));
    }

    return vec4<f32>(col.rgb, col.a * alpha);
}
