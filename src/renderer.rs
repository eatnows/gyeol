//! Draws a [`Scene`] to a window with wgpu.
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use winit::{dpi::PhysicalSize, window::Window};

use crate::{
    error::{err, Error, Result},
    gpu::{Globals, GrowBuffer},
    scene::{Gradient, GradientKind, Item, Path, PathCommand, Quad, RadialSize, Rect, Scene, MAX_GRADIENT_STOPS},
    shaper::Shaper,
    text::TextSystem,
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct QuadInstance {
    pos: [f32; 2],
    size: [f32; 2],
    background: [f32; 4],
    border_color: [f32; 4],
    radii: [f32; 4],
    border_width: f32,
    /// `[offset, gradient layers, outer shadows, inset shadows]`: where this quad's records start in the
    /// frame's `extras` buffer, counted in `vec4`s, and how many of each follow, in that order.
    extras: [u32; 4],
    /// How far the outer shadows reach past the box, in logical pixels.
    pad: f32,
    /// `[blur radius, saturate]`; only read when the quad has a backdrop blur.
    backdrop: [f32; 2],
}

/// `vec4`s one gradient layer takes in the extras buffer: 2 of geometry, 2 of stop positions, 8 colors.
/// (A shadow takes 2: offset/blur/spread and color.)
const LAYER_VEC4S: usize = 12;

/// Appends `gradient`'s layer record to `out` (the layout `rect.wgsl` reads); `false` when it has no stops.
fn encode_gradient(gradient: &Gradient, bounds: Rect, out: &mut Vec<[f32; 4]>) -> bool {
    let stops = gradient.resolved_stops();
    if stops.is_empty() {
        return false;
    }
    let (w, h) = (bounds.w, bounds.h);
    let (kind, geometry) = match gradient.kind {
        GradientKind::Linear { angle } => {
            // The gradient line is as long as the box's projection on it, so the corners get 0% and 100%.
            let (dx, dy) = (angle.to_radians().sin(), -angle.to_radians().cos());
            (0., [dx, dy, (w * dx).abs() + (h * dy).abs(), 0.])
        }
        GradientKind::Radial { center, size } => {
            let (cx, cy) = ((center.0 - 0.5) * w, (center.1 - 0.5) * h);
            // Distances from the center to the box's sides and corners.
            let sides = [cx + w / 2., w / 2. - cx, cy + h / 2., h / 2. - cy].map(f32::abs);
            let corner = (cx.abs() + w / 2.).hypot(cy.abs() + h / 2.);
            let (rx, ry) = match size {
                RadialSize::FarthestCorner => (corner, corner),
                RadialSize::ClosestSide => {
                    let r = sides.into_iter().fold(f32::INFINITY, f32::min);
                    (r, r)
                }
                RadialSize::Circle(r) => (r, r),
                RadialSize::Ellipse(rx, ry) => (rx, ry),
                RadialSize::EllipseFraction(fx, fy) => (fx * w, fy * h),
            };
            (1., [cx, cy, rx.max(0.0001), ry.max(0.0001)])
        }
    };
    let mut positions = [1.; MAX_GRADIENT_STOPS];
    let mut colors = [[0.; 4]; MAX_GRADIENT_STOPS];
    for (i, (position, color)) in stops.iter().enumerate() {
        positions[i] = *position;
        // Premultiplied, so stops blend the way CSS does: `red -> transparent` fades, it does not go dark.
        colors[i] = [color.r * color.a, color.g * color.a, color.b * color.a, color.a];
    }
    let start = out.len();
    out.push([kind, stops.len() as f32, 0., 0.]);
    out.push(geometry);
    out.push([positions[0], positions[1], positions[2], positions[3]]);
    out.push([positions[4], positions[5], positions[6], positions[7]]);
    out.extend(colors);
    debug_assert_eq!(out.len() - start, LAYER_VEC4S);
    true
}

impl QuadInstance {
    fn new(q: &Quad, extras: &mut Vec<[f32; 4]>) -> Self {
        let offset = extras.len() as u32;
        let layers = q.gradients.iter().filter(|g| encode_gradient(g, q.bounds, extras)).count() as u32;
        for shadow in q.shadows.iter().chain(q.inset_shadows.iter()) {
            extras.push([shadow.offset.0, shadow.offset.1, shadow.blur.max(0.), shadow.spread]);
            extras.push(shadow.color.to_array());
        }
        // A gaussian is negligible past 3 sigma = 1.5 blur radii.
        let pad = q.shadows.iter().map(|s| s.blur.max(0.) * 1.5 + s.spread.max(0.) + s.offset.0.abs().max(s.offset.1.abs()) + 1.).fold(0., f32::max);
        QuadInstance {
            pos: [q.bounds.x, q.bounds.y],
            size: [q.bounds.w, q.bounds.h],
            background: q.background.to_array(),
            border_color: q.border_color.to_array(),
            radii: q.corner_radii,
            border_width: q.border_width,
            extras: [offset, layers, q.shadows.len() as u32, q.inset_shadows.len() as u32],
            pad,
            backdrop: q.backdrop_blur.map_or([0.; 2], |b| [b.radius, b.saturate]),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PathVertex {
    pos: [f32; 2],
    color: [f32; 4],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BatchKind {
    Quads,
    Glyphs,
    Paths,
    /// Blur what is behind a quad; `range.start` indexes the frame's backdrop plans.
    Backdrop,
    /// Rasterized icons; `range` indexes this frame's icon draws.
    #[cfg(feature = "svg")]
    Icons,
    /// App-owned images; `range` indexes this frame's image draws.
    Images,
}

/// How far a gaussian is cut off, in standard deviations.
const BLUR_SIGMAS: f32 = 3.;
/// The most taps on each side of the center in one blur pass.
const BLUR_MAX_TAPS: f32 = 32.;
/// Downsample factors for the blur targets; larger blurs run on smaller textures.
const BLUR_LEVELS: [u32; 4] = [1, 2, 4, 8];
/// The widest gaussian (in target texels) a level is allowed to run before the next level takes over.
const BLUR_MAX_SIGMA: f32 = 6.;

type Scissor = (u32, u32, u32, u32);

/// One separable blur pass, drawn as a fullscreen triangle restricted by a scissor.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct BlurPass {
    /// uv distance between taps.
    step: [f32; 2],
    /// Size of the target texture in pixels.
    dims: [f32; 2],
    sigma: f32,
    taps: f32,
}

/// Everything the renderer needs to blur the backdrop of one quad, worked out without the GPU.
#[derive(Debug, PartialEq)]
struct BackdropPlan {
    /// Index into `BLUR_LEVELS`.
    level: usize,
    /// Scissors for the horizontal pass (into texture A) and the vertical pass (into B).
    blur_scissors: [Scissor; 2],
    /// The part of the screen the blurred copy replaces.
    composite: Scissor,
    passes: [BlurPass; 2],
}

/// The scissor rectangle in device pixels for a logical clip (`None` clip = the whole target), or
/// `None` when nothing of it is visible.
fn scissor_rect(clip: Option<Rect>, scale: f32, (tw, th): (u32, u32)) -> Option<Scissor> {
    let Some(clip) = clip else { return Some((0, 0, tw, th)) };
    let x0 = ((clip.x * scale).floor().max(0.) as u32).min(tw);
    let y0 = ((clip.y * scale).floor().max(0.) as u32).min(th);
    let x1 = (((clip.x + clip.w) * scale).ceil().max(0.) as u32).min(tw);
    let y1 = (((clip.y + clip.h) * scale).ceil().max(0.) as u32).min(th);
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

/// Plans the two blur passes behind `bounds` (logical pixels) on a `target`-sized (device pixels) scene.
/// `radius` is the gaussian's standard deviation in logical pixels. `None` when nothing is visible.
fn plan_backdrop(bounds: Rect, clip: Option<Rect>, radius: f32, scale: f32, target: (u32, u32)) -> Option<BackdropPlan> {
    let (tw, th) = target;
    let (bx, by, bw, bh) = scissor_rect(Some(bounds), scale, target)?;
    let (cx, cy, cw, ch) = scissor_rect(clip, scale, target)?;
    let (x0, y0) = (bx.max(cx), by.max(cy));
    let (x1, y1) = ((bx + bw).min(cx + cw), (by + bh).min(cy + ch));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }

    let sigma = (radius * scale).max(0.01);
    let level = BLUR_LEVELS.iter().position(|&k| sigma / k as f32 <= BLUR_MAX_SIGMA).unwrap_or(BLUR_LEVELS.len() - 1);
    let k = BLUR_LEVELS[level];
    let sigma_t = sigma / k as f32;
    let taps = (sigma_t * BLUR_SIGMAS).ceil().clamp(1., BLUR_MAX_TAPS);
    let dims = (tw.div_ceil(k), th.div_ceil(k));

    // The horizontal pass must also cover the rows the vertical pass will read above and below.
    let margin = (sigma * BLUR_SIGMAS).ceil() as u32;
    // One extra texel on every side: the composite samples these textures bilinearly, and texels outside
    // the drawn area are empty, so without it the quad's edge pixels would blend with transparent black.
    let to_target = |x0: u32, y0: u32, x1: u32, y1: u32| -> Scissor {
        let (tx0, ty0) = ((x0 / k).saturating_sub(1), (y0 / k).saturating_sub(1));
        let (tx1, ty1) = ((x1.div_ceil(k) + 2).min(dims.0), (y1.div_ceil(k) + 2).min(dims.1));
        (tx0, ty0, tx1 - tx0, ty1 - ty0)
    };
    let tall = to_target(x0, y0.saturating_sub(margin), x1, (y1 + margin).min(th));
    let region = to_target(x0, y0, x1, y1);
    let (dw, dh) = (dims.0 as f32, dims.1 as f32);
    Some(BackdropPlan {
        level,
        blur_scissors: [tall, region],
        composite: (x0, y0, x1 - x0, y1 - y0),
        passes: [
            BlurPass { step: [1. / dw, 0.], dims: [dw, dh], sigma: sigma_t, taps },
            BlurPass { step: [0., 1. / dh], dims: [dw, dh], sigma: sigma_t, taps },
        ],
    })
}

/// A texture the renderer can draw into and sample from.
struct Target {
    view: wgpu::TextureView,
    /// Binds the texture and the sampler for the blur and composite shaders.
    group: wgpu::BindGroup,
}

/// Two ping-pong textures at one downsample level.
struct BlurSet {
    a: Target,
    b: Target,
}

/// The scene rendered off screen, so it can be sampled while blurring. Only exists in frames that blur.
struct Offscreen {
    size: (u32, u32),
    scene: Target,
    sets: [Option<BlurSet>; 4],
}

struct BackdropPipelines {
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    blur: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    erase: wgpu::RenderPipeline,
    fill: wgpu::RenderPipeline,
    passes: GrowBuffer,
}

/// A run of same-kind items drawn under one clip.
struct Batch {
    kind: BatchKind,
    range: std::ops::Range<u32>,
    clip: Option<Rect>,
}

pub struct Renderer {
    /// `None` for a renderer that only draws offscreen ([`Renderer::offscreen`]).
    surface: Option<wgpu::Surface<'static>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    scale_factor: f32,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    quad_pipeline: wgpu::RenderPipeline,
    quad_instances: GrowBuffer,
    /// Gradient layers (and, later, shadows) of this frame's quads: too big for per-instance attributes.
    quad_extras: GrowBuffer,
    extras_layout: wgpu::BindGroupLayout,
    extras_group: Option<wgpu::BindGroup>,
    path_pipeline: wgpu::RenderPipeline,
    path_vertices: GrowBuffer,
    text: TextSystem,
    backdrop: BackdropPipelines,
    #[cfg(feature = "svg")]
    icons: crate::icon_render::IconRenderer,
    images: crate::image_render::ImageRenderer,
    offscreen: Option<Offscreen>,
}

impl Renderer {
    /// Sets up the GPU for `window`. Blocks until the adapter and device are ready. With `transparent`,
    /// the surface blends with what is behind the window wherever the scene leaves it clear.
    pub fn new(window: Arc<Window>, transparent: bool) -> Result<Self> {
        pollster::block_on(Self::new_async(window, transparent))
    }

    async fn new_async(window: Arc<Window>, transparent: bool) -> Result<Self> {
        let size = window.inner_size();
        let scale_factor = window.scale_factor() as f32;

        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).map_err(err("create surface"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(err("request adapter"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(err("request device"))?;

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| Error("the surface is not supported by the adapter".into()))?;
        // UI colors are authored in sRGB and blended as-is, so draw to a non-sRGB format.
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|f| !f.is_srgb()) {
            config.format = format;
        }
        if transparent {
            // Quads are blended premultiplied, so prefer the matching mode; Metal only offers PostMultiplied,
            // which just makes its layer non-opaque and is composited premultiplied as well.
            use wgpu::CompositeAlphaMode::{PostMultiplied, PreMultiplied};
            config.alpha_mode = [PreMultiplied, PostMultiplied]
                .into_iter()
                .find(|m| caps.alpha_modes.contains(m))
                .ok_or_else(|| Error("the surface cannot be transparent".into()))?;
        }
        surface.configure(&device, &config);
        Ok(Self::from_parts(Some(surface), device, queue, config, scale_factor))
    }

    /// A renderer with no window or surface, for [`Renderer::render_to_image`]. The image is
    /// `size` logical pixels at `scale_factor` device pixels per logical pixel, RGBA8, and transparent
    /// wherever the scene paints nothing. Blocks until the adapter and device are ready.
    pub fn offscreen(size: (f32, f32), scale_factor: f32) -> Result<Self> {
        pollster::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).await.map_err(err("request adapter"))?;
            let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.map_err(err("request device"))?;
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                // Non-sRGB like the window surface, so colors blend as authored.
                format: wgpu::TextureFormat::Rgba8Unorm,
                width: ((size.0 * scale_factor).round() as u32).max(1),
                height: ((size.1 * scale_factor).round() as u32).max(1),
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                // Anything but Opaque: a scene with no background starts fully clear.
                alpha_mode: wgpu::CompositeAlphaMode::PostMultiplied,
                view_formats: vec![],
                color_space: Default::default(),
            };
            Ok(Self::from_parts(None, device, queue, config, scale_factor))
        })
    }

    fn from_parts(surface: Option<wgpu::Surface<'static>>, device: wgpu::Device, queue: wgpu::Queue, config: wgpu::SurfaceConfiguration, scale_factor: f32) -> Self {
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gyeol globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gyeol globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gyeol globals group"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });

        let extras_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gyeol extras layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let quad_pipeline = create_quad_pipeline(&device, config.format, &globals_layout, &extras_layout);
        let quad_instances = GrowBuffer::new(&device, wgpu::BufferUsages::VERTEX);
        let quad_extras = GrowBuffer::new(&device, wgpu::BufferUsages::STORAGE);
        let path_pipeline = create_path_pipeline(&device, config.format, &globals_layout);
        let path_vertices = GrowBuffer::new(&device, wgpu::BufferUsages::VERTEX);
        let text = TextSystem::new(&device, config.format, &globals_layout);
        let backdrop = create_backdrop_pipelines(&device, config.format, &globals_layout);
        #[cfg(feature = "svg")]
        let icons = crate::icon_render::IconRenderer::new(&device, config.format, &globals_layout, &backdrop.tex_layout);
        let images = crate::image_render::ImageRenderer::new(&device, config.format, &globals_layout, &backdrop.tex_layout);

        Renderer {
            surface,
            device,
            queue,
            config,
            scale_factor,
            globals,
            globals_group,
            quad_pipeline,
            quad_instances,
            quad_extras,
            extras_layout,
            extras_group: None,
            path_pipeline,
            path_vertices,
            text,
            backdrop,
            #[cfg(feature = "svg")]
            icons,
            images,
            offscreen: None,
        }
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>, scale_factor: f32) {
        self.scale_factor = scale_factor;
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.offscreen = None;
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        }
    }

    pub(crate) fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// The target size in device pixels.
    pub(crate) fn size_in_pixels(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// The drawable area in logical pixels.
    pub fn logical_size(&self) -> (f32, f32) {
        (self.config.width as f32 / self.scale_factor, self.config.height as f32 / self.scale_factor)
    }

    fn scissor(&self, clip: Option<Rect>) -> Option<Scissor> {
        scissor_rect(clip, self.scale_factor, (self.config.width, self.config.height))
    }

    pub fn render(&mut self, scene: &Scene, shaper: &mut Shaper) {
        let Some(surface) = &self.surface else { return };
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.render_to(scene, shaper, &view);
        self.queue.present(frame);
    }

    /// Draws `scene` and returns it as tightly packed RGBA8 rows, top to bottom, with premultiplied
    /// alpha. The image is `logical_size() * scale_factor` pixels; works with or without a window.
    pub fn render_to_image(&mut self, scene: &Scene, shaper: &mut Shaper) -> Vec<u8> {
        let (width, height) = (self.config.width, self.config.height);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gyeol snapshot"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.render_to(scene, shaper, &view);

        // Copies need each row padded to 256 bytes.
        let row = width * 4;
        let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gyeol snapshot readback"),
            size: padded as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gyeol snapshot copy") });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.expect("map snapshot buffer"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("wait for the snapshot copy");
        let mapped = slice.get_mapped_range().expect("read the snapshot");
        let mut pixels = Vec::with_capacity((row * height) as usize);
        for rows in mapped.chunks(padded as usize) {
            pixels.extend_from_slice(&rows[..row as usize]);
        }
        pixels
    }

    /// Bytes of image pixels uploaded to textures by the latest frame (0 when every image was up to date).
    #[cfg(test)]
    pub(crate) fn image_upload_bytes(&self) -> u64 {
        self.images.uploaded_bytes
    }

    /// Draws `scene` into `view`, a texture of the surface's size and format.
    fn render_to(&mut self, scene: &Scene, shaper: &mut Shaper, view: &wgpu::TextureView) {
        let (w, h) = self.logical_size();
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&Globals { viewport: [w, h], scale: self.scale_factor, _pad: 0. }));

        // Glyphs first (they may upload to the atlas), then walk the scene in painting order and group
        // neighbouring items of the same kind and clip into batches.
        let glyph_ranges = self.text.prepare(shaper, &self.device, &self.queue, scene.texts(), self.scale_factor);
        let mut quads: Vec<QuadInstance> = Vec::new();
        let mut extras: Vec<[f32; 4]> = Vec::new();
        let mut paths: Vec<PathVertex> = Vec::new();
        let mut batches: Vec<Batch> = Vec::new();
        let mut backdrops: Vec<(BackdropPlan, u32)> = Vec::new(); // plan, index of the quad's instance
        #[cfg(feature = "svg")]
        self.icons.begin_frame();
        self.images.begin_frame();
        let mut next_text = 0;
        let target_size = (self.config.width, self.config.height);
        for item in &scene.items {
            let (kind, range, clip) = match item {
                Item::Quad(q) => {
                    if let Some(blur) = q.backdrop_blur {
                        let plan = plan_backdrop(q.bounds, q.clip, blur.radius, self.scale_factor, target_size);
                        if let Some(plan) = plan {
                            let at = backdrops.len() as u32;
                            backdrops.push((plan, quads.len() as u32));
                            batches.push(Batch { kind: BatchKind::Backdrop, range: at..at + 1, clip: q.clip });
                        }
                    }
                    quads.push(QuadInstance::new(q, &mut extras));
                    (BatchKind::Quads, quads.len() as u32 - 1..quads.len() as u32, q.clip)
                }
                #[cfg(feature = "svg")]
                Item::Icon(icon) => {
                    let prepared = self.icons.prepare(&self.device, &self.queue, &self.backdrop.tex_layout, &self.backdrop.sampler, icon, self.scale_factor);
                    match prepared {
                        Some(i) => (BatchKind::Icons, i..i + 1, icon.clip),
                        None => continue,
                    }
                }
                #[cfg(not(feature = "svg"))]
                Item::Icon(_) => continue, // icons need the `svg` feature to be rasterized
                Item::Image(image) => match self.images.prepare(&self.device, &self.queue, &self.backdrop.tex_layout, image) {
                    Some(i) => (BatchKind::Images, i..i + 1, image.clip),
                    None => continue,
                },
                Item::Text(t) => {
                    next_text += 1;
                    (BatchKind::Glyphs, glyph_ranges[next_text - 1].clone(), t.clip)
                }
                Item::Path(path) => {
                    let start = paths.len() as u32;
                    tessellate_path(path, &mut paths);
                    (BatchKind::Paths, start..paths.len() as u32, path.clip)
                }
            };
            if range.is_empty() {
                continue;
            }
            match batches.last_mut() {
                Some(last) if last.kind == kind && last.clip == clip && last.range.end == range.start => last.range.end = range.end,
                _ => batches.push(Batch { kind, range, clip }),
            }
        }
        self.quad_instances.write(&self.device, &self.queue, bytemuck::cast_slice(&quads));
        #[cfg(feature = "svg")]
        self.icons.upload(&self.device, &self.queue);
        self.images.upload(&self.device, &self.queue);
        if extras.is_empty() {
            extras.push([0.; 4]); // a storage binding cannot be empty
        }
        self.quad_extras.write(&self.device, &self.queue, bytemuck::cast_slice(&extras));
        self.extras_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gyeol extras group"),
            layout: &self.extras_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.quad_extras.buffer.as_entire_binding() }],
        }));
        self.path_vertices.write(&self.device, &self.queue, bytemuck::cast_slice(&paths));
        let blur_passes: Vec<BlurPass> = backdrops.iter().flat_map(|(plan, _)| plan.passes).collect();
        self.backdrop.passes.write(&self.device, &self.queue, bytemuck::cast_slice(&blur_passes));

        // On a transparent surface the clear color is premultiplied so a translucent background shows the
        // desktop through; with no background it starts fully clear. An opaque surface is cleared as before.
        let transparent = self.config.alpha_mode != wgpu::CompositeAlphaMode::Opaque;
        let clear = match scene.background {
            None if transparent => wgpu::Color::TRANSPARENT,
            None => wgpu::Color::BLACK,
            Some(c) => {
                let k = if transparent { c.a } else { 1. };
                wgpu::Color { r: (c.r * k) as f64, g: (c.g * k) as f64, b: (c.b * k) as f64, a: c.a as f64 }
            }
        };
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gyeolui frame") });
        if backdrops.is_empty() {
            let mut pass = begin_pass(&mut encoder, view, wgpu::LoadOp::Clear(clear));
            self.draw_batches(&mut pass, &batches);
        } else {
            // Blurring needs to sample what is painted so far, which the surface cannot do: paint into an
            // offscreen texture, and every time a blurred quad comes up, flush, blur its backdrop and
            // composite that, then carry on. The finished texture is copied to the surface at the end.
            let levels: Vec<usize> = backdrops.iter().map(|(plan, _)| plan.level).collect();
            self.ensure_offscreen(&levels);
            let offscreen = self.offscreen.as_ref().expect("just created");
            let mut load = wgpu::LoadOp::Clear(clear);
            let mut next = 0;
            loop {
                let end = batches[next..].iter().position(|b| b.kind == BatchKind::Backdrop).map_or(batches.len(), |p| next + p);
                {
                    let mut pass = begin_pass(&mut encoder, &offscreen.scene.view, load);
                    self.draw_batches(&mut pass, &batches[next..end]);
                }
                load = wgpu::LoadOp::Load;
                if end == batches.len() {
                    break;
                }
                let at = batches[end].range.start as usize;
                let (plan, quad) = &backdrops[at];
                let set = offscreen.sets[plan.level].as_ref().expect("created for every planned level");
                let first_pass = at as u32 * 2;
                for (i, (source, into)) in [(&offscreen.scene, &set.a), (&set.a, &set.b)].into_iter().enumerate() {
                    let mut pass = begin_pass(&mut encoder, &into.view, wgpu::LoadOp::Load);
                    let (x, y, w, h) = plan.blur_scissors[i];
                    pass.set_scissor_rect(x, y, w, h);
                    pass.set_pipeline(&self.backdrop.blur);
                    pass.set_bind_group(0, &self.globals_group, &[]);
                    pass.set_bind_group(1, &source.group, &[]);
                    pass.set_vertex_buffer(0, self.backdrop.passes.buffer.slice(..));
                    pass.draw(0..3, first_pass + i as u32..first_pass + i as u32 + 1);
                }
                let mut pass = begin_pass(&mut encoder, &offscreen.scene.view, wgpu::LoadOp::Load);
                let (x, y, w, h) = plan.composite;
                pass.set_scissor_rect(x, y, w, h);
                pass.set_bind_group(0, &self.globals_group, &[]);
                pass.set_bind_group(1, &set.b.group, &[]);
                pass.set_vertex_buffer(0, self.quad_instances.buffer.slice(..));
                for pipeline in [&self.backdrop.erase, &self.backdrop.fill] {
                    pass.set_pipeline(pipeline);
                    pass.draw(0..6, *quad..*quad + 1);
                }
                drop(pass);
                next = end + 1;
            }
            let mut pass = begin_pass(&mut encoder, view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT));
            pass.set_pipeline(&self.backdrop.blit);
            pass.set_bind_group(0, &self.globals_group, &[]);
            pass.set_bind_group(1, &offscreen.scene.group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
    }

    fn draw_batches(&self, pass: &mut wgpu::RenderPass<'_>, batches: &[Batch]) {
        for batch in batches {
            let Some((x, y, cw, ch)) = self.scissor(batch.clip) else { continue };
            pass.set_scissor_rect(x, y, cw, ch);
            match batch.kind {
                BatchKind::Quads => {
                    pass.set_pipeline(&self.quad_pipeline);
                    pass.set_bind_group(0, &self.globals_group, &[]);
                    pass.set_bind_group(1, self.extras_group.as_ref(), &[]);
                    pass.set_vertex_buffer(0, self.quad_instances.buffer.slice(..));
                    pass.draw(0..6, batch.range.clone());
                }
                BatchKind::Glyphs => self.text.draw(pass, &self.globals_group, batch.range.clone()),
                BatchKind::Paths => {
                    pass.set_pipeline(&self.path_pipeline);
                    pass.set_bind_group(0, &self.globals_group, &[]);
                    pass.set_vertex_buffer(0, self.path_vertices.buffer.slice(..));
                    pass.draw(batch.range.clone(), 0..1);
                }
                #[cfg(feature = "svg")]
                BatchKind::Icons => self.icons.draw(pass, &self.globals_group, batch.range.clone()),
                BatchKind::Images => self.images.draw(pass, &self.globals_group, batch.range.clone()),
                BatchKind::Backdrop => {}
            }
        }
    }

    /// Makes sure the offscreen scene texture and the blur textures for `levels` exist at the current size.
    fn ensure_offscreen(&mut self, levels: &[usize]) {
        let size = (self.config.width, self.config.height);
        if self.offscreen.as_ref().is_none_or(|o| o.size != size) {
            let scene = self.make_target(size.0, size.1);
            self.offscreen = Some(Offscreen { size, scene, sets: [None, None, None, None] });
        }
        for &level in levels {
            if self.offscreen.as_ref().is_some_and(|o| o.sets[level].is_none()) {
                let k = BLUR_LEVELS[level];
                let set = BlurSet { a: self.make_target(size.0.div_ceil(k), size.1.div_ceil(k)), b: self.make_target(size.0.div_ceil(k), size.1.div_ceil(k)) };
                self.offscreen.as_mut().expect("just created").sets[level] = Some(set);
            }
        }
    }

    fn make_target(&self, width: u32, height: u32) -> Target {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gyeol offscreen"),
            size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gyeol offscreen group"),
            layout: &self.backdrop.tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.backdrop.sampler) },
            ],
        });
        Target { view, group }
    }
}

