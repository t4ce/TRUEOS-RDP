use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::executor::block_on;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::command::{
    BlendFactor, BlendState, Frame, FrameCommand, SamplerFilter, SamplerState, ScissorRect,
    TextureEffect, TextureSampleKind,
};
use crate::records::{Rgba8, SolidRect, SpriteQuad};
use crate::texture::{TextureImage, TextureRegistry};

#[derive(Debug)]
pub enum RenderError {
    AdapterUnavailable,
    DeviceUnavailable(String),
    Surface(String),
    MissingTexture(u32),
    InvalidTarget(u32),
}

#[derive(Clone)]
struct CpuSurface {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl CpuSurface {
    fn new(width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            width,
            height,
            rgba: vec![0; byte_len(width, height)],
        }
    }

    fn from_image(image: &TextureImage) -> Self {
        Self {
            width: image.width.max(1),
            height: image.height.max(1),
            rgba: image.rgba.clone(),
        }
    }

    fn clear(&mut self, rgb: u32) {
        let r = ((rgb >> 16) & 0xff) as u8;
        let g = ((rgb >> 8) & 0xff) as u8;
        let b = (rgb & 0xff) as u8;
        for px in self.rgba.chunks_exact_mut(4) {
            px[0] = r;
            px[1] = g;
            px[2] = b;
            px[3] = 255;
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.rgba.resize(byte_len(width, height), 0);
    }
}

#[derive(Clone, Copy)]
struct PaintState {
    blend: BlendState,
    sampler: SamplerState,
    effect: TextureEffect,
    scissor: Option<ScissorRect>,
    target: u32,
}

impl Default for PaintState {
    fn default() -> Self {
        Self {
            blend: BlendState::default(),
            sampler: SamplerState::default(),
            effect: TextureEffect::default(),
            scissor: None,
            target: 0,
        }
    }
}

enum SourceImage<'a> {
    Texture(&'a TextureImage),
    Surface(&'a CpuSurface),
}

#[derive(Clone, Copy)]
struct SourceView<'a> {
    width: u32,
    height: u32,
    rgba: &'a [u8],
}

impl SourceImage<'_> {
    fn view(&self) -> SourceView<'_> {
        match self {
            Self::Texture(image) => SourceView {
                width: image.width,
                height: image.height,
                rgba: &image.rgba,
            },
            Self::Surface(surface) => SourceView {
                width: surface.width,
                height: surface.height,
                rgba: &surface.rgba,
            },
        }
    }
}

pub struct WgpuRenderer {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    surfaces: HashMap<u32, CpuSurface>,
    upload_scratch: Vec<u8>,
}

pub struct WgpuHeadlessRenderer {
    _device: wgpu::Device,
    _queue: wgpu::Queue,
    width: u32,
    height: u32,
    surfaces: HashMap<u32, CpuSurface>,
}

impl WgpuRenderer {
    pub fn new(window: Arc<Window>) -> Result<Self, RenderError> {
        let backends = desktop_backends();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| RenderError::Surface(error.to_string()))?;
        let power_preference = desktop_power_preference();
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or(RenderError::AdapterUnavailable)?;
        let info = adapter.get_info();
        let (device, queue) = block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("adapterlibgfx-copy-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|error| RenderError::DeviceUnavailable(error.to_string()))?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|format| *format == wgpu::TextureFormat::Rgba8UnormSrgb)
            .or_else(|| {
                caps.formats
                    .iter()
                    .copied()
                    .find(|format| *format == wgpu::TextureFormat::Rgba8Unorm)
            })
            .or_else(|| caps.formats.iter().copied().find(|format| format.is_srgb()))
            .or_else(|| caps.formats.first().copied())
            .ok_or(RenderError::AdapterUnavailable)?;
        let present_mode = caps
            .present_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::PresentMode::Fifo)
            .unwrap_or(wgpu::PresentMode::AutoVsync);
        let alpha_mode = caps
            .alpha_modes
            .iter()
            .copied()
            .find(|mode| *mode == wgpu::CompositeAlphaMode::Opaque)
            .or_else(|| caps.alpha_modes.first().copied())
            .unwrap_or(wgpu::CompositeAlphaMode::Opaque);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        eprintln!(
            "adapterlibgfx desktop selected: backend={:?} requested_backends={:?} vendor=0x{:04X} device=0x{:04X} name={} power={:?} surface_format={:?} present_mode={:?} alpha_mode={:?} usage={:?} size={}x{}",
            info.backend,
            backends,
            info.vendor,
            info.device,
            info.name,
            power_preference,
            config.format,
            config.present_mode,
            config.alpha_mode,
            config.usage,
            config.width,
            config.height
        );
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            surfaces: HashMap::new(),
            upload_scratch: Vec::new(),
        })
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
        let logical = paint_frame(
            textures,
            frame,
            &mut self.surfaces,
            self.config.width,
            self.config.height,
        )?;
        if !frame.allow_present {
            return Ok(());
        }

        let surface_frame = match self.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.resize(self.window.inner_size());
                self.surface
                    .get_current_texture()
                    .map_err(|error| RenderError::Surface(error.to_string()))?
            }
            Err(error) => return Err(RenderError::Surface(error.to_string())),
        };
        let present = scale_surface(&logical, self.config.width, self.config.height);
        let upload = surface_upload_bytes(&present, self.config.format, &mut self.upload_scratch);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &surface_frame.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            upload.as_ref(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(present.width * 4),
                rows_per_image: Some(present.height),
            },
            wgpu::Extent3d {
                width: present.width,
                height: present.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(std::iter::empty());
        surface_frame.present();
        Ok(())
    }
}

