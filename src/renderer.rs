//! Draws a [`Scene`] to a window with wgpu.
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use winit::{dpi::PhysicalSize, window::Window};

use crate::{
    error::{err, Error, Result},
    gpu::{Globals, GrowBuffer},
    scene::{Quad, Scene},
    text::TextSystem,
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct QuadInstance {
    pos: [f32; 2],
    size: [f32; 2],
    background: [f32; 4],
    border_color: [f32; 4],
    params: [f32; 2],
}

impl From<&Quad> for QuadInstance {
    fn from(q: &Quad) -> Self {
        QuadInstance {
            pos: [q.bounds.x, q.bounds.y],
            size: [q.bounds.w, q.bounds.h],
            background: q.background.to_array(),
            border_color: q.border_color.to_array(),
            params: [q.border_width, q.corner_radius],
        }
    }
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

    pub fn render(&mut self, scene: &Scene) {
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
        let quads: Vec<QuadInstance> = scene.quads.iter().map(QuadInstance::from).collect();
        self.quad_instances.write(&self.device, &self.queue, bytemuck::cast_slice(&quads));

        self.text.prepare(&self.device, &self.queue, &scene.texts, self.scale_factor);

        let clear = scene.background.map_or(wgpu::Color::BLACK, |c| wgpu::Color {
            r: c.r as f64,
            g: c.g as f64,
            b: c.b as f64,
            a: c.a as f64,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gyeol frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gyeol pass"),
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
            if !quads.is_empty() {
                pass.set_pipeline(&self.quad_pipeline);
                pass.set_bind_group(0, &self.globals_group, &[]);
                pass.set_vertex_buffer(0, self.quad_instances.buffer.slice(..));
                pass.draw(0..6, 0..quads.len() as u32);
            }
            self.text.draw(&mut pass, &self.globals_group);
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
    }
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
                    0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32x2
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
