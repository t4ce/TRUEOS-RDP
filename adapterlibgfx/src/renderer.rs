use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use futures::executor::block_on;
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::command::{
    BlendFactor, BlendState, Frame, FrameCommand, SamplerFilter, SamplerState, SamplerWrap,
    ScissorRect, TextureEffect, TextureSampleKind,
};
use crate::texture::{TextureImage, TextureRegistry};
use crate::vertex::{GpuRgbVertex, GpuTexVertex, RgbVertex, TexVertex};

const SHADER_COMMON: &str = r#"
struct RgbIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct RgbOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

struct FrameParams {
    data: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame_params: FrameParams;

@vertex
fn rgb_vs(in: RgbIn) -> RgbOut {
    var out: RgbOut;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn rgb_fs(in: RgbOut) -> @location(0) vec4<f32> {
    return in.color;
}

struct TexIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct TexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@group(1) @binding(0) var tex_sampler: sampler;
@group(1) @binding(1) var tex_image: texture_2d<f32>;

@vertex
fn tex_vs(in: TexIn) -> TexOut {
    var out: TexOut;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

@fragment
fn tex_mask_fs(in: TexOut) -> @location(0) vec4<f32> {
    let tex = textureSample(tex_image, tex_sampler, in.uv);
    let mask = select(tex.r, tex.a, tex.a < 1.0);
    return vec4<f32>(in.color.rgb * mask, in.color.a * mask);
}
"#;

#[cfg(not(feature = "shaderson"))]
const SHADER_TEX_FRAGMENT: &str = r#"
@fragment
fn world_rgb_fs(in: RgbOut) -> @location(0) vec4<f32> {
    return in.color;
}

@fragment
fn tex_fs(in: TexOut) -> @location(0) vec4<f32> {
    return textureSample(tex_image, tex_sampler, in.uv) * in.color;
}

@fragment
fn world_tex_fs(in: TexOut) -> @location(0) vec4<f32> {
    return textureSample(tex_image, tex_sampler, in.uv) * in.color;
}

@fragment
fn blur_tex_fs(in: TexOut) -> @location(0) vec4<f32> {
    let dims = max(vec2<f32>(textureDimensions(tex_image, 0u)), vec2<f32>(1.0, 1.0));
    let texel = 1.0 / dims;
    let center = textureSample(tex_image, tex_sampler, in.uv) * 4.0;
    let cardinal =
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>(-texel.x, 0.0)) * 2.0 +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>( texel.x, 0.0)) * 2.0 +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>(0.0, -texel.y)) * 2.0 +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>(0.0,  texel.y)) * 2.0;
    let diagonal =
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>(-texel.x, -texel.y)) +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>( texel.x, -texel.y)) +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>(-texel.x,  texel.y)) +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>( texel.x,  texel.y));
    return ((center + cardinal + diagonal) / 16.0) * in.color;
}
"#;

#[cfg(feature = "shaderson")]
const SHADER_TEX_FRAGMENT: &str = r#"
fn tex_alpha(uv: vec2<f32>) -> f32 {
    return textureSample(tex_image, tex_sampler, uv).a;
}

fn frame_time() -> f32 {
    return frame_params.data.x;
}

