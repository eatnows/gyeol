struct Globals {
    viewport: vec2<f32>,
    scale: f32, // device pixels per logical pixel: edges are anti-aliased over one device pixel
    _pad: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;
// Per-quad data too big for instance attributes, from the instance's `extras.x` on: its gradient layers,
// then its outer shadows, then its inset shadows.
//  - A gradient layer is 12 vec4s: [kind, stop count], [linear: dir.xy, length | radial: center.xy,
//    radii.xy], positions 0-3, positions 4-7, then 8 premultiplied stop colors.
//  - A shadow is 2 vec4s: [offset x, offset y, blur, spread], [straight rgba].
@group(1) @binding(0) var<storage, read> extras: array<vec4<f32>>;

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) background: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) radii: vec4<f32>, // top-left, top-right, bottom-right, bottom-left
    @location(5) border_width: f32,
    @location(6) extras: vec4<u32>, // offset into `extras`, gradient layers, outer shadows, inset shadows
    @location(7) pad: f32,          // how far the outer shadows reach past the box
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) background: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) radii: vec4<f32>,
    @location(5) border_width: f32,
    @location(6) @interpolate(flat) extras: vec4<u32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32, inst: Instance) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[index];
    // Grow the quad so the shadows, which spill past the box, have pixels to land on.
    let grown = inst.size + vec2<f32>(inst.pad, inst.pad) * 2.0;
    let px = inst.pos - vec2<f32>(inst.pad, inst.pad) + c * grown;
    var out: VsOut;
    out.position = vec4<f32>(px.x / globals.viewport.x * 2.0 - 1.0, 1.0 - px.y / globals.viewport.y * 2.0, 0.0, 1.0);
    out.local = (c - vec2<f32>(0.5, 0.5)) * grown;
    out.size = inst.size;
    out.background = inst.background;
    out.border_color = inst.border_color;
    out.radii = inst.radii;
    out.border_width = inst.border_width;
    out.extras = inst.extras;
    return out;
}

// Signed distance from `p` (relative to the rectangle's center) to a rounded box.
fn sd_round_box(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius, radius);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

// The corner radius of the quadrant `p` is in (y grows downwards), limited to what fits in the box.
fn radius_at(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    var radius = radii.x;
    if p.x >= 0.0 && p.y < 0.0 { radius = radii.y; }
    if p.x >= 0.0 && p.y >= 0.0 { radius = radii.z; }
    if p.x < 0.0 && p.y >= 0.0 { radius = radii.w; }
    return min(radius, min(size.x, size.y) * 0.5);
}

// Signed distance to the quad's rounded shape at `p`.
fn sd_shape(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    return sd_round_box(p, size * 0.5, radius_at(p, size, radii));
}

// erf(x), Abramowitz & Stegun 7.1.27 (error below 5e-4): enough for a shadow's falloff.
fn erf_approx(x: f32) -> f32 {
    let a = abs(x);
    let t = 1.0 + a * (0.278393 + a * (0.230389 + a * (0.000972 + a * 0.078108)));
    let t2 = t * t;
    return sign(x) * (1.0 - 1.0 / (t2 * t2));
}

fn gaussian(x: f32, sigma: f32) -> f32 {
    return exp(-x * x / (2.0 * sigma * sigma)) / (2.5066283 * sigma);
}

// A gaussian-blurred rounded box, integrated exactly across x and numerically (4 samples) across y.
// `p` is relative to the box's center. After Evan Wallace's "Fast Rounded Rectangle Shadows".
fn blurred_box(p: vec2<f32>, half_size: vec2<f32>, corner: f32, sigma: f32) -> f32 {
    let low = p.y - half_size.y;
    let high = p.y + half_size.y;
    let start = clamp(-3.0 * sigma, low, high);
    let end = clamp(3.0 * sigma, low, high);
    let step = (end - start) / 4.0;
    var y = start + step * 0.5;
    var value = 0.0;
    for (var i = 0; i < 4; i = i + 1) {
        let delta = min(half_size.y - corner - abs(p.y - y), 0.0);
        let curved = half_size.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
        let k = 0.70710678 / sigma;
        let integral = 0.5 + 0.5 * vec2<f32>(erf_approx((p.x - curved) * k), erf_approx((p.x + curved) * k));
        value = value + (integral.y - integral.x) * gaussian(y, sigma) * step;
        y = y + step;
    }
    return clamp(value, 0.0, 1.0);
}

