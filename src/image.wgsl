// Draws an app-owned RGBA image (premultiplied alpha) as one quad covering the image's rectangle,
// over an optional checkerboard. The quad's uv is the image's 0..1, so nearest sampling lands on
// exact texels; the scissor rect cuts the quad to the element.

struct Globals {
    viewport: vec2<f32>,
    scale: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) light: vec4<f32>,
    @location(3) dark: vec4<f32>,
    // x: checker cell in logical pixels (0 = no checkerboard), y: opacity
    @location(4) params: vec2<f32>,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) offset: vec2<f32>,
    @location(2) light: vec4<f32>,
    @location(3) dark: vec4<f32>,
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
    out.uv = c;
    out.offset = c * inst.size;
    out.light = inst.light;
    out.dark = inst.dark;
    out.params = inst.params;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSampleLevel(image, samp, in.uv, 0.0);
    var color = texel;
    if (in.params.x > 0.0) {
        let cell = floor(in.offset / in.params.x);
        let odd = (i32(cell.x) + i32(cell.y)) & 1;
        let bg = select(in.light, in.dark, odd == 1);
        color = texel + vec4<f32>(bg.rgb * bg.a, bg.a) * (1.0 - texel.a);
    }
    return color * in.params.y;
}