fn hash21(pixel: vec2<f32>) -> f32 {
    return fract(sin(dot(pixel, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn day_night_tint(color: vec4<f32>) -> vec4<f32> {
    let cycle = fract(frame_time() / 180.0);
    let night = smoothstep(0.48, 0.72, cycle) * (1.0 - smoothstep(0.88, 1.0, cycle));
    let dawn = smoothstep(0.88, 1.0, cycle) + (1.0 - smoothstep(0.0, 0.12, cycle));
    let dusk = smoothstep(0.38, 0.52, cycle) * (1.0 - smoothstep(0.52, 0.66, cycle));
    var rgb = color.rgb;
    rgb = mix(rgb, rgb * vec3<f32>(0.58, 0.66, 0.88) + vec3<f32>(0.010, 0.018, 0.045), night * 0.42);
    rgb = mix(rgb, rgb * vec3<f32>(1.08, 0.94, 0.78) + vec3<f32>(0.030, 0.018, 0.004), (dawn + dusk) * 0.16);
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), color.a);
}

@fragment
fn world_rgb_fs(in: RgbOut) -> @location(0) vec4<f32> {
    return day_night_tint(in.color);
}

@fragment
fn tex_fs(in: TexOut) -> @location(0) vec4<f32> {
    let base = textureSample(tex_image, tex_sampler, in.uv) * in.color;
    return base;
}

@fragment
fn world_tex_fs(in: TexOut) -> @location(0) vec4<f32> {
    let base = textureSample(tex_image, tex_sampler, in.uv) * in.color;
    let dims = max(vec2<f32>(textureDimensions(tex_image, 0u)), vec2<f32>(1.0, 1.0));
    let texel = 1.0 / dims;

    let left = tex_alpha(in.uv - vec2<f32>(texel.x, 0.0));
    let right = tex_alpha(in.uv + vec2<f32>(texel.x, 0.0));
    let up = tex_alpha(in.uv - vec2<f32>(0.0, texel.y));
    let down = tex_alpha(in.uv + vec2<f32>(0.0, texel.y));
    let neighbor_max = max(max(left, right), max(up, down));

    if base.a < 0.02 {
        if neighbor_max > 0.20 {
            return vec4<f32>(0.015, 0.018, 0.026, neighbor_max * in.color.a * 0.34);
        }
        return base;
    }

    let coverage = smoothstep(0.02, 0.20, base.a);
    let luma = dot(base.rgb, vec3<f32>(0.299, 0.587, 0.114));
    let pixel = floor(in.position.xy);
    let foliage = smoothstep(0.03, 0.22, base.g - max(base.r, base.b));
    let gold = smoothstep(0.08, 0.34, base.r - base.b)
        * smoothstep(0.00, 0.22, base.g - base.b)
        * smoothstep(0.34, 0.78, luma);

    let local_y = fract(in.uv.y * dims.y);
    let tile_band = smoothstep(0.0, 0.55, local_y) * (1.0 - smoothstep(0.72, 1.0, local_y));
    let top_light = max(0.0, 1.0 - min(left, up)) * coverage;
    let foot_shadow = max(0.0, 1.0 - min(right, down)) * coverage;
    let alpha_slope = clamp((right + down - left - up) * 0.38, -0.22, 0.22);

    let gray = vec3<f32>(luma);
    var rgb = gray + (base.rgb - gray) * (1.05 + foliage * 0.08 + gold * 0.10);
    rgb += vec3<f32>(1.0, 0.86, 0.52) * top_light * (0.08 + gold * 0.12);
    rgb -= vec3<f32>(0.025, 0.040, 0.075) * foot_shadow * (0.30 + foliage * 0.16);
    rgb += vec3<f32>(0.030, 0.050, 0.020) * foliage * tile_band;
    rgb *= 1.0 + alpha_slope;

    rgb += vec3<f32>(1.0, 0.82, 0.30) * gold * step(0.965, hash21(pixel)) * 0.20;

    let grain = hash21(pixel) - 0.5;
    let dither = step(0.5, fract((pixel.x + pixel.y) * 0.5)) * 0.018 - 0.009;
    rgb *= 0.995 + grain * 0.018 + dither;

    return day_night_tint(vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), base.a));
}

@fragment
fn blur_tex_fs(in: TexOut) -> @location(0) vec4<f32> {
    let dims = max(vec2<f32>(textureDimensions(tex_image, 0u)), vec2<f32>(1.0, 1.0));
    let texel = 1.0 / dims;
    let center = textureSample(tex_image, tex_sampler, in.uv) * 4.0;
    let cardinal =
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>(-texel.x, 0.0)) * 2.0 +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>( texel.x, 0.0)) * 2.0 +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>(0.0, -texel.y)) * 2.0 +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>(0.0,  texel.y)) * 2.0;
    let diagonal =
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>(-texel.x, -texel.y)) +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>( texel.x, -texel.y)) +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>(-texel.x,  texel.y)) +
        textureSample(tex_image, tex_sampler, in.uv + vec2<f32>( texel.x,  texel.y));
    return ((center + cardinal + diagonal) / 16.0) * in.color;
}
"#;