// How much of a rounded box (centered at the origin) covers `p` once blurred by a CSS blur radius
// (gaussian sigma = blur / 2). Without blur it is the box's anti-aliased shape.
fn shadow_cover(p: vec2<f32>, half_size: vec2<f32>, corner: f32, blur: f32) -> f32 {
    if half_size.x <= 0.0 || half_size.y <= 0.0 { return 0.0; }
    if blur * globals.scale < 0.6 {
        return clamp(0.5 - sd_round_box(p, half_size, corner) * globals.scale, 0.0, 1.0);
    }
    return blurred_box(p, half_size, corner, blur * 0.5);
}

fn stop_position(first: vec4<f32>, second: vec4<f32>, i: u32) -> f32 {
    if i < 4u { return first[i]; }
    return second[i - 4u];
}

// The premultiplied color of the gradient layer at `base` for the point `p` (relative to the center).
fn gradient_layer(base: u32, p: vec2<f32>) -> vec4<f32> {
    let head = extras[base];
    let geometry = extras[base + 1u];
    var t: f32;
    if head.x < 0.5 {
        t = dot(p, geometry.xy) / max(geometry.z, 0.0001) + 0.5; // linear: 0 and 1 at the corners
    } else {
        t = length((p - geometry.xy) / geometry.zw); // radial: 1 at the ellipse
    }
    let positions_a = extras[base + 2u];
    let positions_b = extras[base + 3u];
    let count = u32(head.y);
    var previous = extras[base + 4u];
    var previous_at = stop_position(positions_a, positions_b, 0u);
    if t <= previous_at { return previous; }
    for (var i = 1u; i < count; i = i + 1u) {
        let next = extras[base + 4u + i];
        let next_at = stop_position(positions_a, positions_b, i);
        if t <= next_at {
            return mix(previous, next, (t - previous_at) / max(next_at - previous_at, 0.00001));
        }
        previous = next;
        previous_at = next_at;
    }
    return previous;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = sd_shape(in.local, in.size, in.radii);
    let outer = clamp(0.5 - d * globals.scale, 0.0, 1.0);
    let inner = clamp(0.5 - (d + in.border_width) * globals.scale, 0.0, 1.0);
    let ring = outer - inner;
    let half_size = in.size * 0.5;
    let layers = in.extras.y;
    let outer_shadows = in.extras.x + layers * 12u;
    let inset_shadows = outer_shadows + in.extras.z * 2u;

    // Premultiplied fill: the background color with the gradient layers painted over it, bottom first.
    var fill = vec4<f32>(in.background.rgb * in.background.a, in.background.a);
    for (var l = 0u; l < layers; l = l + 1u) {
        let layer = gradient_layer(in.extras.x + l * 12u, in.local);
        fill = layer + fill * (1.0 - layer.a);
    }
    fill = fill * inner;

    // Inset shadows: what an offset, shrunk, blurred copy of the padding box leaves uncovered, kept
    // inside the border. They go over the fill, under the border ring.
    for (var k = 0u; k < in.extras.w; k = k + 1u) {
        let shadow = extras[inset_shadows + k * 2u];
        let color = extras[inset_shadows + k * 2u + 1u];
        let p = in.local - shadow.xy;
        let inset = in.border_width + shadow.w;
        let corner = max(radius_at(p, in.size, in.radii) - inset, 0.0);
        let hole = shadow_cover(p, half_size - vec2<f32>(inset, inset), corner, shadow.z);
        let s = color.a * (1.0 - hole) * inner;
        fill = vec4<f32>(color.rgb * s, s) + fill * (1.0 - s);
    }

    // Premultiplied output: fill inside the border, border color on the ring.
    let edge = in.border_color.rgb * in.border_color.a * ring;
    var result = vec4<f32>(fill.rgb + edge, fill.a + in.border_color.a * ring);

    // Outer shadows, behind the quad and only outside it, so a translucent fill does not show them
    // through. Later shadows are painted over earlier ones.
    var behind = vec4<f32>(0.0);
    for (var k = 0u; k < in.extras.z; k = k + 1u) {
        let shadow = extras[outer_shadows + k * 2u];
        let color = extras[outer_shadows + k * 2u + 1u];
        let p = in.local - shadow.xy;
        let grown = half_size + vec2<f32>(shadow.w, shadow.w);
        let corner = max(radius_at(p, in.size, in.radii) + shadow.w, 0.0);
        let s = color.a * shadow_cover(p, grown, corner, shadow.z);
        behind = vec4<f32>(color.rgb * s, s) + behind * (1.0 - s);
    }
    return result + behind * (1.0 - outer);
}
