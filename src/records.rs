use bytemuck::{Pod, Zeroable};

pub const SOLID_RECT_SIZE: usize = 20;
pub const SPRITE_QUAD_SIZE: usize = 68;

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
pub struct SolidRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Rgba8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct SpriteCorner {
    pub x: f32,
    pub y: f32,
    pub u: f32,
    pub v: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct SpriteQuad {
    pub c0: SpriteCorner,
    pub c1: SpriteCorner,
    pub c2: SpriteCorner,
    pub c3: SpriteCorner,
    pub color: Rgba8,
}

pub fn usable_solid_len(len: usize) -> usize {
    len - (len % SOLID_RECT_SIZE)
}

pub fn usable_sprite_len(len: usize) -> usize {
    len - (len % SPRITE_QUAD_SIZE)
}

pub fn decode_solid_rects(bytes: &[u8]) -> Vec<SolidRect> {
    let usable = usable_solid_len(bytes.len());
    let mut out = Vec::with_capacity(usable / SOLID_RECT_SIZE);
    let mut off = 0usize;
    while off + SOLID_RECT_SIZE <= usable {
        out.push(SolidRect {
            x: f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()),
            y: f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()),
            w: f32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap()),
            h: f32::from_le_bytes(bytes[off + 12..off + 16].try_into().unwrap()),
            color: Rgba8::new(
                bytes[off + 16],
                bytes[off + 17],
                bytes[off + 18],
                bytes[off + 19],
            ),
        });
        off += SOLID_RECT_SIZE;
    }
    out
}

pub fn decode_sprite_quads(bytes: &[u8]) -> Vec<SpriteQuad> {
    let usable = usable_sprite_len(bytes.len());
    let mut out = Vec::with_capacity(usable / SPRITE_QUAD_SIZE);
    let mut off = 0usize;
    while off + SPRITE_QUAD_SIZE <= usable {
        out.push(SpriteQuad {
            c0: decode_sprite_corner(bytes, off),
            c1: decode_sprite_corner(bytes, off + 16),
            c2: decode_sprite_corner(bytes, off + 32),
            c3: decode_sprite_corner(bytes, off + 48),
            color: Rgba8::new(
                bytes[off + 64],
                bytes[off + 65],
                bytes[off + 66],
                bytes[off + 67],
            ),
        });
        off += SPRITE_QUAD_SIZE;
    }
    out
}

fn decode_sprite_corner(bytes: &[u8], off: usize) -> SpriteCorner {
    SpriteCorner {
        x: f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()),
        y: f32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()),
        u: f32::from_le_bytes(bytes[off + 8..off + 12].try_into().unwrap()),
        v: f32::from_le_bytes(bytes[off + 12..off + 16].try_into().unwrap()),
    }
}