#[derive(Debug)]
pub enum RenderError {
    AdapterUnavailable,
    DeviceUnavailable(String),
    Surface(String),
    MissingTexture(u32),
    InvalidTarget(u32),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum PipelineKind {
    Rgb,
    Tex,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PipelineKey {
    kind: PipelineKind,
    texture_effect: TextureEffect,
    sample_kind: TextureSampleKind,
    blend: BlendState,
    format: wgpu::TextureFormat,
}

struct GpuTexture {
    width: u32,
    height: u32,
    revision: u64,
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

pub struct WgpuRenderer {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    shader: wgpu::ShaderModule,
    started_at: Instant,
    frame_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    textures: HashMap<u32, GpuTexture>,
    sampler: SamplerState,
    texture_effect: TextureEffect,
    blend: BlendState,
    scissor: Option<ScissorRect>,
}

pub struct WgpuHeadlessRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    _target: wgpu::Texture,
    target_view: wgpu::TextureView,
    shader: wgpu::ShaderModule,
    started_at: Instant,
    frame_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    textures: HashMap<u32, GpuTexture>,
    sampler: SamplerState,
    texture_effect: TextureEffect,
    blend: BlendState,
    scissor: Option<ScissorRect>,
}

impl WgpuRenderer {
    pub fn new(window: Arc<Window>) -> Result<Self, RenderError> {
        block_on(Self::new_async(window))
    }

    pub async fn new_async(window: Arc<Window>) -> Result<Self, RenderError> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|err| RenderError::Surface(err.to_string()))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .ok_or(RenderError::AdapterUnavailable)?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("adapterlibgfx-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(|err| RenderError::DeviceUnavailable(err.to_string()))?;
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or(RenderError::AdapterUnavailable)?;
        config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        surface.configure(&device, &config);

        let shader_source = [SHADER_COMMON, SHADER_TEX_FRAGMENT].concat();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("adapterlibgfx-shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("adapterlibgfx-frame-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("adapterlibgfx-texture-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            shader,
            started_at: Instant::now(),
            frame_layout,
            texture_layout,
            pipelines: HashMap::new(),
            textures: HashMap::new(),
            sampler: SamplerState::default(),
            texture_effect: TextureEffect::default(),
            blend: BlendState::default(),
            scissor: None,
        })
    }

    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        let width = size.width.max(1);
        let height = size.height.max(1);
        if self.config.width == width && self.config.height == height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn render_frame(
        &mut self,
        textures: &TextureRegistry,
        frame: &Frame,
    ) -> Result<(), RenderError> {
        self.sync_textures(textures);
        let surface_texture = if frame.allow_present {
            match self.surface.get_current_texture() {
                Ok(texture) => Some(texture),
                Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                    self.surface.configure(&self.device, &self.config);
                    Some(
                        self.surface
                            .get_current_texture()
                            .map_err(|err| RenderError::Surface(err.to_string()))?,
                    )
                }
                Err(wgpu::SurfaceError::Timeout) => return Ok(()),
                Err(err) => return Err(RenderError::Surface(err.to_string())),
            }
        } else {
            None
        };
        let surface_view = surface_texture.as_ref().map(|texture| {
            texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default())
        });

