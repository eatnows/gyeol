//! Draws a [`Scene`] to a window with wgpu.
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use winit::{dpi::PhysicalSize, window::Window};

use crate::{
    error::{err, Error, Result},
    gpu::{Globals, GrowBuffer},
    scene::{Item, Path, PathCommand, Quad, Rect, Scene},
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
}

impl From<&Quad> for QuadInstance {
    fn from(q: &Quad) -> Self {
        QuadInstance {
            pos: [q.bounds.x, q.bounds.y],
            size: [q.bounds.w, q.bounds.h],
            background: q.background.to_array(),
            border_color: q.border_color.to_array(),
            radii: q.corner_radii,
            border_width: q.border_width,
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
}

/// A run of same-kind items drawn under one clip.
struct Batch {
    kind: BatchKind,
    range: std::ops::Range<u32>,
    clip: Option<Rect>,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    scale_factor: f32,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    quad_pipeline: wgpu::RenderPipeline,
    quad_instances: GrowBuffer,
    path_pipeline: wgpu::RenderPipeline,
    path_vertices: GrowBuffer,
    text: TextSystem,
}

impl Renderer {
    /// Sets up the GPU for `window`. Blocks until the adapter and device are ready.
    pub fn new(window: Arc<Window>) -> Result<Self> {
        pollster::block_on(Self::new_async(window))
    }

    async fn new_async(window: Arc<Window>) -> Result<Self> {
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
        surface.configure(&device, &config);

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
                visibility: wgpu::ShaderStages::VERTEX,
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

        let quad_pipeline = create_quad_pipeline(&device, config.format, &globals_layout);
        let quad_instances = GrowBuffer::new(&device, wgpu::BufferUsages::VERTEX);
        let path_pipeline = create_path_pipeline(&device, config.format, &globals_layout);
        let path_vertices = GrowBuffer::new(&device, wgpu::BufferUsages::VERTEX);
        let text = TextSystem::new(&device, config.format, &globals_layout);

        Ok(Renderer {
            surface,
            device,
            queue,
            config,
            scale_factor,
            globals,
            globals_group,
            quad_pipeline,
            quad_instances,
            path_pipeline,
            path_vertices,
            text,
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>, scale_factor: f32) {
        self.scale_factor = scale_factor;
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    /// The drawable area in logical pixels.
    pub fn logical_size(&self) -> (f32, f32) {
        (self.config.width as f32 / self.scale_factor, self.config.height as f32 / self.scale_factor)
    }

    /// The scissor rectangle in device pixels for a logical clip (`None` clip = the whole target), or
    /// `None` when nothing of it is visible.
    fn scissor(&self, clip: Option<Rect>) -> Option<(u32, u32, u32, u32)> {
        let (tw, th) = (self.config.width, self.config.height);
        let Some(clip) = clip else { return Some((0, 0, tw, th)) };
        let s = self.scale_factor;
        let x0 = ((clip.x * s).floor().max(0.) as u32).min(tw);
        let y0 = ((clip.y * s).floor().max(0.) as u32).min(th);
        let x1 = (((clip.x + clip.w) * s).ceil().max(0.) as u32).min(tw);
        let y1 = (((clip.y + clip.h) * s).ceil().max(0.) as u32).min(th);
        (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
    }

    pub fn render(&mut self, scene: &Scene, shaper: &mut Shaper) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let (w, h) = self.logical_size();
        self.queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&Globals { viewport: [w, h], _pad: [0.; 2] }));

        // Glyphs first (they may upload to the atlas), then walk the scene in painting order and group
        // neighbouring items of the same kind and clip into batches.
        let glyph_ranges = self.text.prepare(shaper, &self.device, &self.queue, scene.texts(), self.scale_factor);
        let mut quads: Vec<QuadInstance> = Vec::new();
        let mut paths: Vec<PathVertex> = Vec::new();
        let mut batches: Vec<Batch> = Vec::new();
        let mut next_text = 0;
        for item in &scene.items {
            let (kind, range, clip) = match item {
                Item::Quad(q) => {
                    quads.push(QuadInstance::from(q));
                    (BatchKind::Quads, quads.len() as u32 - 1..quads.len() as u32, q.clip)
                }
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
        self.path_vertices.write(&self.device, &self.queue, bytemuck::cast_slice(&paths));

        let clear = scene.background.map_or(wgpu::Color::BLACK, |c| wgpu::Color {
            r: c.r as f64,
            g: c.g as f64,
            b: c.b as f64,
            a: c.a as f64,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gyeolui frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gyeolui pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for batch in &batches {
                let Some((x, y, cw, ch)) = self.scissor(batch.clip) else { continue };
                pass.set_scissor_rect(x, y, cw, ch);
                match batch.kind {
                    BatchKind::Quads => {
                        pass.set_pipeline(&self.quad_pipeline);
                        pass.set_bind_group(0, &self.globals_group, &[]);
                        pass.set_vertex_buffer(0, self.quad_instances.buffer.slice(..));
                        pass.draw(0..6, batch.range.clone());
                    }
                    BatchKind::Glyphs => self.text.draw(&mut pass, &self.globals_group, batch.range.clone()),
                    BatchKind::Paths => {
                        pass.set_pipeline(&self.path_pipeline);
                        pass.set_bind_group(0, &self.globals_group, &[]);
                        pass.set_vertex_buffer(0, self.path_vertices.buffer.slice(..));
                        pass.draw(batch.range.clone(), 0..1);
                    }
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
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
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gyeol rect shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("rect.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gyeol rect layout"),
        bind_group_layouts: &[Some(globals_layout)],
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
                    0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32                ],
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