fn begin_pass<'a>(encoder: &'a mut wgpu::CommandEncoder, view: &'a wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("gyeolui pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

fn create_backdrop_pipelines(device: &wgpu::Device, format: wgpu::TextureFormat, globals_layout: &wgpu::BindGroupLayout) -> BackdropPipelines {
    let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gyeol backdrop layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("gyeol backdrop sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gyeol blur shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gyeol blur layout"),
        bind_group_layouts: &[Some(globals_layout), Some(&tex_layout)],
        immediate_size: 0,
    });
    let pass_attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32, 3 => Float32];
    // The composite only reads position, size, radii and the backdrop settings out of the quad instance.
    let quad_attributes = [
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: std::mem::offset_of!(QuadInstance, pos) as u64, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: std::mem::offset_of!(QuadInstance, size) as u64, shader_location: 1 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: std::mem::offset_of!(QuadInstance, radii) as u64, shader_location: 4 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: std::mem::offset_of!(QuadInstance, backdrop) as u64, shader_location: 8 },
    ];
    let make = |label: &str, vs: &str, fs: &str, buffers: &[Option<wgpu::VertexBufferLayout<'_>>], blend: Option<wgpu::BlendState>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };
    let pass_buffer = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<BlurPass>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &pass_attributes,
    })];
    let quad_buffer = [Some(wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<QuadInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &quad_attributes,
    })];
    let component = |src, dst| wgpu::BlendComponent { src_factor: src, dst_factor: dst, operation: wgpu::BlendOperation::Add };
    let blend = |c: wgpu::BlendComponent| Some(wgpu::BlendState { color: c, alpha: c });
    use wgpu::BlendFactor::{One, OneMinusSrcAlpha, Zero};
    BackdropPipelines {
        blur: make("gyeol blur pipeline", "vs_blur", "fs_blur", &pass_buffer, None),
        blit: make("gyeol blit pipeline", "vs_blit", "fs_blit", &[], None),
        erase: make("gyeol backdrop erase pipeline", "vs_quad", "fs_erase", &quad_buffer, blend(component(Zero, OneMinusSrcAlpha))),
        fill: make("gyeol backdrop fill pipeline", "vs_quad", "fs_fill", &quad_buffer, blend(component(One, One))),
        passes: GrowBuffer::new(device, wgpu::BufferUsages::VERTEX),
        tex_layout,
        sampler,
    }
}