fn desktop_backends() -> wgpu::Backends {
    if let Some(backends) = wgpu::Backends::from_env() {
        return backends;
    }
    if let Ok(value) = std::env::var("TACTICS_WGPU_BACKEND") {
        return wgpu::Backends::from_comma_list(&value);
    }

    wgpu::Backends::PRIMARY
}

fn desktop_power_preference() -> wgpu::PowerPreference {
    if let Ok(value) = std::env::var("TACTICS_WGPU_POWER") {
        match value.to_ascii_lowercase().as_str() {
            "low" => return wgpu::PowerPreference::LowPower,
            "none" => return wgpu::PowerPreference::None,
            _ => return wgpu::PowerPreference::HighPerformance,
        }
    }
    wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance)
}

fn surface_upload_bytes<'a>(
    surface: &'a CpuSurface,
    format: wgpu::TextureFormat,
    scratch: &'a mut Vec<u8>,
) -> Cow<'a, [u8]> {
    match format {
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
            scratch.clear();
            scratch.extend_from_slice(&surface.rgba);
            for px in scratch.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            Cow::Borrowed(scratch.as_slice())
        }
        _ => Cow::Borrowed(&surface.rgba),
    }
}

impl WgpuHeadlessRenderer {
    pub fn new_intel(width: u32, height: u32) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::default();
        let adapters = instance.enumerate_adapters(wgpu::Backends::all());
        let adapter = adapters
            .into_iter()
            .find(|adapter| adapter.get_info().vendor == 0x8086)
            .ok_or(RenderError::AdapterUnavailable)?;
        let info = adapter.get_info();
        eprintln!(
            "adapterlibgfx headless selected: backend={:?} vendor=0x{:04X} device=0x{:04X} name={}",
            info.backend, info.vendor, info.device, info.name
        );
        let (device, queue) = block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("adapterlibgfx-headless-copy-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|error| RenderError::DeviceUnavailable(error.to_string()))?;
        Ok(Self {
            _device: device,
            _queue: queue,
            width: width.max(1),
            height: height.max(1),
            surfaces: HashMap::new(),
        })
    }

    pub fn render_frame(
        &mut self,
        textures: &TextureRegistry,
        frame: &Frame,
    ) -> Result<(), RenderError> {
        let _ = paint_frame(textures, frame, &mut self.surfaces, self.width, self.height)?;
        Ok(())
    }

