use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::api::{Adapter, AdapterConfig, FrameStats};
#[cfg(feature = "ffi")]
use crate::command::ScissorRect;

static ADAPTER: Lazy<Mutex<Adapter>> =
    Lazy::new(|| Mutex::new(Adapter::new(AdapterConfig::default())));

pub fn with_adapter<R>(f: impl FnOnce(&mut Adapter) -> R) -> R {
    let mut adapter = ADAPTER.lock();
    f(&mut adapter)
}

pub fn adapter_last_frame_stats() -> FrameStats {
    ADAPTER.lock().last_stats()
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_begin_frame(clear_rgb: u32) -> i32 {
    ADAPTER.lock().begin_frame(clear_rgb)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_begin_frame_preserve(clear_rgb: u32) -> i32 {
    ADAPTER.lock().begin_frame_preserve(clear_rgb)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_begin_frame_no_present(clear_rgb: u32) -> i32 {
    ADAPTER.lock().begin_frame_no_present(clear_rgb)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_end_frame() -> i32 {
    ADAPTER
        .lock()
        .end_frame()
        .map(|_| 0)
        .unwrap_or_else(|rc| rc)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_set_blend(
    enabled: u32,
    src_rgb: u32,
    dst_rgb: u32,
    _src_alpha: u32,
    _dst_alpha: u32,
    _eq_rgb: u32,
    _eq_alpha: u32,
) -> i32 {
    ADAPTER.lock().set_blend_raw(enabled, src_rgb, dst_rgb)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_set_sampler(
    wrap_s: u32,
    wrap_t: u32,
    min_filter: u32,
    mag_filter: u32,
) -> i32 {
    ADAPTER
        .lock()
        .set_sampler_raw(wrap_s, wrap_t, min_filter, mag_filter)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_set_texture_effect(effect: u32) -> i32 {
    ADAPTER.lock().set_texture_effect_raw(effect)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_set_scissor(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> i32 {
    ADAPTER.lock().set_scissor(Some(ScissorRect {
        x,
        y,
        width,
        height,
    }))
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_clear_scissor() -> i32 {
    ADAPTER.lock().set_scissor(None)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_set_render_target(tex_id: u32) -> i32 {
    ADAPTER.lock().set_render_target(tex_id)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_clear_render_target() -> i32 {
    ADAPTER.lock().set_render_target(0)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_draw_rgb_triangles_no_present(
    vtx_ptr: *const u8,
    vtx_len: usize,
) -> i32 {
    if vtx_ptr.is_null() {
        return if vtx_len == 0 { 0 } else { -1 };
    }
    let bytes = unsafe { core::slice::from_raw_parts(vtx_ptr, vtx_len) };
    ADAPTER.lock().draw_rgb_triangles_no_present(bytes)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_draw_tex_triangles_no_present(
    tex_id: u32,
    vtx_ptr: *const u8,
    vtx_len: usize,
) -> i32 {
    if vtx_ptr.is_null() {
        return if vtx_len == 0 { 0 } else { -2 };
    }
    let bytes = unsafe { core::slice::from_raw_parts(vtx_ptr, vtx_len) };
    ADAPTER.lock().draw_tex_triangles_no_present(tex_id, bytes)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_upload_texture_rgba_image(
    tex_id: u32,
    width: u32,
    height: u32,
    data_ptr: *const u8,
    data_len: usize,
) -> i32 {
    if data_ptr.is_null() {
        return -3;
    }
    let bytes = unsafe { core::slice::from_raw_parts(data_ptr, data_len) };
    ADAPTER
        .lock()
        .upload_texture_rgba_image(tex_id, width, height, bytes)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_upload_texture_rgba_image_async(
    tex_id: u32,
    width: u32,
    height: u32,
    data_ptr: *const u8,
    data_len: usize,
) -> i32 {
    unsafe { trueos_cabi_gfx_upload_texture_rgba_image(tex_id, width, height, data_ptr, data_len) }
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub extern "C" fn trueos_cabi_gfx_texture_status(tex_id: u32) -> i32 {
    ADAPTER.lock().texture_status(tex_id)
}

#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn trueos_cabi_gfx_texture_dimensions(
    tex_id: u32,
    out_width: *mut u32,
    out_height: *mut u32,
) -> i32 {
    if out_width.is_null() || out_height.is_null() {
        return -1;
    }
    let Some((width, height)) = ADAPTER.lock().texture_dimensions(tex_id) else {
        return -2;
    };
    unsafe {
        core::ptr::write(out_width, width);
        core::ptr::write(out_height, height);
    }
    0
}
