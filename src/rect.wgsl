struct Globals {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) background: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) radii: vec4<f32>, // top-left, top-right, bottom-right, bottom-left
    @location(5) border_width: f32,
    @location(6) gradient_end: vec4<f32>,
    @location(7) gradient: vec2<f32>, // enabled, angle in radians
    @location(8) shadow: vec4<f32>, // offset x, offset y, blur
    @location(9) shadow_color: vec4<f32>,
    @location(10) inset: vec4<f32>,
    @location(11) inset_color: vec4<f32>,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) background: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) radii: vec4<f32>,
    @location(5) border_width: f32,
    @location(6) gradient_end: vec4<f32>,
    @location(7) gradient: vec2<f32>,
    @location(8) shadow: vec4<f32>,
    @location(9) shadow_color: vec4<f32>,
    @location(10) inset: vec4<f32>,
    @location(11) inset_color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32, inst: Instance) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[index];
    // Grow the quad so the blurred shadow, which spills past the box, has pixels to land on.
    let pad = select(0.0, inst.shadow.z + max(abs(inst.shadow.x), abs(inst.shadow.y)) + 1.0, inst.shadow_color.a > 0.0);
    let grown = inst.size + vec2<f32>(pad, pad) * 2.0;
    let px = inst.pos - vec2<f32>(pad, pad) + c * grown;
    var out: VsOut;
    out.position = vec4<f32>(px.x / globals.viewport.x * 2.0 - 1.0, 1.0 - px.y / globals.viewport.y * 2.0, 0.0, 1.0);
    out.local = (c - vec2<f32>(0.5, 0.5)) * grown;
    out.size = inst.size;
    out.background = inst.background;
    out.border_color = inst.border_color;
    out.radii = inst.radii;
    out.border_width = inst.border_width;
    out.gradient_end = inst.gradient_end;
    out.gradient = inst.gradient;
    out.shadow = inst.shadow;
    out.shadow_color = inst.shadow_color;
    out.inset = inst.inset;
    out.inset_color = inst.inset_color;
    return out;
}

// Signed distance from `p` (relative to the rectangle's center) to a rounded box.
fn sd_round_box(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius, radius);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

// Signed distance to the quad's rounded shape at `p`, picking the corner radius of the quadrant
// `p` is in (y grows downwards).
fn sd_shape(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    var radius = radii.x;
    if p.x >= 0.0 && p.y < 0.0 { radius = radii.y; }
    if p.x >= 0.0 && p.y >= 0.0 { radius = radii.z; }
    if p.x < 0.0 && p.y >= 0.0 { radius = radii.w; }
    radius = min(radius, min(half_size.x, half_size.y));
    return sd_round_box(p, half_size, radius);
}

// How much of a shape with signed distance `d` is covered once blurred by a CSS blur radius.
// A smoothstep over +-blur stands in for the gaussian (sigma = blur / 2) edge.
fn blurred_cover(d: f32, blur: f32) -> f32 {
    if blur <= 0.5 { return clamp(0.5 - d, 0.0, 1.0); }
    return 1.0 - smoothstep(-blur, blur, d);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = sd_shape(in.local, in.size, in.radii);
    let outer = clamp(0.5 - d, 0.0, 1.0);
    let inner = clamp(0.5 - (d + in.border_width), 0.0, 1.0);
    let ring = outer - inner;
    // Premultiplied fill color, flat or a CSS-style linear gradient (interpolated premultiplied).
    var base = vec4<f32>(in.background.rgb * in.background.a, in.background.a);
    if in.gradient.x > 0.5 {
        // 0 rad points up, clockwise; the gradient line is as long as the box's projection on it.
        let dir = vec2<f32>(sin(in.gradient.y), -cos(in.gradient.y));
        let len = abs(in.size.x * dir.x) + abs(in.size.y * dir.y);
        let t = clamp(dot(in.local, dir) / max(len, 0.0001) + 0.5, 0.0, 1.0);
        let end = vec4<f32>(in.gradient_end.rgb * in.gradient_end.a, in.gradient_end.a);
        base = mix(base, end, t);
    }
    // Premultiplied output: fill inside the border, border color on the ring.
    let fill = base.rgb * inner;
    let edge = in.border_color.rgb * in.border_color.a * ring;
    let alpha = base.a * inner + in.border_color.a * ring;
    var color = vec4<f32>(fill + edge, alpha);
    if in.inset_color.a > 0.0 {
        // The inset shadow is whatever the offset, blurred copy of the shape leaves uncovered, kept
        // inside the border. It goes over the fill, under the border ring.
        let di = sd_shape(in.local - in.inset.xy, in.size, in.radii);
        let s = in.inset_color.a * (1.0 - blurred_cover(di + in.border_width, in.inset.z)) * inner;
        let over = vec4<f32>(in.inset_color.rgb * s, s);
        color = over + vec4<f32>(fill, base.a * inner) * (1.0 - s) + vec4<f32>(edge, in.border_color.a * ring);
    }
    if in.shadow_color.a > 0.0 {
        // Behind the quad and only outside it, so a translucent fill does not show the shadow through.
        let ds = sd_shape(in.local - in.shadow.xy, in.size, in.radii);
        let s = in.shadow_color.a * blurred_cover(ds, in.shadow.z) * (1.0 - outer);
        color = color + vec4<f32>(in.shadow_color.rgb * s, s);
    }
    return color;
}
