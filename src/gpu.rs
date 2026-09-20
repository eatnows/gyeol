//! Small wgpu helpers shared by the renderer's pipelines.
use bytemuck::{Pod, Zeroable};

/// Per-frame uniforms: the drawable size in logical pixels.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Globals {
    pub viewport: [f32; 2],
    pub _pad: [f32; 2],
}

/// A GPU buffer that grows (never shrinks) to fit what is written to it each frame.
pub(crate) struct GrowBuffer {
    pub buffer: wgpu::Buffer,
    capacity: u64,
    usage: wgpu::BufferUsages,
}

impl GrowBuffer {
    pub fn new(device: &wgpu::Device, usage: wgpu::BufferUsages) -> Self {
        let capacity = 4096;
        GrowBuffer { buffer: Self::alloc(device, capacity, usage), capacity, usage }
    }

    fn alloc(device: &wgpu::Device, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gyui buffer"),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) {
        let needed = bytes.len() as u64;
        if needed > self.capacity {
            self.capacity = needed.next_power_of_two();
            self.buffer = Self::alloc(device, self.capacity, self.usage);
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytes);
        }
    }
}
