// Backdrop blur: a separable gaussian over the scene texture, then a masked composite of the result
// into the scene through a quad's rounded shape.

struct Globals {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

// ---- blur passes ---------------------------------------------------------------------------

struct Pass {
    @location(0) step: vec2<f32>,   // uv distance between taps (one target texel along the blur axis)
    @location(1) dims: vec2<f32>,   // size of the render target in pixels
    @location(2) sigma: f32,        // in taps
    @location(3) taps: f32,         // taps on each side of the center
};

struct PassOut {
    @builtin(position) position: vec4<f32>,
    @location(0) step: vec2<f32>,
    @location(1) dims: vec2<f32>,
    @location(2) sigma: f32,
    @location(3) taps: f32,
};

fn full_screen(index: u32) -> vec4<f32> {
    // One oversized triangle covering the target.
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

@vertex
fn vs_blur(@builtin(vertex_index) index: u32, inst: Pass) -> PassOut {
    var out: PassOut;
    out.position = full_screen(index);
    out.step = inst.step;
    out.dims = inst.dims;
    out.sigma = inst.sigma;
    out.taps = inst.taps;
    return out;
}

@fragment
fn fs_blur(in: PassOut) -> @location(0) vec4<f32> {
    let uv = in.position.xy / in.dims;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    let n = i32(in.taps);
    for (var i = -n; i <= n; i = i + 1) {
        let t = f32(i) / in.sigma;
        let w = exp(-0.5 * t * t);
        sum = sum + textureSampleLevel(source, samp, uv + in.step * f32(i), 0.0) * w;
        total = total + w;
    }
    return sum / total;
}

// ---- presenting the offscreen scene --------------------------------------------------------

struct BlitOut {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_blit(@builtin(vertex_index) index: u32) -> BlitOut {
    var out: BlitOut;
    out.position = full_screen(index);
    return out;
}

@fragment
fn fs_blit(in: BlitOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(source));
    return textureSampleLevel(source, samp, in.position.xy / size, 0.0);
}

// ---- compositing the blurred backdrop through a quad's shape --------------------------------

struct Quad {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(4) radii: vec4<f32>,
    @location(12) backdrop: vec2<f32>, // blur sigma, saturate
};

struct QuadOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) radii: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) saturate: f32,
};

@vertex
fn vs_quad(@builtin(vertex_index) index: u32, inst: Quad) -> QuadOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[index];
    let px = inst.pos + c * inst.size;
    var out: QuadOut;
    out.position = vec4<f32>(px.x / globals.viewport.x * 2.0 - 1.0, 1.0 - px.y / globals.viewport.y * 2.0, 0.0, 1.0);
    out.local = (c - vec2<f32>(0.5, 0.5)) * inst.size;
    out.size = inst.size;
    out.radii = inst.radii;
    out.uv = px / globals.viewport;
    out.saturate = inst.backdrop.y;
    return out;
}

// Same rounded-box coverage as rect.wgsl.
fn coverage(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    var radius = radii.x;
    if p.x >= 0.0 && p.y < 0.0 { radius = radii.y; }
    if p.x >= 0.0 && p.y >= 0.0 { radius = radii.z; }
    if p.x < 0.0 && p.y >= 0.0 { radius = radii.w; }
    radius = min(radius, min(half_size.x, half_size.y));
    let q = abs(p) - half_size + vec2<f32>(radius, radius);
    let d = length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;
    return clamp(0.5 - d, 0.0, 1.0);
}

// Step 1: clear the scene inside the shape (dst * (1 - coverage)), so the blurred copy replaces it.
@fragment
fn fs_erase(in: QuadOut) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, coverage(in.local, in.size, in.radii));
}

// Step 2: add the blurred, saturated backdrop (premultiplied) times the coverage.
@fragment
fn fs_fill(in: QuadOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(source, samp, in.uv, 0.0);
    let gray = dot(c.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let rgb = clamp(mix(vec3<f32>(gray), c.rgb, in.saturate), vec3<f32>(0.0), vec3<f32>(c.a));
    return vec4<f32>(rgb, c.a) * coverage(in.local, in.size, in.radii);
}