/// Turns each segment into two triangles. Curves are flattened finely enough for the small,
/// two-pixel graph lanes this primitive was introduced for.
fn tessellate_path(path: &Path, vertices: &mut Vec<PathVertex>) {
    if path.width <= 0. || path.color.a <= 0. {
        return;
    }
    let mut at = None;
    for command in &path.commands {
        match *command {
            PathCommand::MoveTo(x, y) => at = Some((x, y)),
            PathCommand::LineTo(x, y) => {
                if let Some(from) = at {
                    push_segment(vertices, from, (x, y), path.width, path.color.to_array());
                }
                at = Some((x, y));
            }
            PathCommand::CubicTo(cx0, cy0, cx1, cy1, x, y) => {
                if let Some(from) = at {
                    let mut previous = from;
                    // Twelve pieces make the graph's 16px-wide curves visually smooth while
                    // keeping the display list inexpensive.
                    for i in 1..=12 {
                        let t = i as f32 / 12.;
                        let point = cubic(from, (cx0, cy0), (cx1, cy1), (x, y), t);
                        push_segment(vertices, previous, point, path.width, path.color.to_array());
                        previous = point;
                    }
                }
                at = Some((x, y));
            }
        }
    }
}

fn cubic(a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32), t: f32) -> (f32, f32) {
    let u = 1. - t;
    let u2 = u * u;
    let t2 = t * t;
    (
        u2 * u * a.0 + 3. * u2 * t * b.0 + 3. * u * t2 * c.0 + t2 * t * d.0,
        u2 * u * a.1 + 3. * u2 * t * b.1 + 3. * u * t2 * c.1 + t2 * t * d.1,
    )
}

