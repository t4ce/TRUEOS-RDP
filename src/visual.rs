use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// A newly decoded image crossing the media-to-presentation boundary.
///
/// Presentation owns the pixels. The receiver is therefore free to replace its
/// latest-frame slot immediately after handing an image to the window thread.
#[derive(Debug)]
pub struct VisualImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One presentation request. `image` is absent when only the local overlay has
/// changed; the presenter retains and reuses the last uploaded video texture.
#[derive(Debug)]
pub struct VisualPresentation {
    pub image: Option<VisualImage>,
    /// Video bounds in clip coordinates: left, top, right, bottom.
    pub video_bounds: [f32; 4],
    /// Triangle-list vertices encoded as f32 x/y followed by RGBA8.
    pub overlay_vertices: Vec<u8>,
    /// A newly stamped telemetry image. The prior image is retained when absent.
    pub telemetry: Option<VisualImage>,
    pub telemetry_visible: bool,
    /// Telemetry bounds in clip coordinates: left, top, right, bottom.
    pub telemetry_bounds: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VideoVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

struct VideoTexture {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

/// GPU implementation of the connected visual receiver. This is intentionally
/// independent from the adapter command interpreter used by local UI frames.
pub struct VisualPresenter {
    video_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    video_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    video_texture: Option<VideoTexture>,
    telemetry_texture: Option<VideoTexture>,
}

impl VisualPresenter {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let video_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("trueos-rdp visual texture layout"),
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
        let video_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("trueos-rdp visual video shader"),
            source: wgpu::ShaderSource::Wgsl(VIDEO_SHADER.into()),
        });
        let video_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("trueos-rdp visual video pipeline layout"),
                bind_group_layouts: &[Some(&video_layout)],
                immediate_size: 0,
            });
        let video_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("trueos-rdp visual video pipeline"),
            layout: Some(&video_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &video_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: core::mem::size_of::<VideoVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &video_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let overlay_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("trueos-rdp visual overlay shader"),
            source: wgpu::ShaderSource::Wgsl(OVERLAY_SHADER.into()),
        });
        let overlay_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("trueos-rdp visual overlay pipeline layout"),
                bind_group_layouts: &[],
                immediate_size: 0,
            });
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("trueos-rdp visual overlay pipeline"),
            layout: Some(&overlay_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &overlay_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Unorm8x4],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &overlay_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("trueos-rdp visual sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            video_pipeline,
            overlay_pipeline,
            video_layout,
            sampler,
            video_texture: None,
            telemetry_texture: None,
        }
    }

    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        presentation: VisualPresentation,
    ) -> Option<wgpu::CommandBuffer> {
        if let Some(image) = presentation.image {
            upload_image(
                device,
                queue,
                &self.video_layout,
                &self.sampler,
                &mut self.video_texture,
                image,
                "trueos-rdp decoded visual",
            )?;
        }
        if !presentation.telemetry_visible {
            self.telemetry_texture = None;
        } else if let Some(image) = presentation.telemetry {
            upload_image(
                device,
                queue,
                &self.video_layout,
                &self.sampler,
                &mut self.telemetry_texture,
                image,
                "trueos-rdp telemetry",
            )?;
        }
        let video_texture = self.video_texture.as_ref()?;
        let [left, top, right, bottom] = presentation.video_bounds;
        let video_vertices = [
            video_vertex(left, top, 0.0, 0.0),
            video_vertex(right, top, 1.0, 0.0),
            video_vertex(right, bottom, 1.0, 1.0),
            video_vertex(left, top, 0.0, 0.0),
            video_vertex(right, bottom, 1.0, 1.0),
            video_vertex(left, bottom, 0.0, 1.0),
        ];
        let video_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("trueos-rdp visual video vertices"),
            contents: bytemuck::cast_slice(&video_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let telemetry_buffer = self.telemetry_texture.as_ref().map(|_| {
            let [left, top, right, bottom] = presentation.telemetry_bounds;
            let vertices = [
                video_vertex(left, top, 0.0, 0.0),
                video_vertex(right, top, 1.0, 0.0),
                video_vertex(right, bottom, 1.0, 1.0),
                video_vertex(left, top, 0.0, 0.0),
                video_vertex(right, bottom, 1.0, 1.0),
                video_vertex(left, bottom, 0.0, 1.0),
            ];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("trueos-rdp telemetry vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let overlay_len = presentation.overlay_vertices.len() / 12;
        let overlay_buffer = (overlay_len != 0).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("trueos-rdp visual overlay vertices"),
                contents: &presentation.overlay_vertices[..overlay_len * 12],
                usage: wgpu::BufferUsages::VERTEX,
            })
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("trueos-rdp visual encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("trueos-rdp visual pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.video_pipeline);
            pass.set_bind_group(0, &video_texture.bind_group, &[]);
            pass.set_vertex_buffer(0, video_buffer.slice(..));
            pass.draw(0..6, 0..1);
            if let (Some(texture), Some(buffer)) = (&self.telemetry_texture, &telemetry_buffer) {
                pass.set_bind_group(0, &texture.bind_group, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..6, 0..1);
            }
            if let Some(overlay_buffer) = &overlay_buffer {
                pass.set_pipeline(&self.overlay_pipeline);
                pass.set_vertex_buffer(0, overlay_buffer.slice(..));
                pass.draw(0..overlay_len as u32, 0..1);
            }
        }
        Some(encoder.finish())
    }
}

fn upload_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    slot: &mut Option<VideoTexture>,
    image: VisualImage,
    label: &'static str,
) -> Option<()> {
    let expected = image.width as usize * image.height as usize * 4;
    if image.width == 0 || image.height == 0 || image.rgba.len() != expected {
        return None;
    }
    let needs_texture = slot
        .as_ref()
        .is_none_or(|texture| texture.width != image.width || texture.height != image.height);
    if needs_texture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        *slot = Some(VideoTexture {
            texture,
            bind_group,
            width: image.width,
            height: image.height,
        });
    }
    let texture = slot.as_ref()?;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &image.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width * 4),
            rows_per_image: Some(image.height),
        },
        wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
    );
    Some(())
}

fn video_vertex(x: f32, y: f32, u: f32, v: f32) -> VideoVertex {
    VideoVertex {
        position: [x, y],
        uv: [u, v],
    }
}

const VIDEO_SHADER: &str = r#"
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@group(0) @binding(0) var video: texture_2d<f32>;
@group(0) @binding(1) var video_sampler: sampler;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(video, video_sampler, in.uv);
}
"#;

const OVERLAY_SHADER: &str = r#"
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) color: vec4<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;
