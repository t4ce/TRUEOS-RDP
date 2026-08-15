use std::collections::HashMap;

pub const ASYNC_TEX_STATUS_UNKNOWN: i32 = 0;
pub const ASYNC_TEX_STATUS_PENDING: i32 = 1;
pub const ASYNC_TEX_STATUS_READY: i32 = 2;

#[derive(Clone, Debug)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub revision: u64,
}

#[derive(Debug, Default)]
pub struct TextureRegistry {
    next_revision: u64,
    images: HashMap<u32, TextureImage>,
}

impl TextureRegistry {
    pub fn upload_rgba(&mut self, tex_id: u32, width: u32, height: u32, rgba: Vec<u8>) -> i32 {
        if tex_id == 0 || width == 0 || height == 0 {
            return -1;
        }
        let Some(expected) = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(4))
        else {
            return -2;
        };
        if rgba.len() != expected {
            return -2;
        }
        self.next_revision = self.next_revision.wrapping_add(1).max(1);
        self.images.insert(
            tex_id,
            TextureImage {
                width,
                height,
                rgba,
                revision: self.next_revision,
            },
        );
        0
    }

    pub fn status(&self, tex_id: u32) -> i32 {
        if self.images.contains_key(&tex_id) {
            ASYNC_TEX_STATUS_READY
        } else {
            ASYNC_TEX_STATUS_UNKNOWN
        }
    }

    pub fn dimensions(&self, tex_id: u32) -> Option<(u32, u32)> {
        self.images.get(&tex_id).map(|x| (x.width, x.height))
    }

    pub fn image(&self, tex_id: u32) -> Option<&TextureImage> {
        self.images.get(&tex_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u32, &TextureImage)> {
        self.images.iter()
    }
}
