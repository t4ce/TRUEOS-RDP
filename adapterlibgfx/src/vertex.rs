use bytemuck::{Pod, Zeroable};

pub const RGB_VERTEX_SIZE: usize = 12;
pub const TEX_VERTEX_SIZE: usize = 20;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const WHITE: Self = Self::new(255, 255, 255, 255);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct RgbVertex {
    pub x: f32,
    pub y: f32,
    pub color: Rgba8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct TexVertex {
    pub x: f32,
    pub y: f32,
    pub u: f32,
    pub v: f32,
    pub color: Rgba8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuRgbVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuTexVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

pub fn usable_rgb_len(len: usize) -> usize {
    len - (len % RGB_VERTEX_SIZE)
}

pub fn usable_tex_len(len: usize) -> usize {
    len - (len % TEX_VERTEX_SIZE)
}

pub fn decode_rgb_vertices(bytes: &[u8]) -> Vec<RgbVertex> {
    let usable = usable_rgb_len(bytes.len());
    let mut out = Vec::with_capacity(usable / RGB_VERTEX_SIZE);
    let mut off = 0usize;
    while off + RGB_VERTEX_SIZE <= usable {
        out.push(RgbVertex {
            x: f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()),
            y: f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()),
            color: Rgba8::new(
                bytes[off + 8],
                bytes[off + 9],
                bytes[off + 10],
                bytes[off + 11],
            ),
        });
        off += RGB_VERTEX_SIZE;
    }
    out
}

pub fn decode_tex_vertices(bytes: &[u8]) -> Vec<TexVertex> {
    let usable = usable_tex_len(bytes.len());
    let mut out = Vec::with_capacity(usable / TEX_VERTEX_SIZE);
    let mut off = 0usize;
    while off + TEX_VERTEX_SIZE <= usable {
        out.push(TexVertex {
            x: f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()),
            y: f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()),
            u: f32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap()),
            v: f32::from_le_bytes(bytes[off + 12..off + 16].try_into().unwrap()),
            color: Rgba8::new(
                bytes[off + 16],
                bytes[off + 17],
                bytes[off + 18],
                bytes[off + 19],
            ),
        });
        off += TEX_VERTEX_SIZE;
    }
    out
}

fn color_to_float(color: Rgba8) -> [f32; 4] {
    [
        color.r as f32 / 255.0,
        color.g as f32 / 255.0,
        color.b as f32 / 255.0,
        color.a as f32 / 255.0,
    ]
}

impl From<RgbVertex> for GpuRgbVertex {
    fn from(value: RgbVertex) -> Self {
        Self {
            position: [value.x, value.y],
            color: color_to_float(value.color),
        }
    }
}

impl From<TexVertex> for GpuTexVertex {
    fn from(value: TexVertex) -> Self {
        Self {
            position: [value.x, value.y],
            uv: [value.u, value.v],
            color: color_to_float(value.color),
        }
    }
}

impl GpuRgbVertex {
    pub const ATTRS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: core::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}

impl GpuTexVertex {
    pub const ATTRS: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: core::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}
