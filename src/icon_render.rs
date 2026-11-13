//! Drawing [`Icon`]s (the `svg` feature): each icon is rasterized on the CPU with tiny-skia at the exact
//! device-pixel size it is drawn at, uploaded as a coverage texture, and drawn as a tinted quad on whole
//! device pixels. That keeps strokes and fills crisp at any size and device scale, at the price of one
//! small texture per (icon, size) pair, cached across frames.
use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use tiny_skia::{FillRule as SkFillRule, LineCap as SkCap, LineJoin as SkJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::{
    gpu::GrowBuffer,
    icon::{FillRule, Icon, IconCommand, IconData, LineCap, LineJoin},
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct IconInstance {
    pos: [f32; 2],
    size: [f32; 2],
    color: [f32; 4],
}

/// Rasterizes `data` into a `width` x `height` coverage image (one byte per pixel, rows from the top).
/// The icon's `viewBox` is scaled uniformly to fit, centered (`preserveAspectRatio` `xMidYMid meet`).
pub(crate) fn rasterize(data: &IconData, width: u32, height: u32) -> Vec<u8> {
    let Some(mut pixmap) = Pixmap::new(width.max(1), height.max(1)) else { return vec![0; (width * height) as usize] };
    let view = data.view_box;
    let scale = (width as f32 / view.w).min(height as f32 / view.h);
    let (tx, ty) = ((width as f32 - view.w * scale) / 2. - view.x * scale, (height as f32 - view.h * scale) / 2. - view.y * scale);
    let base = Transform::from_row(scale, 0., 0., scale, tx, ty);

    for shape in &data.shapes {
        let mut builder = PathBuilder::new();
        for command in &shape.commands {
            match *command {
                IconCommand::MoveTo(x, y) => builder.move_to(x, y),
                IconCommand::LineTo(x, y) => builder.line_to(x, y),
                IconCommand::QuadTo(x1, y1, x, y) => builder.quad_to(x1, y1, x, y),
                IconCommand::CubicTo(x1, y1, x2, y2, x, y) => builder.cubic_to(x1, y1, x2, y2, x, y),
                IconCommand::Close => builder.close(),
            }
        }
        let Some(path) = builder.finish() else { continue };
        let [a, b, c, d, e, f] = shape.transform;
        // Points go through the shape's own transform first, then the fit into the box.
        let transform = base.pre_concat(Transform::from_row(a, b, c, d, e, f));
        let paint = |opacity: f32| {
            let mut paint = Paint::default();
            paint.set_color_rgba8(255, 255, 255, (opacity.clamp(0., 1.) * 255.).round() as u8);
            paint.anti_alias = true;
            paint
        };
        if let Some(fill) = shape.fill {
            let rule = if fill.rule == FillRule::EvenOdd { SkFillRule::EvenOdd } else { SkFillRule::Winding };
            pixmap.fill_path(&path, &paint(fill.opacity), rule, transform, None);
        }
        if let Some(stroke) = shape.stroke {
            let stroke = (
                Stroke {
                    width: stroke.width,
                    miter_limit: stroke.miter_limit,
                    line_cap: match stroke.cap {
                        LineCap::Butt => SkCap::Butt,
                        LineCap::Round => SkCap::Round,
                        LineCap::Square => SkCap::Square,
                    },
                    line_join: match stroke.join {
                        LineJoin::Miter => SkJoin::Miter,
                        LineJoin::Round => SkJoin::Round,
                        LineJoin::Bevel => SkJoin::Bevel,
                    },
                    dash: None,
                },
                paint(stroke.opacity),
            );
            pixmap.stroke_path(&path, &stroke.1, &stroke.0, transform, None);
        }
    }
    pixmap.data().chunks_exact(4).map(|px| px[3]).collect()
}

struct Cached {
    _texture: wgpu::Texture,
    group: wgpu::BindGroup,
    last_used: u64,
}

pub(crate) struct IconRenderer {
    pipeline: wgpu::RenderPipeline,
    instances: GrowBuffer,
    cache: HashMap<(u64, u32, u32), Cached>,
    /// This frame: the cache key of each draw, in draw order, and its instance.
    draws: Vec<(u64, u32, u32)>,
    frame_instances: Vec<IconInstance>,
    frame: u64,
}

/// Cached textures untouched for this many frames are dropped.
const KEEP_FRAMES: u64 = 600;

impl IconRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, globals_layout: &wgpu::BindGroupLayout, tex_layout: &wgpu::BindGroupLayout) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gyeol icon shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("icon.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gyeol icon layout"),
            bind_group_layouts: &[Some(globals_layout), Some(tex_layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gyeol icon pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<IconInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
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
        });
        IconRenderer { pipeline, instances: GrowBuffer::new(device, wgpu::BufferUsages::VERTEX), cache: HashMap::new(), draws: Vec::new(), frame_instances: Vec::new(), frame: 0 }
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.draws.clear();
        self.frame_instances.clear();
    }

    /// Rasterizes (or finds) `icon` at the device size it covers, and queues a draw. Returns its draw
    /// index, or `None` when there is nothing to draw.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tex_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        icon: &Icon,
        scale: f32,
    ) -> Option<u32> {
        let b = icon.bounds;
        // Whole device pixels, so the texture is sampled 1:1 and edges stay sharp.
        let (x, y) = ((b.x * scale).round(), (b.y * scale).round());
        let (w, h) = (((b.w * scale).round() as u32).max(1), ((b.h * scale).round() as u32).max(1));
        if icon.color.a <= 0. || icon.data.shapes.is_empty() || b.w <= 0. || b.h <= 0. || w > 8192 || h > 8192 {
            return None;
        }
        let key = (icon.data.id(), w, h);
        let frame = self.frame;
        let entry = self.cache.entry(key).or_insert_with(|| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("gyeol icon"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                &rasterize(&icon.data, w, h),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gyeol icon group"),
                layout: tex_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                ],
            });
            Cached { _texture: texture, group, last_used: frame }
        });
        entry.last_used = frame;
        self.draws.push(key);
        self.frame_instances.push(IconInstance { pos: [x / scale, y / scale], size: [w as f32 / scale, h as f32 / scale], color: icon.color.to_array() });
        Some(self.draws.len() as u32 - 1)
    }

    /// Uploads this frame's instances; call once all icons are prepared.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.instances.write(device, queue, bytemuck::cast_slice(&self.frame_instances));
        let (frame, cache) = (self.frame, &mut self.cache);
        cache.retain(|_, c| c.last_used + KEEP_FRAMES > frame);
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, globals_group: &wgpu::BindGroup, draws: std::ops::Range<u32>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, globals_group, &[]);
        pass.set_vertex_buffer(0, self.instances.buffer.slice(..));
        for i in draws {
            let Some(cached) = self.cache.get(&self.draws[i as usize]) else { continue };
            pass.set_bind_group(1, &cached.group, &[]);
            pass.draw(0..6, i..i + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        icon::{IconShape, IconStroke},
        scene::Rect,
    };

    fn shape(commands: Vec<IconCommand>, fill: bool, stroke: bool) -> IconShape {
        IconShape {
            commands,
            transform: [1., 0., 0., 1., 0., 0.],
            fill: fill.then_some(crate::icon::IconFill { rule: FillRule::NonZero, opacity: 1. }),
            stroke: stroke.then_some(IconStroke { width: 2., cap: LineCap::Butt, join: LineJoin::Miter, miter_limit: 4., opacity: 1. }),
        }
    }

    fn at(image: &[u8], width: u32, x: u32, y: u32) -> u8 {
        image[(y * width + x) as usize]
    }

    #[test]
    fn a_filled_square_covers_its_pixels_and_only_those() {
        use IconCommand::*;
        let icon = IconData::new(Rect::new(0., 0., 10., 10.), vec![shape(vec![MoveTo(2., 2.), LineTo(8., 2.), LineTo(8., 8.), LineTo(2., 8.), Close], true, false)]);
        let image = rasterize(&icon, 20, 20); // 2 pixels per unit
        assert_eq!(at(&image, 20, 10, 10), 255, "the middle");
        assert_eq!(at(&image, 20, 4, 4), 255, "the first covered pixel (unit 2 is pixel 4)");
        assert_eq!(at(&image, 20, 3, 10), 0, "just outside");
        assert_eq!(at(&image, 20, 16, 10), 0, "unit 8 is pixel 16, outside");
        assert_eq!(at(&image, 20, 15, 10), 255, "pixel 15 is the last inside");
    }

    #[test]
    fn the_icon_is_fitted_centered_and_the_shape_transform_applies_first() {
        use IconCommand::*;
        // A 10x10 unit square icon drawn in a 40x20 box is scaled by 2 and centered horizontally.
        let square = |transform| IconShape { transform, ..shape(vec![MoveTo(0., 0.), LineTo(2., 0.), LineTo(2., 2.), LineTo(0., 2.), Close], true, false) };
        let icon = IconData::new(Rect::new(0., 0., 10., 10.), vec![square([1., 0., 0., 1., 3., 4.])]);
        let image = rasterize(&icon, 40, 20);
        // Unit (3..5, 4..6) -> pixels x 3*2+10 .. 5*2+10 = 16..20, y 8..12.
        assert_eq!(at(&image, 40, 17, 9), 255);
        assert_eq!(at(&image, 40, 14, 9), 0);
        assert_eq!(at(&image, 40, 17, 6), 0);
        assert_eq!(at(&image, 40, 21, 9), 0);
        // And a scale transform is applied to the points before the translate of the fit.
        let icon = IconData::new(Rect::new(0., 0., 10., 10.), vec![square([2., 0., 0., 2., 0., 0.])]);
        let image = rasterize(&icon, 10, 10);
        assert_eq!(at(&image, 10, 3, 3), 255, "the 2x2 square became 4x4");
        assert_eq!(at(&image, 10, 5, 3), 0);
    }

    #[test]
    fn a_stroke_is_as_wide_as_asked_and_capped_as_asked() {
        use IconCommand::*;
        let line = |cap| {
            let mut s = shape(vec![MoveTo(4., 10.), LineTo(16., 10.)], false, true);
            s.stroke.as_mut().unwrap().cap = cap;
            IconData::new(Rect::new(0., 0., 20., 20.), vec![s])
        };
        let butt = rasterize(&line(LineCap::Butt), 20, 20);
        assert_eq!(at(&butt, 20, 10, 9), 255, "2 units wide: rows 9 and 10");
        assert_eq!(at(&butt, 20, 10, 10), 255);
        assert_eq!(at(&butt, 20, 10, 8), 0);
        assert_eq!(at(&butt, 20, 3, 10), 0, "a butt cap ends at the endpoint");
        let square = rasterize(&line(LineCap::Square), 20, 20);
        assert_eq!(at(&square, 20, 3, 10), 255, "a square cap sticks out by half the width");
        let round = rasterize(&line(LineCap::Round), 20, 20);
        // A round cap is a half disc of radius 1: a quarter disc (about 78%) falls in this pixel.
        assert!((180..230).contains(&at(&round, 20, 3, 10)), "a round cap sticks out, rounded: {}", at(&round, 20, 3, 10));
        assert_eq!(at(&square, 20, 3, 9), 255, "while a square cap fills the corner");
    }
}
