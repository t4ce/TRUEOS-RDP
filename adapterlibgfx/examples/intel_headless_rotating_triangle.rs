use futures::executor::block_on;
use wgpu::util::DeviceExt;

const SHADER: &str = r#"
struct In {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: In) -> Out {
    var out: Out;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

impl Vertex {
    const ATTRS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: core::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}

fn main() {
    block_on(run());
}

async fn run() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = instance
        .enumerate_adapters(wgpu::Backends::VULKAN)
        .into_iter()
        .find(|adapter| adapter.get_info().vendor == 0x8086)
        .expect("no Intel Vulkan adapter found");
    let info = adapter.get_info();
    eprintln!(
        "selected: backend={:?} vendor=0x{:04X} device=0x{:04X} name={}",
        info.backend, info.vendor, info.device, info.name
    );

    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor {
                label: Some("adapterlibgfx-rotating-intel-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        )
        .await
        .expect("request_device failed");

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("adapterlibgfx-rotating-intel-shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("adapterlibgfx-rotating-intel-pipeline-layout"),
        bind_group_layouts: &[],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("adapterlibgfx-rotating-intel-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Vertex::layout()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("adapterlibgfx-rotating-intel-target"),
        size: wgpu::Extent3d {
            width: 512,
            height: 512,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let readback_pitch = 512usize * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("adapterlibgfx-rotating-intel-visible-readback-rgba"),
        size: (readback_pitch * 512) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    for frame in 0..3 {
        let radians = frame as f32 * 0.55;
        let vertices = rotated_triangle(radians);
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("adapterlibgfx-rotating-intel-vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("adapterlibgfx-rotating-intel-frame"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("adapterlibgfx-rotating-intel-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02 + frame as f64 * 0.015,
                            g: 0.025,
                            b: 0.045,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(readback_pitch as u32),
                    rows_per_image: Some(512),
                },
            },
            wgpu::Extent3d {
                width: 512,
                height: 512,
                depth_or_array_layers: 1,
            },
        );

        queue.submit(Some(encoder.finish()));
        let _ = device.poll(wgpu::Maintain::Wait);
        let slice = readback.slice(..);
        let (tx, rx) = futures::channel::oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        let _ = device.poll(wgpu::Maintain::Wait);
        rx.await
            .expect("readback map callback dropped")
            .expect("readback map failed");
        let mapped = slice.get_mapped_range();
        let center_off = 256usize * readback_pitch + 256usize * 4;
        let center = &mapped[center_off..][..4];
        eprintln!(
            "intel rotating triangle frame {frame} submitted angle={radians:.3} center_rgba={:02X}{:02X}{:02X}{:02X}",
            center[0], center[1], center[2], center[3]
        );
        drop(mapped);
        readback.unmap();
    }
}

fn rotated_triangle(radians: f32) -> [Vertex; 3] {
    let base = [[-0.72, -0.62], [0.72, -0.62], [0.0, 0.76]];
    let colors = [
        [1.0, 0.35, 0.15, 1.0],
        [0.1, 0.8, 0.55, 1.0],
        [0.35, 0.55, 1.0, 1.0],
    ];
    let c = radians.cos();
    let s = radians.sin();
    core::array::from_fn(|index| {
        let [x, y] = base[index];
        Vertex {
            position: [x * c - y * s, x * s + y * c],
            color: colors[index],
        }
    })
}