    pub fn last_surface_summary(&self) -> Option<SurfaceSummary> {
        surface_summary(self.surfaces.get(&0)?)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SurfaceSummary {
    pub width: u32,
    pub height: u32,
    pub non_black_pixels: u32,
    pub non_white_pixels: u32,
    pub alpha_pixels: u32,
    pub first_non_black: Option<(u32, u32, [u8; 4])>,
}

fn paint_frame(
    textures: &TextureRegistry,
    frame: &Frame,
    surfaces: &mut HashMap<u32, CpuSurface>,
    fallback_width: u32,
    fallback_height: u32,
) -> Result<CpuSurface, RenderError> {
    let width = frame.logical_width.max(fallback_width).max(1);
    let height = frame.logical_height.max(fallback_height).max(1);
    let mut cleared = HashSet::new();
    let mut state = PaintState::default();

    ensure_target_surface(surfaces, textures, 0, width, height)?;
    if frame.commands.is_empty() && frame.allow_present {
        if let Some(target) = surfaces.get_mut(&0) {
            target.clear(frame.clear_rgb);
        }
    }

    for command in &frame.commands {
        match command {
            FrameCommand::SetBlend(blend) => state.blend = *blend,
            FrameCommand::SetSampler(sampler) => state.sampler = *sampler,
            FrameCommand::SetTextureEffect(effect) => state.effect = *effect,
            FrameCommand::SetScissor(scissor) => state.scissor = *scissor,
            FrameCommand::SetRenderTarget(tex_id) => {
                state.target = *tex_id;
                state.scissor = None;
                let (target_width, target_height) = if *tex_id == 0 {
                    (width, height)
                } else if let Some(surface) = surfaces.get(tex_id) {
                    (surface.width, surface.height)
                } else if let Some((tw, th)) = textures.dimensions(*tex_id) {
                    (tw, th)
                } else {
                    return Err(RenderError::InvalidTarget(*tex_id));
                };
                ensure_target_surface(surfaces, textures, *tex_id, target_width, target_height)?;
            }
            FrameCommand::DrawSolid { rects } => {
                ensure_target_surface_for_state(surfaces, textures, state.target, width, height)?;
                clear_target_once(surfaces, state.target, &mut cleared, frame);
                let target = surfaces
                    .get_mut(&state.target)
                    .ok_or(RenderError::InvalidTarget(state.target))?;
                for rect in rects {
                    paint_solid_rect(target, *rect, state);
                }
            }
            FrameCommand::DrawSprite {
                tex_id,
                sample_kind,
                quads,
            } => {
                ensure_target_surface_for_state(surfaces, textures, state.target, width, height)?;
                clear_target_once(surfaces, state.target, &mut cleared, frame);
                let source_surface = surfaces.get(tex_id).cloned();
                let source = if let Some(surface) = source_surface.as_ref() {
                    SourceImage::Surface(surface)
                } else if let Some(image) = textures.image(*tex_id) {
                    SourceImage::Texture(image)
                } else {
                    return Err(RenderError::MissingTexture(*tex_id));
                };
                let source = source.view();
                let target = surfaces
                    .get_mut(&state.target)
                    .ok_or(RenderError::InvalidTarget(state.target))?;
                for quad in quads {
                    paint_sprite_quad(target, source, *quad, *sample_kind, state);
                }
            }
        }
    }

    Ok(surfaces
        .get(&0)
        .cloned()
        .unwrap_or_else(|| CpuSurface::new(width, height)))
}

fn ensure_target_surface_for_state(
    surfaces: &mut HashMap<u32, CpuSurface>,
    textures: &TextureRegistry,
    tex_id: u32,
    fallback_width: u32,
    fallback_height: u32,
) -> Result<(), RenderError> {
    if tex_id == 0 {
        ensure_target_surface(surfaces, textures, 0, fallback_width, fallback_height)
    } else {
        let (width, height) = if let Some(surface) = surfaces.get(&tex_id) {
            (surface.width, surface.height)
        } else {
            textures
                .dimensions(tex_id)
                .ok_or(RenderError::InvalidTarget(tex_id))?
        };
        ensure_target_surface(surfaces, textures, tex_id, width, height)
    }
}

fn ensure_target_surface(
    surfaces: &mut HashMap<u32, CpuSurface>,
    textures: &TextureRegistry,
    tex_id: u32,
    width: u32,
    height: u32,
) -> Result<(), RenderError> {
    if tex_id == 0 {
        surfaces
            .entry(0)
            .or_insert_with(|| CpuSurface::new(width, height))
            .resize(width, height);
        return Ok(());
    }
    if !surfaces.contains_key(&tex_id) {
        let image = textures
            .image(tex_id)
            .ok_or(RenderError::InvalidTarget(tex_id))?;
        surfaces.insert(tex_id, CpuSurface::from_image(image));
    }
    Ok(())
}

fn clear_target_once(
    surfaces: &mut HashMap<u32, CpuSurface>,
    tex_id: u32,
    cleared: &mut HashSet<u32>,
    frame: &Frame,
) {
    if frame.preserve_contents || !cleared.insert(tex_id) {
        return;
    }
    if let Some(surface) = surfaces.get_mut(&tex_id) {
        surface.clear(frame.clear_rgb);
    }
}

fn paint_solid_rect(target: &mut CpuSurface, rect: SolidRect, state: PaintState) {
    if !(rect.w > 0.0 && rect.h > 0.0) {
        return;
    }
    let (x0, y0, x1, y1) = rect_bounds(rect.x, rect.y, rect.w, rect.h);
    let clip = clipped_bounds(target, state.scissor, x0, y0, x1, y1);
    for y in clip.1..clip.3 {
        for x in clip.0..clip.2 {
            put_pixel(target, x, y, rect.color, state.blend);
        }
    }
}

fn paint_sprite_quad(
    target: &mut CpuSurface,
    source: SourceView<'_>,
    quad: SpriteQuad,
    sample_kind: TextureSampleKind,
    state: PaintState,
) {
    let min_x = quad
        .c0
        .x
        .min(quad.c1.x)
        .min(quad.c2.x)
        .min(quad.c3.x)
        .floor() as i32;
    let min_y = quad
        .c0
        .y
        .min(quad.c1.y)
        .min(quad.c2.y)
        .min(quad.c3.y)
        .floor() as i32;
    let max_x = quad
        .c0
        .x
        .max(quad.c1.x)
        .max(quad.c2.x)
        .max(quad.c3.x)
        .ceil() as i32;
    let max_y = quad
        .c0
        .y
        .max(quad.c1.y)
        .max(quad.c2.y)
        .max(quad.c3.y)
        .ceil() as i32;
    let clip = clipped_bounds(target, state.scissor, min_x, min_y, max_x, max_y);
    let ux_x = quad.c1.x - quad.c0.x;
    let ux_y = quad.c1.y - quad.c0.y;
    let vy_x = quad.c3.x - quad.c0.x;
    let vy_y = quad.c3.y - quad.c0.y;
    let det = ux_x * vy_y - ux_y * vy_x;
    if det.abs() < 0.0001 {
        return;
    }
    let inv_det = 1.0 / det;
    for y in clip.1..clip.3 {
        for x in clip.0..clip.2 {
            let px = x as f32 + 0.5 - quad.c0.x;
            let py = y as f32 + 0.5 - quad.c0.y;
            let s = (px * vy_y - py * vy_x) * inv_det;
            let t = (ux_x * py - ux_y * px) * inv_det;
            if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&t) {
                continue;
            }
            let u = lerp2(quad.c0.u, quad.c1.u, quad.c2.u, quad.c3.u, s, t);
            let v = lerp2(quad.c0.v, quad.c1.v, quad.c2.v, quad.c3.v, s, t);
            let texel = sample(source, u, v, state.sampler, state.effect);
            let color = texture_color(texel, quad.color, sample_kind);
            if color.a == 0 {
                continue;
            }
            put_pixel(target, x, y, color, state.blend);
        }
    }
}

fn rect_bounds(x: f32, y: f32, w: f32, h: f32) -> (i32, i32, i32, i32) {
    (
        x.floor() as i32,
        y.floor() as i32,
        (x + w).ceil() as i32,
        (y + h).ceil() as i32,
    )
}

fn clipped_bounds(
    target: &CpuSurface,
    scissor: Option<ScissorRect>,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
) -> (i32, i32, i32, i32) {
    let mut left = x0.max(0);
    let mut top = y0.max(0);
    let mut right = x1.min(target.width as i32);
    let mut bottom = y1.min(target.height as i32);
    if let Some(scissor) = scissor {
        left = left.max(scissor.x as i32);
        top = top.max(scissor.y as i32);
        right = right.min(scissor.x.saturating_add(scissor.width) as i32);
        bottom = bottom.min(scissor.y.saturating_add(scissor.height) as i32);
    }
    (left, top, right.max(left), bottom.max(top))
}

fn sample(
    source: SourceView<'_>,
    u: f32,
    v: f32,
    sampler: SamplerState,
    effect: TextureEffect,
) -> Rgba8 {
    if source.width == 0 || source.height == 0 {
        return Rgba8::default();
    }
    if matches!(effect, TextureEffect::Blur) {
        return sample_blur(source, u, v, sampler);
    }
    if matches!(sampler.min_filter, SamplerFilter::Linear)
        || matches!(sampler.mag_filter, SamplerFilter::Linear)
    {
        sample_linear(source, u, v, sampler)
    } else {
        sample_nearest(source, u, v, sampler)
    }
}

fn sample_blur(source: SourceView<'_>, u: f32, v: f32, sampler: SamplerState) -> Rgba8 {
    let du = 1.0 / source.width.max(1) as f32;
    let dv = 1.0 / source.height.max(1) as f32;
    let mut sum = [0u32; 4];
    for oy in -1..=1 {
        for ox in -1..=1 {
            let px = sample_nearest(source, u + ox as f32 * du, v + oy as f32 * dv, sampler);
            sum[0] += px.r as u32;
            sum[1] += px.g as u32;
            sum[2] += px.b as u32;
            sum[3] += px.a as u32;
        }
    }
    Rgba8::new(
        (sum[0] / 9) as u8,
        (sum[1] / 9) as u8,
        (sum[2] / 9) as u8,
        (sum[3] / 9) as u8,
    )
}

fn sample_nearest(source: SourceView<'_>, u: f32, v: f32, sampler: SamplerState) -> Rgba8 {
    let (u, v) = wrap_uv(u, v, sampler);
    let x = (u * source.width as f32)
        .floor()
        .clamp(0.0, source.width.saturating_sub(1) as f32) as u32;
    let y = (v * source.height as f32)
        .floor()
        .clamp(0.0, source.height.saturating_sub(1) as f32) as u32;
    get_pixel(source, x, y)
}

fn sample_linear(source: SourceView<'_>, u: f32, v: f32, sampler: SamplerState) -> Rgba8 {
    let (u, v) = wrap_uv(u, v, sampler);
    let fx = (u * source.width as f32 - 0.5).clamp(0.0, source.width.saturating_sub(1) as f32);
    let fy = (v * source.height as f32 - 0.5).clamp(0.0, source.height.saturating_sub(1) as f32);
    let x0 = fx.floor() as u32;
    let y0 = fy.floor() as u32;
    let x1 = (x0 + 1).min(source.width.saturating_sub(1));
    let y1 = (y0 + 1).min(source.height.saturating_sub(1));
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;
    let a = get_pixel(source, x0, y0);
    let b = get_pixel(source, x1, y0);
    let c = get_pixel(source, x1, y1);
    let d = get_pixel(source, x0, y1);
    Rgba8::new(
        lerp2(a.r as f32, b.r as f32, c.r as f32, d.r as f32, tx, ty).round() as u8,
        lerp2(a.g as f32, b.g as f32, c.g as f32, d.g as f32, tx, ty).round() as u8,
        lerp2(a.b as f32, b.b as f32, c.b as f32, d.b as f32, tx, ty).round() as u8,
        lerp2(a.a as f32, b.a as f32, c.a as f32, d.a as f32, tx, ty).round() as u8,
    )
}

fn wrap_uv(u: f32, v: f32, sampler: SamplerState) -> (f32, f32) {
    let u = if matches!(sampler.wrap_s, crate::command::SamplerWrap::Repeat) {
        u.rem_euclid(1.0)
    } else {
        u.clamp(0.0, 1.0)
    };
    let v = if matches!(sampler.wrap_t, crate::command::SamplerWrap::Repeat) {
        v.rem_euclid(1.0)
    } else {
        v.clamp(0.0, 1.0)
    };
    (u, v)
}

fn texture_color(texel: Rgba8, tint: Rgba8, sample_kind: TextureSampleKind) -> Rgba8 {
    match sample_kind {
        TextureSampleKind::Rgba => Rgba8::new(
            mul_u8(texel.r, tint.r),
            mul_u8(texel.g, tint.g),
            mul_u8(texel.b, tint.b),
            mul_u8(texel.a, tint.a),
        ),
        TextureSampleKind::Mask => {
            let mask = if texel.a < 255 { texel.a } else { texel.r };
            Rgba8::new(tint.r, tint.g, tint.b, mul_u8(tint.a, mask))
        }
    }
}

fn put_pixel(target: &mut CpuSurface, x: i32, y: i32, src: Rgba8, blend: BlendState) {
    if x < 0 || y < 0 || x >= target.width as i32 || y >= target.height as i32 {
        return;
    }
    let off = ((y as u32 * target.width + x as u32) * 4) as usize;
    if !blend.enabled {
        target.rgba[off] = src.r;
        target.rgba[off + 1] = src.g;
        target.rgba[off + 2] = src.b;
        target.rgba[off + 3] = src.a;
        return;
    }
    let dst = Rgba8::new(
        target.rgba[off],
        target.rgba[off + 1],
        target.rgba[off + 2],
        target.rgba[off + 3],
    );
    let src_factor = blend_factor(blend.src_rgb, src, dst);
    let dst_factor = blend_factor(blend.dst_rgb, src, dst);
    target.rgba[off] = blend_channel(src.r, dst.r, src_factor, dst_factor);
    target.rgba[off + 1] = blend_channel(src.g, dst.g, src_factor, dst_factor);
    target.rgba[off + 2] = blend_channel(src.b, dst.b, src_factor, dst_factor);
    target.rgba[off + 3] = src
        .a
        .saturating_add(((dst.a as u16 * (255 - src.a) as u16) / 255) as u8);
}

fn blend_factor(factor: BlendFactor, src: Rgba8, dst: Rgba8) -> u8 {
    match factor {
        BlendFactor::Zero => 0,
        BlendFactor::One => 255,
        BlendFactor::DstColor => dst.r,
        BlendFactor::OneMinusDstColor => 255 - dst.r,
        BlendFactor::SrcAlpha => src.a,
        BlendFactor::OneMinusSrcAlpha => 255 - src.a,
        BlendFactor::OneMinusSrcColor => 255 - src.r,
        BlendFactor::Other(_) => 255,
    }
}

fn blend_channel(src: u8, dst: u8, src_factor: u8, dst_factor: u8) -> u8 {
    let value = src as u32 * src_factor as u32 + dst as u32 * dst_factor as u32;
    (value / 255).min(255) as u8
}

fn get_pixel(source: SourceView<'_>, x: u32, y: u32) -> Rgba8 {
    let off = ((y * source.width + x) * 4) as usize;
    Rgba8::new(
        source.rgba[off],
        source.rgba[off + 1],
        source.rgba[off + 2],
        source.rgba[off + 3],
    )
}

fn scale_surface(source: &CpuSurface, width: u32, height: u32) -> CpuSurface {
    let width = width.max(1);
    let height = height.max(1);
    if source.width == width && source.height == height {
        return source.clone();
    }
    let mut out = CpuSurface::new(width, height);
    for y in 0..height {
        let sy = (y as u64 * source.height as u64 / height as u64) as u32;
        for x in 0..width {
            let sx = (x as u64 * source.width as u64 / width as u64) as u32;
            let src = ((sy * source.width + sx) * 4) as usize;
            let dst = ((y * width + x) * 4) as usize;
            out.rgba[dst..dst + 4].copy_from_slice(&source.rgba[src..src + 4]);
        }
    }
    out
}

fn surface_summary(surface: &CpuSurface) -> Option<SurfaceSummary> {
    if surface.rgba.is_empty() {
        return None;
    }
    let mut summary = SurfaceSummary {
        width: surface.width,
        height: surface.height,
        ..SurfaceSummary::default()
    };
    for (idx, px) in surface.rgba.chunks_exact(4).enumerate() {
        let rgba = [px[0], px[1], px[2], px[3]];
        if rgba[0] != 0 || rgba[1] != 0 || rgba[2] != 0 {
            summary.non_black_pixels = summary.non_black_pixels.saturating_add(1);
            if summary.first_non_black.is_none() {
                let x = (idx as u32) % surface.width.max(1);
                let y = (idx as u32) / surface.width.max(1);
                summary.first_non_black = Some((x, y, rgba));
            }
        }
        if rgba[0] != 255 || rgba[1] != 255 || rgba[2] != 255 {
            summary.non_white_pixels = summary.non_white_pixels.saturating_add(1);
        }
        if rgba[3] != 0 {
            summary.alpha_pixels = summary.alpha_pixels.saturating_add(1);
        }
    }
    Some(summary)
}

fn lerp2(a: f32, b: f32, c: f32, d: f32, s: f32, t: f32) -> f32 {
    let top = a + (b - a) * s;
    let bottom = d + (c - d) * s;
    top + (bottom - top) * t
}

fn mul_u8(a: u8, b: u8) -> u8 {
    ((a as u16 * b as u16 + 127) / 255) as u8
}

fn byte_len(width: u32, height: u32) -> usize {
    width as usize * height as usize * 4
}