        self.sampler = SamplerState::default();
        self.texture_effect = TextureEffect::default();
        self.blend = BlendState::default();
        self.scissor = None;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("adapterlibgfx-frame"),
            });
        let frame_params = [
            self.started_at.elapsed().as_secs_f32(),
            frame.logical_width.max(1) as f32,
            frame.logical_height.max(1) as f32,
            0.0,
        ];
        let frame_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("adapterlibgfx-frame-params"),
                contents: bytemuck::cast_slice(&frame_params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let frame_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("adapterlibgfx-frame-bind-group"),
            layout: &self.frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        let mut current_target = 0u32;
        let mut cleared_targets = HashSet::new();

        if frame.commands.is_empty() && frame.allow_present {
            if let Some(view) = surface_view.as_ref() {
                self.clear_target(&mut encoder, view, frame.clear_rgb);
            }
        }

        for command in &frame.commands {
            match command {
                FrameCommand::SetBlend(blend) => self.blend = *blend,
                FrameCommand::SetSampler(sampler) => self.sampler = *sampler,
                FrameCommand::SetTextureEffect(effect) => self.texture_effect = *effect,
                FrameCommand::SetScissor(scissor) => {
                    self.scissor = if current_target == 0 {
                        scissor.map(|rect| self.clip_surface_scissor(rect))
                    } else {
                        *scissor
                    };
                }
                FrameCommand::SetRenderTarget(tex_id) => {
                    current_target = *tex_id;
                    self.scissor = None;
                }
                FrameCommand::DrawRgb { vertices } => {
                    let Some(view) = self.target_view(current_target, surface_view.as_ref())?
                    else {
                        continue;
                    };
                    let format = self.target_format(current_target);
                    let load = self.load_op_for_target(&mut cleared_targets, current_target, frame);
                    if current_target == 0 {
                        let vertices = self.surface_rgb_vertices(vertices, frame);
                        self.draw_rgb(
                            &mut encoder,
                            &view,
                            format,
                            load,
                            &frame_bind_group,
                            &vertices,
                        );
                    } else {
                        self.draw_rgb(
                            &mut encoder,
                            &view,
                            format,
                            load,
                            &frame_bind_group,
                            vertices,
                        );
                    }
                }
                FrameCommand::DrawTex {
                    tex_id,
                    sample_kind,
                    vertices,
                } => {
                    let Some(view) = self.target_view(current_target, surface_view.as_ref())?
                    else {
                        continue;
                    };
                    let Some(source) = self.textures.get(tex_id) else {
                        return Err(RenderError::MissingTexture(*tex_id));
                    };
                    let source_view = source.view.clone();
                    let format = self.target_format(current_target);
                    let load = self.load_op_for_target(&mut cleared_targets, current_target, frame);
                    if current_target == 0 {
                        let vertices = self.surface_tex_vertices(vertices, frame);
                        self.draw_tex(
                            &mut encoder,
                            &view,
                            format,
                            load,
                            &frame_bind_group,
                            &source_view,
                            *sample_kind,
                            &vertices,
                        );
                    } else {
                        self.draw_tex(
                            &mut encoder,
                            &view,
                            format,
                            load,
                            &frame_bind_group,
                            &source_view,
                            *sample_kind,
                            vertices,
                        );
                    }
                }
            }
        }

        self.queue.submit(Some(encoder.finish()));
        if let Some(texture) = surface_texture {
            texture.present();
        }
        Ok(())
    }

    fn sync_textures(&mut self, textures: &TextureRegistry) {
        for (&tex_id, image) in textures.iter() {
            let needs_upload = self
                .textures
                .get(&tex_id)
                .map(|gpu| {
                    gpu.revision != image.revision
                        || gpu.width != image.width
                        || gpu.height != image.height
                })
                .unwrap_or(true);
            if needs_upload {
                let gpu = self.create_or_upload_texture(tex_id, image);
                self.textures.insert(tex_id, gpu);
            }
        }
    }

    fn create_or_upload_texture(&self, tex_id: u32, image: &TextureImage) -> GpuTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("adapterlibgfx-texture"),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
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
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let _ = tex_id;
        GpuTexture {
            width: image.width,
            height: image.height,
            revision: image.revision,
            _texture: texture,
            view,
        }
    }

    fn target_view(
        &self,
        tex_id: u32,
        surface_view: Option<&wgpu::TextureView>,
    ) -> Result<Option<wgpu::TextureView>, RenderError> {
        if tex_id == 0 {
            Ok(surface_view.cloned())
        } else {
            self.textures
                .get(&tex_id)
                .map(|texture| Some(texture.view.clone()))
                .ok_or(RenderError::InvalidTarget(tex_id))
        }
    }

    fn load_op_for_target(
        &self,
        cleared_targets: &mut HashSet<u32>,
        tex_id: u32,
        frame: &Frame,
    ) -> wgpu::LoadOp<wgpu::Color> {
        if frame.preserve_contents || !cleared_targets.insert(tex_id) {
            wgpu::LoadOp::Load
        } else {
            wgpu::LoadOp::Clear(clear_color(frame.clear_rgb))
        }
    }

    fn target_format(&self, tex_id: u32) -> wgpu::TextureFormat {
        if tex_id == 0 {
            self.config.format
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        }
    }

    fn clip_surface_scissor(&self, rect: crate::command::ScissorRect) -> crate::command::ScissorRect {
        let x = rect.x.min(self.config.width.saturating_sub(1));
        let y = rect.y.min(self.config.height.saturating_sub(1));
        crate::command::ScissorRect {
            x,
            y,
            width: rect.width.min(self.config.width.saturating_sub(x)).max(1),
            height: rect.height.min(self.config.height.saturating_sub(y)).max(1),
        }
    }

    fn surface_rgb_vertices(&self, vertices: &[RgbVertex], frame: &Frame) -> Vec<RgbVertex> {
        vertices
            .iter()
            .copied()
            .map(|mut vertex| {
                vertex.x = self.logical_clip_x_to_surface_clip(vertex.x, frame);
                vertex.y = self.logical_clip_y_to_surface_clip(vertex.y, frame);
                vertex
            })
            .collect()
    }

    fn surface_tex_vertices(&self, vertices: &[TexVertex], frame: &Frame) -> Vec<TexVertex> {
        vertices
            .iter()
            .copied()
            .map(|mut vertex| {
                vertex.x = self.logical_clip_x_to_surface_clip(vertex.x, frame);
                vertex.y = self.logical_clip_y_to_surface_clip(vertex.y, frame);
                vertex
            })
            .collect()
    }

    fn logical_clip_x_to_surface_clip(&self, x: f32, frame: &Frame) -> f32 {
        let logical_x = (x + 1.0) * 0.5 * frame.logical_width.max(1) as f32;
        logical_x / self.config.width.max(1) as f32 * 2.0 - 1.0
    }

    fn logical_clip_y_to_surface_clip(&self, y: f32, frame: &Frame) -> f32 {
        let logical_y = (1.0 - y) * 0.5 * frame.logical_height.max(1) as f32;
        1.0 - logical_y / self.config.height.max(1) as f32 * 2.0
    }

    fn clear_target(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        clear_rgb: u32,
    ) {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("adapterlibgfx-clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color(clear_rgb)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    fn draw_rgb(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        load: wgpu::LoadOp<wgpu::Color>,
        frame_bind_group: &wgpu::BindGroup,
        vertices: &[crate::vertex::RgbVertex],
    ) {
        if vertices.is_empty() {
            return;
        }
        let gpu_vertices = vertices
            .iter()
            .copied()
            .map(GpuRgbVertex::from)
            .collect::<Vec<_>>();
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("adapterlibgfx-rgb-vertices"),
                contents: bytemuck::cast_slice(&gpu_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let pipeline = self
            .pipeline(PipelineKind::Rgb, TextureSampleKind::Rgba, format)
            .clone();
        let mut pass = self.begin_draw_pass(encoder, view, load);
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, frame_bind_group, &[]);
        if let Some(scissor) = self.scissor {
            pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        }
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..gpu_vertices.len() as u32, 0..1);
    }

    fn draw_tex(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        load: wgpu::LoadOp<wgpu::Color>,
        frame_bind_group: &wgpu::BindGroup,
        source_view: &wgpu::TextureView,
        sample_kind: TextureSampleKind,
        vertices: &[crate::vertex::TexVertex],
    ) {
        if vertices.is_empty() {
            return;
        }
        let gpu_vertices = vertices
            .iter()
            .copied()
            .map(GpuTexVertex::from)
            .collect::<Vec<_>>();
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("adapterlibgfx-tex-vertices"),
                contents: bytemuck::cast_slice(&gpu_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let sampler = self
            .device
            .create_sampler(&sampler_descriptor(self.sampler));
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("adapterlibgfx-texture-bind-group"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source_view),
                },
            ],
        });
        let pipeline = self
            .pipeline(PipelineKind::Tex, sample_kind, format)
            .clone();
        let mut pass = self.begin_draw_pass(encoder, view, load);
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, frame_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        if let Some(scissor) = self.scissor {
            pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        }
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..gpu_vertices.len() as u32, 0..1);
    }

    fn begin_draw_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        view: &'a wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("adapterlibgfx-draw"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        })
    }

    fn pipeline(
        &mut self,
        kind: PipelineKind,
        sample_kind: TextureSampleKind,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let key = PipelineKey {
            kind,
            texture_effect: self.texture_effect,
            sample_kind,
            blend: self.blend,
            format,
        };
        if !self.pipelines.contains_key(&key) {
            let pipeline = self.create_pipeline(key);
            self.pipelines.insert(key, pipeline);
        }
        self.pipelines.get(&key).unwrap()
    }

    fn create_pipeline(&self, key: PipelineKey) -> wgpu::RenderPipeline {
        let (vs, fs, buffers, layout) = match key.kind {
            PipelineKind::Rgb => (
                "rgb_vs",
                match key.texture_effect {
                    TextureEffect::Plain => "rgb_fs",
                    TextureEffect::World => "world_rgb_fs",
                    TextureEffect::Blur => "rgb_fs",
                },
                vec![GpuRgbVertex::layout()],
                self.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("adapterlibgfx-rgb-layout"),
                        bind_group_layouts: &[&self.frame_layout],
                        push_constant_ranges: &[],
                    }),
            ),
            PipelineKind::Tex => (
                "tex_vs",
                match key.sample_kind {
                    TextureSampleKind::Mask => "tex_mask_fs",
                    TextureSampleKind::Rgba => match key.texture_effect {
                        TextureEffect::Plain => "tex_fs",
                        TextureEffect::World => "world_tex_fs",
                        TextureEffect::Blur => "blur_tex_fs",
                    },
                },
                vec![GpuTexVertex::layout()],
                self.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("adapterlibgfx-tex-layout"),
                        bind_group_layouts: &[&self.frame_layout, &self.texture_layout],
                        push_constant_ranges: &[],
                    }),
            ),
        };
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("adapterlibgfx-pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some(vs),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some(fs),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: key.format,
                        blend: blend_state_to_wgpu(key.blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
    }
}

