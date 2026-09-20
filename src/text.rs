//! Text: shaping and rasterizing with cosmic-text, drawn from a glyph atlas texture.
use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use cosmic_text::{CacheKey, FontSystem, SwashCache, SwashContent};

use crate::{gpu::GrowBuffer, scene::Text, shaper::Shaper};

const ATLAS_SIZE: u32 = 2048;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlyphInstance {
    pos: [f32; 2],
    size: [f32; 2],
    uv_pos: [f32; 2],
    uv_size: [f32; 2],
    color: [f32; 4],
}

/// Where a rasterized glyph lives in the atlas, and how it sits relative to its pen position.
#[derive(Clone, Copy)]
struct GlyphSlot {
    uv_pos: [f32; 2],
    uv_size: [f32; 2],
    size: [f32; 2],
    left: i32,
    top: i32,
}

/// Packs glyph bitmaps into one texture in rows ("shelves").
struct Atlas {
    texture: wgpu::Texture,
    x: u32,
    y: u32,
    row_height: u32,
}

impl Atlas {
    fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if self.x + w + 1 > ATLAS_SIZE {
            self.x = 0;
            self.y += self.row_height + 1;
            self.row_height = 0;
        }
        // ponytail: when the atlas is full, glyphs stop rendering; evict/clear when a real app hits this.
        if self.y + h + 1 > ATLAS_SIZE || w > ATLAS_SIZE {
            return None;
        }
        let at = (self.x, self.y);
        self.x += w + 1;
        self.row_height = self.row_height.max(h);
        Some(at)
    }
}

pub(crate) struct TextSystem {
    swash: SwashCache,
    atlas: Atlas,
    glyphs: HashMap<CacheKey, Option<GlyphSlot>>,
    pipeline: wgpu::RenderPipeline,
    atlas_group: wgpu::BindGroup,
    instances: GrowBuffer,
    count: u32,
}

impl TextSystem {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, globals_layout: &wgpu::BindGroupLayout) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gyeol glyph atlas"),
            size: wgpu::Extent3d { width: ATLAS_SIZE, height: ATLAS_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Glyph quads land on whole device pixels, so sampling must not blur them.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gyeol atlas layout"),
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
        let atlas_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gyeol atlas group"),
            layout: &atlas_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gyeol text shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("text.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gyeol text layout"),
            bind_group_layouts: &[Some(globals_layout), Some(&atlas_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gyeol text pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GlyphInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4
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
        });

        TextSystem {
            swash: SwashCache::new(),
            atlas: Atlas { texture, x: 0, y: 0, row_height: 0 },
            glyphs: HashMap::new(),
            pipeline,
            atlas_group,
            instances: GrowBuffer::new(device, wgpu::BufferUsages::VERTEX),
            count: 0,
        }
    }

    /// Rasterizes glyphs not yet in the atlas and uploads this frame's glyph quads.
    pub fn prepare(&mut self, shaper: &mut Shaper, device: &wgpu::Device, queue: &wgpu::Queue, texts: &[Text], scale: f32) {
        let mut out: Vec<GlyphInstance> = Vec::new();

        for text in texts {
            let (buffer, font_system) = shaper.buffer_and_fonts(&text.content, text.size);
            for run in buffer.layout_runs() {
                // The baseline sits on a whole device pixel so glyphs stay crisp.
                let baseline = ((text.origin.1 + run.line_y) * scale).round();
                for glyph in run.glyphs {
                    let physical = glyph.physical((text.origin.0 * scale, baseline), scale);
                    let Some(slot) =
                        slot_for(physical.cache_key, &mut self.glyphs, &mut self.swash, font_system, &mut self.atlas, queue)
                    else {
                        continue;
                    };
                    out.push(GlyphInstance {
                        pos: [(physical.x + slot.left) as f32 / scale, (physical.y - slot.top) as f32 / scale],
                        size: [slot.size[0] / scale, slot.size[1] / scale],
                        uv_pos: slot.uv_pos,
                        uv_size: slot.uv_size,
                        color: text.color.to_array(),
                    });
                }
            }
        }

        self.count = out.len() as u32;
        self.instances.write(device, queue, bytemuck::cast_slice(&out));
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, globals: &wgpu::BindGroup) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, globals, &[]);
        pass.set_bind_group(1, &self.atlas_group, &[]);
        pass.set_vertex_buffer(0, self.instances.buffer.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

/// The atlas slot for a glyph, rasterizing and uploading it on first use. `None` for glyphs with no
/// coverage bitmap (spaces, and color glyphs such as emoji, which are not drawn yet).
fn slot_for(
    key: CacheKey,
    glyphs: &mut HashMap<CacheKey, Option<GlyphSlot>>,
    swash: &mut SwashCache,
    font_system: &mut FontSystem,
    atlas: &mut Atlas,
    queue: &wgpu::Queue,
) -> Option<GlyphSlot> {
    if let Some(slot) = glyphs.get(&key) {
        return *slot;
    }
    let slot = swash.get_image(font_system, key).as_ref().and_then(|image| {
        let (w, h) = (image.placement.width, image.placement.height);
        if image.content != SwashContent::Mask || w == 0 || h == 0 {
            return None;
        }
        let (x, y) = atlas.allocate(w, h)?;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &image.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let s = ATLAS_SIZE as f32;
        Some(GlyphSlot {
            uv_pos: [x as f32 / s, y as f32 / s],
            uv_size: [w as f32 / s, h as f32 / s],
            size: [w as f32, h as f32],
            left: image.placement.left,
            top: image.placement.top,
        })
    });
    glyphs.insert(key, slot);
    slot
}