fn push_segment(vertices: &mut Vec<PathVertex>, a: (f32, f32), b: (f32, f32), width: f32, color: [f32; 4]) {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f32::EPSILON {
        return;
    }
    let half = width / 2.;
    let nx = -dy / length * half;
    let ny = dx / length * half;
    let a0 = PathVertex { pos: [a.0 + nx, a.1 + ny], color };
    let a1 = PathVertex { pos: [a.0 - nx, a.1 - ny], color };
    let b0 = PathVertex { pos: [b.0 + nx, b.1 + ny], color };
    let b1 = PathVertex { pos: [b.0 - nx, b.1 - ny], color };
    vertices.extend([a0, a1, b1, a0, b1, b0]);
}

fn create_quad_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    globals_layout: &wgpu::BindGroupLayout,
    extras_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gyeol rect shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("rect.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gyeol rect layout"),
        bind_group_layouts: &[Some(globals_layout), Some(extras_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gyeol rect pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<QuadInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32,
                    6 => Uint32x4, 7 => Float32, 8 => Float32x2,
                ],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn create_path_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    globals_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gyeol path shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("path.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gyeol path layout"),
        bind_group_layouts: &[Some(globals_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gyeol path pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<PathVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Color;

    const SCREEN: (u32, u32) = (800, 600);

    #[test]
    fn small_blurs_run_at_full_resolution_and_big_ones_are_downsampled() {
        let level = |radius| plan_backdrop(Rect::new(100., 100., 200., 100.), None, radius, 1., SCREEN).unwrap().level;
        assert_eq!(level(4.), 0);
        assert_eq!(level(10.), 1, "sigma 10 is over the full-resolution limit");
        assert_eq!(level(24.), 2);
        assert_eq!(level(500.), 3, "the coarsest level takes everything else");
    }

    #[test]
    fn a_backdrop_plan_is_limited_to_the_quad_and_its_clip() {
        let bounds = Rect::new(100., 100., 200., 100.);
        let plan = plan_backdrop(bounds, None, 4., 1., SCREEN).unwrap();
        assert_eq!(plan.composite, (100, 100, 200, 100));
        // The horizontal pass also covers the rows the vertical pass reads: 3 sigma = 12px above and below.
        assert_eq!(plan.blur_scissors[0], (99, 87, 203, 127), "with a texel of slack on each side");

        let clipped = plan_backdrop(bounds, Some(Rect::new(0., 0., 150., 150.)), 4., 1., SCREEN).unwrap();
        assert_eq!(clipped.composite, (100, 100, 50, 50));
        assert_eq!(plan_backdrop(bounds, Some(Rect::new(500., 0., 10., 10.)), 4., 1., SCREEN), None, "clipped away");
        assert_eq!(plan_backdrop(Rect::new(900., 0., 10., 10.), None, 4., 1., SCREEN), None, "off screen");
    }

    #[test]
    fn the_blur_radius_scales_with_the_display_and_downsampling_keeps_taps_bounded() {
        let plan = plan_backdrop(Rect::new(0., 0., 100., 100.), None, 24., 2., (1600, 1200)).unwrap();
        assert_eq!(plan.level, 3, "sigma 48 device pixels needs a /8 target");
        assert_eq!(plan.passes[0].sigma, 6.);
        assert_eq!(plan.passes[0].taps, 18.);
        assert_eq!(plan.passes[0].dims, [200., 150.]);
    }

    #[test]
    fn a_gradient_layer_is_encoded_with_premultiplied_colors_and_resolved_geometry() {
        use crate::scene::Color;
        let clear_red = Color::hex(0xff0000).with_alpha(0.5);
        let gradient = Gradient::linear(90.).stop(clear_red).stop(Color::TRANSPARENT);
        let mut out = Vec::new();
        assert!(encode_gradient(&gradient, Rect::new(0., 0., 200., 100.), &mut out));
        assert_eq!(out.len(), LAYER_VEC4S);
        assert_eq!(out[0], [0., 2., 0., 0.], "linear, two stops");
        assert!((out[1][0] - 1.).abs() < 1e-6 && out[1][1].abs() < 1e-6 && (out[1][2] - 200.).abs() < 1e-3, "pointing right, as long as the width");
        assert_eq!(&out[2][..2], &[0., 1.]);
        assert_eq!(out[4], [0.5, 0., 0., 0.5], "premultiplied");
        assert_eq!(out[5], [0.; 4]);

        out.clear();
        let radial = Gradient::radial((0.25, 0.5), RadialSize::FarthestCorner).stop(Color::hex(0xffffff)).stop(Color::hex(0));
        assert!(encode_gradient(&radial, Rect::new(0., 0., 200., 100.), &mut out));
        assert_eq!(out[0][0], 1.);
        assert_eq!(&out[1][..2], &[-50., 0.], "center relative to the box's middle");
        assert!((out[1][2] - 150f32.hypot(50.)).abs() < 1e-3, "farthest corner is the right one: {:?}", out[1]);

        assert!(!encode_gradient(&Gradient::linear(0.), Rect::default(), &mut out), "no stops, no layer");
    }

    #[test]
    fn a_line_becomes_two_triangles() {
        let mut vertices = Vec::new();
        tessellate_path(&Path::stroke(Color::hex(0xffffff), 2.).move_to(0., 0.).line_to(10., 0.), &mut vertices);
        assert_eq!(vertices.len(), 6);
        assert_eq!(vertices[0].pos, [0., 1.]);
        assert_eq!(vertices[2].pos, [10., -1.]);
    }

    #[test]
    fn a_cubic_is_flattened_into_segments() {
        let mut vertices = Vec::new();
        tessellate_path(&Path::stroke(Color::hex(0xffffff), 2.).move_to(0., 0.).cubic_to(0., 10., 10., 10., 10., 0.), &mut vertices);
        assert_eq!(vertices.len(), 12 * 6);
    }
}
