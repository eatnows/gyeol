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
    @location(4) params: vec2<f32>, // border width, corner radius
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) background: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) params: vec2<f32>,
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
    out.params = inst.params;
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
    let radius = min(in.params.y, min(half_size.x, half_size.y));
    let d = sd_round_box(in.local, half_size, radius);
    let outer = clamp(0.5 - d, 0.0, 1.0);
    let inner = clamp(0.5 - (d + in.params.x), 0.0, 1.0);
    let ring = outer - inner;
    // Premultiplied output: fill inside the border, border color on the ring.
    let fill = in.background.rgb * in.background.a * inner;
    let edge = in.border_color.rgb * in.border_color.a * ring;
    let alpha = in.background.a * inner + in.border_color.a * ring;
    return vec4<f32>(fill + edge, alpha);
}
