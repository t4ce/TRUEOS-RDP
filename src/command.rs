use crate::records::{SolidRect, SpriteQuad};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BlendFactor {
    Zero,
    One,
    DstColor,
    OneMinusDstColor,
    SrcAlpha,
    OneMinusSrcAlpha,
    OneMinusSrcColor,
    Other(u32),
}

impl BlendFactor {
    pub fn from_gl(value: u32) -> Self {
        match value {
            0 => Self::Zero,
            1 => Self::One,
            0x0306 => Self::DstColor,
            0x0307 => Self::OneMinusDstColor,
            0x0302 => Self::SrcAlpha,
            0x0303 => Self::OneMinusSrcAlpha,
            0x0301 => Self::OneMinusSrcColor,
            other => Self::Other(other),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BlendState {
    pub enabled: bool,
    pub src_rgb: BlendFactor,
    pub dst_rgb: BlendFactor,
    pub raw_src_rgb: u32,
    pub raw_dst_rgb: u32,
}

impl Default for BlendState {
    fn default() -> Self {
        Self {
            enabled: false,
            src_rgb: BlendFactor::One,
            dst_rgb: BlendFactor::Zero,
            raw_src_rgb: 1,
            raw_dst_rgb: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SamplerWrap {
    ClampToEdge,
    Repeat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SamplerFilter {
    Nearest,
    Linear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SamplerState {
    pub wrap_s: SamplerWrap,
    pub wrap_t: SamplerWrap,
    pub min_filter: SamplerFilter,
    pub mag_filter: SamplerFilter,
}

impl Default for SamplerState {
    fn default() -> Self {
        Self {
            wrap_s: SamplerWrap::ClampToEdge,
            wrap_t: SamplerWrap::ClampToEdge,
            min_filter: SamplerFilter::Nearest,
            mag_filter: SamplerFilter::Nearest,
        }
    }
}

impl SamplerState {
    pub fn from_raw(wrap_s: u32, wrap_t: u32, min_filter: u32, mag_filter: u32) -> Self {
        Self {
            wrap_s: if wrap_s == 1 {
                SamplerWrap::Repeat
            } else {
                SamplerWrap::ClampToEdge
            },
            wrap_t: if wrap_t == 1 {
                SamplerWrap::Repeat
            } else {
                SamplerWrap::ClampToEdge
            },
            min_filter: if min_filter == 0 {
                SamplerFilter::Nearest
            } else {
                SamplerFilter::Linear
            },
            mag_filter: if mag_filter == 0 {
                SamplerFilter::Nearest
            } else {
                SamplerFilter::Linear
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TextureEffect {
    #[default]
    Plain,
    World,
    Blur,
}

impl TextureEffect {
    pub fn from_raw(value: u32) -> Self {
        match value {
            1 => Self::World,
            2 => Self::Blur,
            _ => Self::Plain,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TextureSampleKind {
    Mask,
    #[default]
    Rgba,
}

impl TextureSampleKind {
    pub fn from_raw(value: u32) -> Self {
        match value {
            0 => Self::Mask,
            _ => Self::Rgba,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScissorRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub enum FrameCommand {
    SetBlend(BlendState),
    SetSampler(SamplerState),
    SetTextureEffect(TextureEffect),
    SetScissor(Option<ScissorRect>),
    SetRenderTarget(u32),
    DrawSolid {
        rects: Vec<SolidRect>,
    },
    DrawSprite {
        tex_id: u32,
        sample_kind: TextureSampleKind,
        quads: Vec<SpriteQuad>,
    },
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub seq: u32,
    pub logical_width: u32,
    pub logical_height: u32,
    pub clear_rgb: u32,
    pub allow_present: bool,
    pub preserve_contents: bool,
    pub commands: Vec<FrameCommand>,
}
