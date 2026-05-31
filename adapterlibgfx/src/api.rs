use std::collections::HashMap;

use crate::command::{
    BlendState, ColorFormat, Frame, FrameCommand, PipelineResource, SamplerState, ShaderFormat,
    ShaderResource, ShaderStage, TextureEffect, TextureSampleKind, TexCoordFormat,
    VertexLayoutDesc,
};
use crate::texture::TextureRegistry;
use crate::vertex::{decode_rgb_vertices, decode_tex_vertices, usable_rgb_len, usable_tex_len};

#[derive(Clone, Copy, Debug)]
pub struct AdapterConfig {
    pub width: u32,
    pub height: u32,
}

impl Default for AdapterConfig {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameStats {
    pub frame_seq: u32,
    pub rgb_draws: u32,
    pub tex_draws: u32,
    pub draw_bytes: u32,
    pub command_count: u32,
}

#[derive(Debug)]
pub struct Adapter {
    pub config: AdapterConfig,
    textures: TextureRegistry,
    frame_active: bool,
    frame_seq: u32,
    clear_rgb: u32,
    allow_present: bool,
    preserve_contents: bool,
    commands: Vec<FrameCommand>,
    shaders: HashMap<u32, ShaderResource>,
    pipelines: HashMap<u32, PipelineResource>,
    stats: FrameStats,
    last_frame: Option<Frame>,
}

impl Default for Adapter {
    fn default() -> Self {
        Self::new(AdapterConfig::default())
    }
}

impl Adapter {
    pub fn new(config: AdapterConfig) -> Self {
        Self {
            config,
            textures: TextureRegistry::default(),
            frame_active: false,
            frame_seq: 0,
            clear_rgb: 0,
            allow_present: false,
            preserve_contents: false,
            commands: Vec::new(),
            shaders: HashMap::new(),
            pipelines: HashMap::new(),
            stats: FrameStats::default(),
            last_frame: None,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
    }

    pub fn begin_frame(&mut self, clear_rgb: u32) -> i32 {
        self.begin_frame_inner(clear_rgb, false, true)
    }

    pub fn begin_frame_no_present(&mut self, clear_rgb: u32) -> i32 {
        self.begin_frame_inner(clear_rgb, false, false)
    }

    pub fn begin_frame_preserve(&mut self, clear_rgb: u32) -> i32 {
        self.begin_frame_inner(clear_rgb, true, true)
    }

    pub fn begin_frame_preserve_no_present(&mut self, clear_rgb: u32) -> i32 {
        self.begin_frame_inner(clear_rgb, true, false)
    }

    fn begin_frame_inner(
        &mut self,
        clear_rgb: u32,
        preserve_contents: bool,
        allow_present: bool,
    ) -> i32 {
        if self.frame_active {
            return -1;
        }
        self.frame_active = true;
        self.frame_seq = self.frame_seq.wrapping_add(1).max(1);
        self.clear_rgb = clear_rgb & 0x00FF_FFFF;
        self.allow_present = allow_present;
        self.preserve_contents = preserve_contents;
        self.commands.clear();
        self.stats = FrameStats {
            frame_seq: self.frame_seq,
            ..FrameStats::default()
        };
        0
    }

    pub fn end_frame(&mut self) -> Result<Frame, i32> {
        if !self.frame_active {
            return Err(-1);
        }
        self.frame_active = false;
        let frame = Frame {
            seq: self.frame_seq,
            logical_width: self.config.width,
            logical_height: self.config.height,
            clear_rgb: self.clear_rgb,
            allow_present: self.allow_present,
            preserve_contents: self.preserve_contents,
            shaders: self.shaders.clone(),
            pipelines: self.pipelines.clone(),
            commands: core::mem::take(&mut self.commands),
        };
        self.stats.command_count = frame.commands.len().min(u32::MAX as usize) as u32;
        self.last_frame = Some(frame.clone());
        Ok(frame)
    }

    pub fn upload_texture_rgba_image(
        &mut self,
        tex_id: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> i32 {
        self.textures
            .upload_rgba(tex_id, width, height, rgba.to_vec())
    }

    pub fn texture_status(&self, tex_id: u32) -> i32 {
        self.textures.status(tex_id)
    }

    pub fn texture_dimensions(&self, tex_id: u32) -> Option<(u32, u32)> {
        self.textures.dimensions(tex_id)
    }

    pub fn set_blend_raw(&mut self, enabled: u32, src_rgb: u32, dst_rgb: u32) -> i32 {
        let state = BlendState {
            enabled: enabled != 0,
            src_rgb: crate::command::BlendFactor::from_gl(src_rgb),
            dst_rgb: crate::command::BlendFactor::from_gl(dst_rgb),
            raw_src_rgb: src_rgb,
            raw_dst_rgb: dst_rgb,
        };
        if self.frame_active {
            self.commands.push(FrameCommand::SetBlend(state));
        }
        0
    }

    pub fn set_sampler_raw(
        &mut self,
        wrap_s: u32,
        wrap_t: u32,
        min_filter: u32,
        mag_filter: u32,
    ) -> i32 {
        if self.frame_active {
            self.commands
                .push(FrameCommand::SetSampler(SamplerState::from_raw(
                    wrap_s, wrap_t, min_filter, mag_filter,
                )));
        }
        0
    }

    pub fn set_texture_effect(&mut self, effect: TextureEffect) -> i32 {
        if self.frame_active {
            self.commands.push(FrameCommand::SetTextureEffect(effect));
        }
        0
    }

    pub fn set_texture_effect_raw(&mut self, effect: u32) -> i32 {
        self.set_texture_effect(TextureEffect::from_raw(effect))
    }

    pub fn set_scissor(&mut self, rect: Option<crate::command::ScissorRect>) -> i32 {
        if self.frame_active {
            self.commands.push(FrameCommand::SetScissor(rect));
        }
        0
    }

    pub fn set_render_target(&mut self, tex_id: u32) -> i32 {
        if tex_id != 0 && self.texture_dimensions(tex_id).is_none() {
            return -1;
        }
        if self.frame_active {
            self.commands.push(FrameCommand::SetScissor(None));
            self.commands.push(FrameCommand::SetRenderTarget(tex_id));
        }
        0
    }

    pub fn clear_rgba_raw(&mut self, r: u32, g: u32, b: u32, a: u32) -> i32 {
        if self.frame_active {
            self.commands.push(FrameCommand::ClearRgba {
                r: r.min(255) as u8,
                g: g.min(255) as u8,
                b: b.min(255) as u8,
                a: a.min(255) as u8,
            });
        }
        0
    }

    pub fn create_shader_raw(
        &mut self,
        shader_id: u32,
        stage: u32,
        format: u32,
        source: &[u8],
    ) -> i32 {
        if shader_id == 0 {
            return -1;
        }
        let stage = ShaderStage::from_raw(stage);
        let format = ShaderFormat::from_raw(format);
        let Ok(source) = std::str::from_utf8(source) else {
            return -2;
        };
        if !matches!(format, ShaderFormat::Wgsl) {
            return -3;
        }
        self.shaders.insert(
            shader_id,
            ShaderResource {
                id: shader_id,
                stage,
                format,
                source: source.to_string(),
            },
        );
        self.pipelines.retain(|_, pipeline| {
            pipeline.vs_shader_id != shader_id && pipeline.fs_shader_id != shader_id
        });
        0
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_pipeline_raw(
        &mut self,
        pipeline_id: u32,
        stride: u32,
        pos_offset: u32,
        color_offset: u32,
        color_format: u32,
        texcoord_offset: u32,
        texcoord_format: u32,
        vs_shader_id: u32,
        fs_shader_id: u32,
    ) -> i32 {
        if pipeline_id == 0 || fs_shader_id == 0 {
            return -1;
        }
        if !self.shaders.contains_key(&fs_shader_id) {
            return -2;
        }
        self.pipelines.insert(
            pipeline_id,
            PipelineResource {
                id: pipeline_id,
                vertex_layout: VertexLayoutDesc {
                    stride,
                    pos_offset,
                    color_offset,
                    color_format: ColorFormat::from_raw(color_format),
                    texcoord_offset,
                    texcoord_format: TexCoordFormat::from_raw(texcoord_format),
                },
                vs_shader_id,
                fs_shader_id,
            },
        );
        0
    }

    pub fn draw_rgb_triangles_no_present(&mut self, bytes: &[u8]) -> i32 {
        if bytes.is_empty() {
            return 0;
        }
        let usable = usable_rgb_len(bytes.len());
        if usable == 0 {
            return -2;
        }
        if !self.frame_active {
            return -3;
        }
        self.stats.rgb_draws = self.stats.rgb_draws.saturating_add(1);
        self.stats.draw_bytes = self
            .stats
            .draw_bytes
            .saturating_add(usable.min(u32::MAX as usize) as u32);
        self.commands.push(FrameCommand::DrawRgb {
            vertices: decode_rgb_vertices(&bytes[..usable]),
        });
        0
    }

    pub fn draw_tex_triangles_no_present(&mut self, tex_id: u32, bytes: &[u8]) -> i32 {
        self.draw_tex_triangles_sample_kind_no_present(tex_id, TextureSampleKind::Rgba, bytes)
    }

    pub fn draw_tex_triangles_sample_kind_no_present(
        &mut self,
        tex_id: u32,
        sample_kind: TextureSampleKind,
        bytes: &[u8],
    ) -> i32 {
        if tex_id == 0 {
            return -1;
        }
        if bytes.is_empty() {
            return 0;
        }
        let usable = usable_tex_len(bytes.len());
        if usable == 0 {
            return -3;
        }
        if !self.frame_active {
            return -4;
        }
        if self.texture_dimensions(tex_id).is_none() {
            return -5;
        }
        self.stats.tex_draws = self.stats.tex_draws.saturating_add(1);
        self.stats.draw_bytes = self
            .stats
            .draw_bytes
            .saturating_add(usable.min(u32::MAX as usize) as u32);
        self.commands.push(FrameCommand::DrawTex {
            tex_id,
            sample_kind,
            vertices: decode_tex_vertices(&bytes[..usable]),
        });
        0
    }

    pub fn draw_pipeline_triangles_no_present(&mut self, pipeline_id: u32, bytes: &[u8]) -> i32 {
        if pipeline_id == 0 {
            return -1;
        }
        if bytes.is_empty() {
            return 0;
        }
        let usable = usable_tex_len(bytes.len());
        if usable == 0 {
            return -3;
        }
        if !self.frame_active {
            return -4;
        }
        if !self.pipelines.contains_key(&pipeline_id) {
            return -5;
        }
        self.stats.tex_draws = self.stats.tex_draws.saturating_add(1);
        self.stats.draw_bytes = self
            .stats
            .draw_bytes
            .saturating_add(usable.min(u32::MAX as usize) as u32);
        self.commands.push(FrameCommand::DrawPipeline {
            pipeline_id,
            vertices: decode_tex_vertices(&bytes[..usable]),
        });
        0
    }

    pub fn textures(&self) -> &TextureRegistry {
        &self.textures
    }

    pub fn last_stats(&self) -> FrameStats {
        self.stats
    }

    pub fn take_last_frame(&mut self) -> Option<Frame> {
        self.last_frame.take()
    }
}
