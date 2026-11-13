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
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32, inst: Instance) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[index];
    let px = inst.pos + c * inst.size;
    var out: VsOut;
    out.position = vec4<f32>(px.x / globals.viewport.x * 2.0 - 1.0, 1.0 - px.y / globals.viewport.y * 2.0, 0.0, 1.0);
    out.local = (c - vec2<f32>(0.5, 0.5)) * inst.size;
    out.size = inst.size;
    out.background = inst.background;
    out.border_color = inst.border_color;
    out.radii = inst.radii;
    out.border_width = inst.border_width;
    out.gradient_end = inst.gradient_end;
    out.gradient = inst.gradient;
    return out;
}

// Signed distance from `p` (relative to the rectangle's center) to a rounded box.
fn sd_round_box(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius, radius);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let half_size = in.size * 0.5;
    // Pick the corner radius of the quadrant the pixel is in (y grows downwards).
    var radius = in.radii.x;
    if in.local.x >= 0.0 && in.local.y < 0.0 { radius = in.radii.y; }
    if in.local.x >= 0.0 && in.local.y >= 0.0 { radius = in.radii.z; }
    if in.local.x < 0.0 && in.local.y >= 0.0 { radius = in.radii.w; }
    radius = min(radius, min(half_size.x, half_size.y));
    let d = sd_round_box(in.local, half_size, radius);
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
    return vec4<f32>(fill + edge, alpha);
}