impl WgpuHeadlessRenderer {
    pub fn new_intel(width: u32, height: u32) -> Result<Self, RenderError> {
        block_on(Self::new_intel_async(width, height))
    }

    pub async fn new_intel_async(width: u32, height: u32) -> Result<Self, RenderError> {
        let width = width.max(1);
        let height = height.max(1);
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = instance
            .enumerate_adapters(wgpu::Backends::VULKAN)
            .into_iter()
            .find(|adapter| adapter.get_info().vendor == 0x8086)
            .ok_or(RenderError::AdapterUnavailable)?;
        let info = adapter.get_info();
        eprintln!(
            "adapterlibgfx headless selected: backend={:?} vendor=0x{:04X} device=0x{:04X} name={}",
            info.backend, info.vendor, info.device, info.name
        );
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("adapterlibgfx-headless-intel-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .map_err(|err| RenderError::DeviceUnavailable(err.to_string()))?;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let (target, target_view) = create_headless_target(&device, width, height, format);
        let shader_source = [SHADER_COMMON, SHADER_TEX_FRAGMENT].concat();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("adapterlibgfx-headless-shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("adapterlibgfx-headless-frame-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("adapterlibgfx-headless-texture-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        Ok(Self {
            device,
            queue,
            width,
            height,
            format,
            _target: target,
            target_view,
            shader,
            started_at: Instant::now(),
            frame_layout,
            texture_layout,
            pipelines: HashMap::new(),
            textures: HashMap::new(),
            sampler: SamplerState::default(),
            texture_effect: TextureEffect::default(),
            blend: BlendState::default(),
            scissor: None,
        })
    }

    pub fn render_frame(
        &mut self,
        textures: &TextureRegistry,
        frame: &Frame,
    ) -> Result<(), RenderError> {
        self.sync_textures(textures);
        self.sampler = SamplerState::default();
        self.texture_effect = TextureEffect::default();
        self.blend = BlendState::default();
        self.scissor = None;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("adapterlibgfx-headless-frame"),
            });
        let frame_params = [
            self.started_at.elapsed().as_secs_f32(),
            frame.logical_width.max(1) as f32,
            frame.logical_height.max(1) as f32,
            0.0,
        ];
        let frame_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("adapterlibgfx-headless-frame-params"),
                contents: bytemuck::cast_slice(&frame_params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let frame_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("adapterlibgfx-headless-frame-bind-group"),
            layout: &self.frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        let mut current_target = 0u32;
        let mut cleared_targets = HashSet::new();

        if frame.commands.is_empty() {
            let view = self.target_view.clone();
            self.clear_target(&mut encoder, &view, frame.clear_rgb);
        }

        for command in &frame.commands {
            match command {
                FrameCommand::SetBlend(blend) => self.blend = *blend,
                FrameCommand::SetSampler(sampler) => self.sampler = *sampler,
                FrameCommand::SetTextureEffect(effect) => self.texture_effect = *effect,
                FrameCommand::SetScissor(scissor) => {
                    self.scissor = if current_target == 0 {
                        scissor.map(|rect| self.clip_surface_scissor(rect))
                    } else {
                        *scissor
                    };
                }
                FrameCommand::SetRenderTarget(tex_id) => {
                    current_target = *tex_id;
                    self.scissor = None;
                }
                FrameCommand::DrawRgb { vertices } => {
                    let view = self.target_view(current_target)?;
                    let format = self.target_format(current_target);
                    let load = self.load_op_for_target(&mut cleared_targets, current_target, frame);
                    if current_target == 0 {
                        let vertices = self.surface_rgb_vertices(vertices, frame);
                        self.draw_rgb(&mut encoder, &view, format, load, &frame_bind_group, &vertices);
                    } else {
                        self.draw_rgb(&mut encoder, &view, format, load, &frame_bind_group, vertices);
                    }
                }
                FrameCommand::DrawTex {
                    tex_id,
                    sample_kind,
                    vertices,
                } => {
                    let view = self.target_view(current_target)?;
                    let Some(source) = self.textures.get(tex_id) else {
                        return Err(RenderError::MissingTexture(*tex_id));
                    };
                    let source_view = source.view.clone();
                    let format = self.target_format(current_target);
                    let load = self.load_op_for_target(&mut cleared_targets, current_target, frame);
                    if current_target == 0 {
                        let vertices = self.surface_tex_vertices(vertices, frame);
                        self.draw_tex(
                            &mut encoder,
                            &view,
                            format,
                            load,
                            &frame_bind_group,
                            &source_view,
                            *sample_kind,
                            &vertices,
                        );
                    } else {
                        self.draw_tex(
                            &mut encoder,
                            &view,
                            format,
                            load,
                            &frame_bind_group,
                            &source_view,
                            *sample_kind,
                            vertices,
                        );
                    }
                }
            }
        }

        self.queue.submit(Some(encoder.finish()));
        let _ = self.device.poll(wgpu::Maintain::Wait);
        Ok(())
    }

