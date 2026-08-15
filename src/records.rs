use bytemuck::{Pod, Zeroable};

pub const SOLID_VERTEX_SIZE: usize = 12;
pub const SOLID_TRIANGLE_SIZE: usize = SOLID_VERTEX_SIZE * 3;
pub const SPRITE_VERTEX_SIZE: usize = 20;
pub const SPRITE_TRIANGLE_SIZE: usize = SPRITE_VERTEX_SIZE * 3;

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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SolidVertex {
    pub x: f32,
    pub y: f32,
    pub color: Rgba8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SolidTriangle {
    pub vertices: [SolidVertex; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpriteVertex {
    pub x: f32,
    pub y: f32,
    pub u: f32,
    pub v: f32,
    pub color: Rgba8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpriteTriangle {
    pub vertices: [SpriteVertex; 3],
}

pub fn usable_solid_len(len: usize) -> usize {
    len - (len % SOLID_TRIANGLE_SIZE)
}

pub fn usable_sprite_len(len: usize) -> usize {
    len - (len % SPRITE_TRIANGLE_SIZE)
}

pub fn decode_solid_triangles(bytes: &[u8]) -> Vec<SolidTriangle> {
    let usable = usable_solid_len(bytes.len());
    let mut out = Vec::with_capacity(usable / SOLID_TRIANGLE_SIZE);
    let mut off = 0usize;
    while off + SOLID_TRIANGLE_SIZE <= usable {
        out.push(SolidTriangle {
            vertices: [
                decode_solid_vertex(bytes, off),
                decode_solid_vertex(bytes, off + SOLID_VERTEX_SIZE),
                decode_solid_vertex(bytes, off + SOLID_VERTEX_SIZE * 2),
            ],
        });
        off += SOLID_TRIANGLE_SIZE;
    }
    out
}

pub fn decode_sprite_triangles(bytes: &[u8]) -> Vec<SpriteTriangle> {
    let usable = usable_sprite_len(bytes.len());
    let mut out = Vec::with_capacity(usable / SPRITE_TRIANGLE_SIZE);
    let mut off = 0usize;
    while off + SPRITE_TRIANGLE_SIZE <= usable {
        out.push(SpriteTriangle {
            vertices: [
                decode_sprite_vertex(bytes, off),
                decode_sprite_vertex(bytes, off + SPRITE_VERTEX_SIZE),
                decode_sprite_vertex(bytes, off + SPRITE_VERTEX_SIZE * 2),
            ],
        });
        off += SPRITE_TRIANGLE_SIZE;
    }
    out
}

fn decode_solid_vertex(bytes: &[u8], off: usize) -> SolidVertex {
    SolidVertex {
        x: read_f32(bytes, off),
        y: read_f32(bytes, off + 4),
        color: decode_color(bytes, off + 8),
    }
}

fn decode_sprite_vertex(bytes: &[u8], off: usize) -> SpriteVertex {
    SpriteVertex {
        x: read_f32(bytes, off),
        y: read_f32(bytes, off + 4),
        u: read_f32(bytes, off + 8),
        v: read_f32(bytes, off + 12),
        color: decode_color(bytes, off + 16),
    }
}

fn read_f32(bytes: &[u8], off: usize) -> f32 {
    f32::from_le_bytes(bytes[off..off + 4].try_into().expect("checked record size"))
}

fn decode_color(bytes: &[u8], off: usize) -> Rgba8 {
    Rgba8::new(bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_existing_solid_triangle_wire_shape() {
        let mut bytes = Vec::new();
        for (x, y, color) in [
            (-1.0f32, 1.0f32, [1, 2, 3, 4]),
            (1.0, 1.0, [5, 6, 7, 8]),
            (0.0, -1.0, [9, 10, 11, 12]),
        ] {
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&y.to_le_bytes());
            bytes.extend_from_slice(&color);
        }
        let triangles = decode_solid_triangles(&bytes);
        assert_eq!(triangles.len(), 1);
        assert_eq!(triangles[0].vertices[2].y, -1.0);
        assert_eq!(triangles[0].vertices[1].color, Rgba8::new(5, 6, 7, 8));
    }

    #[test]
    fn decodes_two_existing_textured_triangles_without_truncation() {
        let mut bytes = Vec::new();
        for index in 0..6u8 {
            bytes.extend_from_slice(&(index as f32).to_le_bytes());
            bytes.extend_from_slice(&0.0f32.to_le_bytes());
            bytes.extend_from_slice(&(index as f32 / 5.0).to_le_bytes());
            bytes.extend_from_slice(&0.5f32.to_le_bytes());
            bytes.extend_from_slice(&[255, 255, 255, 255]);
        }
        assert_eq!(bytes.len(), 120);
        let triangles = decode_sprite_triangles(&bytes);
        assert_eq!(triangles.len(), 2);
        assert_eq!(triangles[1].vertices[2].x, 5.0);
    }
}
