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

    let adapters = instance.enumerate_adapters(wgpu::Backends::VULKAN);
    for adapter in &adapters {
        let info = adapter.get_info();
        eprintln!(
            "adapter: backend={:?} vendor=0x{:04X} device=0x{:04X} type={:?} name={}",
            info.backend, info.vendor, info.device, info.device_type, info.name
        );
    }

    let adapter = adapters
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
                label: Some("adapterlibgfx-intel-headless-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        )
        .await
        .expect("request_device failed");

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("adapterlibgfx-intel-headless-shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("adapterlibgfx-intel-headless-pipeline-layout"),
        bind_group_layouts: &[],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("adapterlibgfx-intel-headless-pipeline"),
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

    let vertices = [
        Vertex {
            position: [-0.75, -0.70],
            color: [1.0, 0.35, 0.15, 1.0],
        },
        Vertex {
            position: [0.75, -0.65],
            color: [0.1, 0.8, 0.55, 1.0],
        },
        Vertex {
            position: [0.0, 0.75],
            color: [0.35, 0.55, 1.0, 1.0],
        },
    ];
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("adapterlibgfx-intel-headless-vertices"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("adapterlibgfx-intel-headless-target"),
        size: wgpu::Extent3d {
            width: 512,
            height: 512,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("adapterlibgfx-intel-headless-frame"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("adapterlibgfx-intel-headless-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.03,
                        g: 0.035,
                        b: 0.05,
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

    queue.submit(Some(encoder.finish()));
    let _ = device.poll(wgpu::Maintain::Wait);
    eprintln!("intel headless frame submitted");
}