    fn sync_textures(&mut self, textures: &TextureRegistry) {
        for (&tex_id, image) in textures.iter() {
            let needs_upload = self
                .textures
                .get(&tex_id)
                .map(|gpu| {
                    gpu.revision != image.revision
                        || gpu.width != image.width
                        || gpu.height != image.height
                })
                .unwrap_or(true);
            if needs_upload {
                let gpu = self.create_or_upload_texture(tex_id, image);
                self.textures.insert(tex_id, gpu);
            }
        }
    }

    fn create_or_upload_texture(&self, tex_id: u32, image: &TextureImage) -> GpuTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("adapterlibgfx-headless-texture"),
            size: wgpu::Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
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
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let _ = tex_id;
        GpuTexture {
            width: image.width,
            height: image.height,
            revision: image.revision,
            _texture: texture,
            view,
        }
    }

    fn target_view(&self, tex_id: u32) -> Result<wgpu::TextureView, RenderError> {
        if tex_id == 0 {
            Ok(self.target_view.clone())
        } else {
            self.textures
                .get(&tex_id)
                .map(|texture| texture.view.clone())
                .ok_or(RenderError::InvalidTarget(tex_id))
        }
    }

    fn load_op_for_target(
        &self,
        cleared_targets: &mut HashSet<u32>,
        tex_id: u32,
        frame: &Frame,
    ) -> wgpu::LoadOp<wgpu::Color> {
        if frame.preserve_contents || !cleared_targets.insert(tex_id) {
            wgpu::LoadOp::Load
        } else {
            wgpu::LoadOp::Clear(clear_color(frame.clear_rgb))
        }
    }

    fn target_format(&self, tex_id: u32) -> wgpu::TextureFormat {
        if tex_id == 0 {
            self.format
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        }
    }

    fn clip_surface_scissor(&self, rect: crate::command::ScissorRect) -> crate::command::ScissorRect {
        let x = rect.x.min(self.width.saturating_sub(1));
        let y = rect.y.min(self.height.saturating_sub(1));
        crate::command::ScissorRect {
            x,
            y,
            width: rect.width.min(self.width.saturating_sub(x)).max(1),
            height: rect.height.min(self.height.saturating_sub(y)).max(1),
        }
    }

    fn surface_rgb_vertices(&self, vertices: &[RgbVertex], frame: &Frame) -> Vec<RgbVertex> {
        vertices
            .iter()
            .copied()
            .map(|mut vertex| {
                vertex.x = self.logical_clip_x_to_surface_clip(vertex.x, frame);
                vertex.y = self.logical_clip_y_to_surface_clip(vertex.y, frame);
                vertex
            })
            .collect()
    }

    fn surface_tex_vertices(&self, vertices: &[TexVertex], frame: &Frame) -> Vec<TexVertex> {
        vertices
            .iter()
            .copied()
            .map(|mut vertex| {
                vertex.x = self.logical_clip_x_to_surface_clip(vertex.x, frame);
                vertex.y = self.logical_clip_y_to_surface_clip(vertex.y, frame);
                vertex
            })
            .collect()
    }

    fn logical_clip_x_to_surface_clip(&self, x: f32, frame: &Frame) -> f32 {
        let logical_x = (x + 1.0) * 0.5 * frame.logical_width.max(1) as f32;
        logical_x / self.width.max(1) as f32 * 2.0 - 1.0
    }

    fn logical_clip_y_to_surface_clip(&self, y: f32, frame: &Frame) -> f32 {
        let logical_y = (1.0 - y) * 0.5 * frame.logical_height.max(1) as f32;
        1.0 - logical_y / self.height.max(1) as f32 * 2.0
    }

    fn clear_target(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        clear_rgb: u32,
    ) {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("adapterlibgfx-headless-clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color(clear_rgb)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    fn draw_rgb(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        load: wgpu::LoadOp<wgpu::Color>,
        frame_bind_group: &wgpu::BindGroup,
        vertices: &[crate::vertex::RgbVertex],
    ) {
        if vertices.is_empty() {
            return;
        }
        let gpu_vertices = vertices
            .iter()
            .copied()
            .map(GpuRgbVertex::from)
            .collect::<Vec<_>>();
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("adapterlibgfx-headless-rgb-vertices"),
                contents: bytemuck::cast_slice(&gpu_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let pipeline = self
            .pipeline(PipelineKind::Rgb, TextureSampleKind::Rgba, format)
            .clone();
        let mut pass = self.begin_draw_pass(encoder, view, load);
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, frame_bind_group, &[]);
        if let Some(scissor) = self.scissor {
            pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        }
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..gpu_vertices.len() as u32, 0..1);
    }

    fn draw_tex(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        load: wgpu::LoadOp<wgpu::Color>,
        frame_bind_group: &wgpu::BindGroup,
        source_view: &wgpu::TextureView,
        sample_kind: TextureSampleKind,
        vertices: &[crate::vertex::TexVertex],
    ) {
        if vertices.is_empty() {
            return;
        }
        let gpu_vertices = vertices
            .iter()
            .copied()
            .map(GpuTexVertex::from)
            .collect::<Vec<_>>();
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("adapterlibgfx-headless-tex-vertices"),
                contents: bytemuck::cast_slice(&gpu_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let sampler = self
            .device
            .create_sampler(&sampler_descriptor(self.sampler));
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("adapterlibgfx-headless-texture-bind-group"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source_view),
                },
            ],
        });
        let pipeline = self
            .pipeline(PipelineKind::Tex, sample_kind, format)
            .clone();
        let mut pass = self.begin_draw_pass(encoder, view, load);
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, frame_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        if let Some(scissor) = self.scissor {
            pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        }
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..gpu_vertices.len() as u32, 0..1);
    }

    fn begin_draw_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
        view: &'a wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("adapterlibgfx-headless-draw"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        })
    }

    fn pipeline(
        &mut self,
        kind: PipelineKind,
        sample_kind: TextureSampleKind,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let key = PipelineKey {
            kind,
            texture_effect: self.texture_effect,
            sample_kind,
            blend: self.blend,
            format,
        };
        if !self.pipelines.contains_key(&key) {
            let pipeline = self.create_pipeline(key);
            self.pipelines.insert(key, pipeline);
        }
        self.pipelines.get(&key).unwrap()
    }

    fn create_pipeline(&self, key: PipelineKey) -> wgpu::RenderPipeline {
        let (vs, fs, buffers, layout) = match key.kind {
            PipelineKind::Rgb => (
                "rgb_vs",
                match key.texture_effect {
                    TextureEffect::Plain => "rgb_fs",
                    TextureEffect::World => "world_rgb_fs",
                    TextureEffect::Blur => "rgb_fs",
                },
                vec![GpuRgbVertex::layout()],
                self.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("adapterlibgfx-headless-rgb-layout"),
                        bind_group_layouts: &[&self.frame_layout],
                        push_constant_ranges: &[],
                    }),
            ),
            PipelineKind::Tex => (
                "tex_vs",
                match key.sample_kind {
                    TextureSampleKind::Mask => "tex_mask_fs",
                    TextureSampleKind::Rgba => match key.texture_effect {
                        TextureEffect::Plain => "tex_fs",
                        TextureEffect::World => "world_tex_fs",
                        TextureEffect::Blur => "blur_tex_fs",
                    },
                },
                vec![GpuTexVertex::layout()],
                self.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("adapterlibgfx-headless-tex-layout"),
                        bind_group_layouts: &[&self.frame_layout, &self.texture_layout],
                        push_constant_ranges: &[],
                    }),
            ),
        };
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("adapterlibgfx-headless-pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some(vs),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some(fs),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: key.format,
                        blend: blend_state_to_wgpu(key.blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
    }
}

