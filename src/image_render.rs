//! Drawing [`ImageDraw`]s: each [`ImageBuffer`] gets one GPU texture, kept in step with the buffer's
//! revision (only the dirty rectangle is re-uploaded; nothing when the revision is unchanged), and
//! every draw is one quad over the image's rectangle.
use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};

use crate::{
    gpu::GrowBuffer,
    image::{Dirty, ImageDraw, PixelRect},
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImageInstance {
    pos: [f32; 2],
    size: [f32; 2],
    light: [f32; 4],
    dark: [f32; 4],
    params: [f32; 2],
}

struct GpuImage {
    texture: wgpu::Texture,
    size: (u32, u32),
    /// The buffer revision the texture holds.
    revision: u64,
    linear: wgpu::BindGroup,
    nearest: wgpu::BindGroup,
    last_used: u64,
}

pub(crate) struct ImageRenderer {
    pipeline: wgpu::RenderPipeline,
    instances: GrowBuffer,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    cache: HashMap<u64, GpuImage>,
    /// This frame: the buffer id and filter of each draw, in draw order.
    draws: Vec<(u64, bool)>,
    frame_instances: Vec<ImageInstance>,
    frame: u64,
    /// Bytes written to textures this frame (tests and diagnostics).
    pub uploaded_bytes: u64,
}

/// Textures untouched for this many frames are dropped.
const KEEP_FRAMES: u64 = 600;

impl ImageRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, globals_layout: &wgpu::BindGroupLayout, tex_layout: &wgpu::BindGroupLayout) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gyeol image shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("image.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gyeol image layout"),
            bind_group_layouts: &[Some(globals_layout), Some(tex_layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32x2];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gyeol image pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ImageInstance>() as u64,
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
        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("gyeol image sampler"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        ImageRenderer {
            pipeline,
            instances: GrowBuffer::new(device, wgpu::BufferUsages::VERTEX),
            linear: sampler(wgpu::FilterMode::Linear),
            nearest: sampler(wgpu::FilterMode::Nearest),
            cache: HashMap::new(),
            draws: Vec::new(),
            frame_instances: Vec::new(),
            frame: 0,
            uploaded_bytes: 0,
        }
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
        self.draws.clear();
        self.frame_instances.clear();
        self.uploaded_bytes = 0;
    }

    /// Brings the image's texture up to date and queues a draw; returns its draw index, or `None` when
    /// there is nothing to draw.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, tex_layout: &wgpu::BindGroupLayout, image: &ImageDraw) -> Option<u32> {
        let max = device.limits().max_texture_dimension_2d;
        let id = image.buffer.id();
        let frame = self.frame;
        let mut uploaded = 0;
        let ok = image.buffer.with_state(|state, pixels| {
            let (w, h) = state.size();
            if w == 0 || h == 0 || w > max || h > max {
                return None;
            }
            let stale_size = self.cache.get(&id).is_some_and(|g| g.size != (w, h));
            if stale_size {
                self.cache.remove(&id);
            }
            let gpu = self.cache.entry(id).or_insert_with(|| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("gyeol image"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                let group = |sampler: &wgpu::Sampler| {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("gyeol image group"),
                        layout: tex_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                        ],
                    })
                };
                let (linear, nearest) = (group(&self.linear), group(&self.nearest));
                // revision 0 is older than any history: the first sync uploads everything.
                GpuImage { texture, size: (w, h), revision: 0, linear, nearest, last_used: frame }
            });
            gpu.last_used = frame;
            let dirty = if gpu.revision == 0 { Dirty::All } else { state.dirty_since(gpu.revision) };
            let rect = match dirty {
                Dirty::Nothing => None,
                Dirty::All => Some(PixelRect::new(0, 0, w, h)),
                Dirty::Rect(r) => Some(r.clamped(w, h)).filter(|r| !r.is_empty()),
            };
            if let Some(r) = rect {
                // The rows start at the rectangle's corner but keep the buffer's stride, so no copy is needed.
                let offset = (r.y as usize * w as usize + r.x as usize) * 4;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &gpu.texture, mip_level: 0, origin: wgpu::Origin3d { x: r.x, y: r.y, z: 0 }, aspect: wgpu::TextureAspect::All },
                    &pixels[offset..],
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(r.h) },
                    wgpu::Extent3d { width: r.w, height: r.h, depth_or_array_layers: 1 },
                );
                uploaded = r.w as u64 * r.h as u64 * 4;
            }
            gpu.revision = state.revision();
            Some((w, h))
        });
        self.uploaded_bytes += uploaded;
        let (w, h) = ok?;
        if image.opacity <= 0. || image.zoom <= 0. {
            return None;
        }
        let (light, dark, cell) = match image.checkerboard {
            Some(c) if c.cell > 0. => (c.light.to_array(), c.dark.to_array(), c.cell),
            _ => ([0.; 4], [0.; 4], 0.),
        };
        self.draws.push((id, image.filter.nearest_at(image.zoom)));
        self.frame_instances.push(ImageInstance {
            pos: [image.origin.0, image.origin.1],
            size: [w as f32 * image.zoom, h as f32 * image.zoom],
            light,
            dark,
            params: [cell, image.opacity],
        });
        Some(self.draws.len() as u32 - 1)
    }

    /// Uploads this frame's instances; call once all images are prepared.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.instances.write(device, queue, bytemuck::cast_slice(&self.frame_instances));
        let frame = self.frame;
        self.cache.retain(|_, g| g.last_used + KEEP_FRAMES > frame);
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, globals_group: &wgpu::BindGroup, draws: std::ops::Range<u32>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, globals_group, &[]);
        pass.set_vertex_buffer(0, self.instances.buffer.slice(..));
        for i in draws {
            let (id, nearest) = self.draws[i as usize];
            let Some(gpu) = self.cache.get(&id) else { continue };
            pass.set_bind_group(1, if nearest { &gpu.nearest } else { &gpu.linear }, &[]);
            pass.draw(0..6, i..i + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        image::{ImageBuffer, ImageDraw, ImageFilter, PixelRect},
        scene::{Rect, Scene},
        Renderer, Shaper,
    };

    fn draw(buffer: &ImageBuffer) -> Scene {
        let mut scene = Scene::default();
        scene.push_image(ImageDraw { buffer: buffer.clone(), origin: (0., 0.), zoom: 1., filter: ImageFilter::Nearest, checkerboard: None, opacity: 1., clip: None::<Rect> });
        scene
    }

    fn px(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    }

    #[test]
    fn a_texture_is_uploaded_once_then_only_the_dirty_rectangle_then_nothing() {
        let Ok(mut renderer) = Renderer::offscreen((64., 64.), 1.) else { return eprintln!("skipping, no GPU") };
        let mut shaper = Shaper::new();
        let buffer = ImageBuffer::new(64, 64);
        buffer.fill([255, 0, 0, 255]);
        let scene = draw(&buffer);

        renderer.render_to_image(&scene, &mut shaper);
        assert_eq!(renderer.image_upload_bytes(), 64 * 64 * 4, "first frame: the whole image");
        let pixels = renderer.render_to_image(&scene, &mut shaper);
        assert_eq!(renderer.image_upload_bytes(), 0, "same revision: nothing is uploaded");
        assert_eq!(px(&pixels, 64, 30, 30), [255, 0, 0, 255]);

        buffer.edit(Some(PixelRect::new(10, 20, 8, 4)), |p, w| {
            for y in 20..24 {
                for x in 10..18 {
                    let i = ((y * w + x) * 4) as usize;
                    p[i..i + 4].copy_from_slice(&[0, 255, 0, 255]);
                }
            }
        });
        let pixels = renderer.render_to_image(&scene, &mut shaper);
        assert_eq!(renderer.image_upload_bytes(), 8 * 4 * 4, "only the dirty rectangle");
        assert_eq!(px(&pixels, 64, 12, 21), [0, 255, 0, 255], "the edit shows");
        assert_eq!(px(&pixels, 64, 9, 21), [255, 0, 0, 255], "its surroundings are untouched");
        assert_eq!(px(&pixels, 64, 12, 24), [255, 0, 0, 255]);

        // Two edits between frames upload their union.
        buffer.write_rect(PixelRect::new(0, 0, 2, 2), &[0, 0, 255, 255].repeat(4));
        buffer.write_rect(PixelRect::new(6, 6, 2, 2), &[0, 0, 255, 255].repeat(4));
        renderer.render_to_image(&scene, &mut shaper);
        assert_eq!(renderer.image_upload_bytes(), 8 * 8 * 4);

        buffer.resize(32, 16);
        buffer.fill([0, 0, 255, 255]);
        let pixels = renderer.render_to_image(&scene, &mut shaper);
        assert_eq!(renderer.image_upload_bytes(), 32 * 16 * 4, "a new size is a new texture, uploaded whole");
        assert_eq!(px(&pixels, 64, 20, 8), [0, 0, 255, 255]);
        assert_eq!(px(&pixels, 64, 40, 8)[3], 0, "outside the smaller image is empty");
    }

    #[test]
    fn two_renderers_each_catch_up_on_their_own() {
        let (Ok(mut a), Ok(mut b)) = (Renderer::offscreen((8., 8.), 1.), Renderer::offscreen((8., 8.), 1.)) else { return eprintln!("skipping, no GPU") };
        let mut shaper = Shaper::new();
        let buffer = ImageBuffer::new(8, 8);
        let scene = draw(&buffer);
        a.render_to_image(&scene, &mut shaper);
        buffer.write_rect(PixelRect::new(1, 1, 1, 1), &[9, 9, 9, 255]);
        a.render_to_image(&scene, &mut shaper);
        let pixels = b.render_to_image(&scene, &mut shaper); // never saw the earlier revisions
        assert_eq!(b.image_upload_bytes(), 8 * 8 * 4);
        assert_eq!(px(&pixels, 8, 1, 1), [9, 9, 9, 255]);
    }
}