fn create_headless_target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("adapterlibgfx-headless-target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    (target, target_view)
}

fn clear_color(rgb: u32) -> wgpu::Color {
    wgpu::Color {
        r: ((rgb >> 16) & 0xFF) as f64 / 255.0,
        g: ((rgb >> 8) & 0xFF) as f64 / 255.0,
        b: (rgb & 0xFF) as f64 / 255.0,
        a: 1.0,
    }
}

fn blend_state_to_wgpu(state: BlendState) -> Option<wgpu::BlendState> {
    if !state.enabled {
        return None;
    }
    Some(wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: blend_factor_to_wgpu(state.src_rgb),
            dst_factor: blend_factor_to_wgpu(state.dst_rgb),
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: blend_factor_to_wgpu(state.src_rgb),
            dst_factor: blend_factor_to_wgpu(state.dst_rgb),
            operation: wgpu::BlendOperation::Add,
        },
    })
}

fn blend_factor_to_wgpu(factor: BlendFactor) -> wgpu::BlendFactor {
    match factor {
        BlendFactor::Zero => wgpu::BlendFactor::Zero,
        BlendFactor::One => wgpu::BlendFactor::One,
        BlendFactor::DstColor => wgpu::BlendFactor::Dst,
        BlendFactor::OneMinusDstColor => wgpu::BlendFactor::OneMinusDst,
        BlendFactor::SrcAlpha => wgpu::BlendFactor::SrcAlpha,
        BlendFactor::OneMinusSrcAlpha => wgpu::BlendFactor::OneMinusSrcAlpha,
        BlendFactor::OneMinusSrcColor => wgpu::BlendFactor::OneMinusSrc,
        BlendFactor::Other(_) => wgpu::BlendFactor::One,
    }
}

fn sampler_descriptor(state: SamplerState) -> wgpu::SamplerDescriptor<'static> {
    wgpu::SamplerDescriptor {
        label: Some("adapterlibgfx-sampler"),
        address_mode_u: address_mode(state.wrap_s),
        address_mode_v: address_mode(state.wrap_t),
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: filter_mode(state.mag_filter),
        min_filter: filter_mode(state.min_filter),
        mipmap_filter: filter_mode(state.min_filter),
        ..Default::default()
    }
}

fn address_mode(wrap: SamplerWrap) -> wgpu::AddressMode {
    match wrap {
        SamplerWrap::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        SamplerWrap::Repeat => wgpu::AddressMode::Repeat,
    }
}

fn filter_mode(filter: SamplerFilter) -> wgpu::FilterMode {
    match filter {
        SamplerFilter::Nearest => wgpu::FilterMode::Nearest,
        SamplerFilter::Linear => wgpu::FilterMode::Linear,
    }
}
