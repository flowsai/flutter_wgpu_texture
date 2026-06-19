pub mod api;
mod engine;
mod frb_generated;
mod gizmo;
mod level;
mod light;
#[cfg(target_os = "linux")]
mod linux_dma_buf;
mod picking;
mod present;
mod viewport;

// Re-export FRB boilerplate symbols so a combined workspace cdylib can pull
// them in via `pub use flutter_wgpu_texture_engine::*`.  These live inside
// the private `frb_generated` module; surfacing them at the crate root makes
// them part of the public API without duplicating any definitions.
#[cfg(not(target_family = "wasm"))]
pub use frb_generated::{
    frb_dart_fn_deliver_output, frb_get_rust_content_hash, frb_pde_ffi_dispatcher_primary,
    frb_pde_ffi_dispatcher_sync,
};

use std::ffi::c_void;

#[no_mangle]
pub extern "C" fn engine_create(width: u32, height: u32, scene_type: u8) -> u64 {
    let scene_name = match scene_type {
        2 => "particles",
        3 => "shader_playground",
        _ => "cube",
    };
    engine::engine_create(width, height, scene_name).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn engine_dispose(handle: u64) {
    engine::engine_dispose(handle);
}

#[no_mangle]
pub extern "C" fn engine_get_backend(handle: u64) -> u8 {
    engine::backend_code_for_handle(handle)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[no_mangle]
pub extern "C" fn engine_get_mtl_device(handle: u64) -> *mut c_void {
    engine::mtl_device_ptr_for_handle(handle)
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[no_mangle]
pub extern "C" fn engine_get_mtl_device(_handle: u64) -> *mut c_void {
    std::ptr::null_mut()
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
#[no_mangle]
pub extern "C" fn engine_attach_present_texture(
    handle: u64,
    mtl_texture_ptr: *mut c_void,
    width: u32,
    height: u32,
    bytes_per_row: u32,
) {
    let _ = engine::attach_metal_texture(handle, mtl_texture_ptr, width, height, bytes_per_row);
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[no_mangle]
pub extern "C" fn engine_attach_present_texture(
    _handle: u64,
    _mtl_texture_ptr: *mut c_void,
    _width: u32,
    _height: u32,
    _bytes_per_row: u32,
) {
}

#[cfg(target_os = "windows")]
#[no_mangle]
pub extern "C" fn engine_create_present_dxgi_surface(
    handle: u64,
    width: u32,
    height: u32,
) -> *mut c_void {
    engine::create_dxgi_surface(handle, width, height)
        .map(|value| value as *mut c_void)
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(not(target_os = "windows"))]
#[no_mangle]
pub extern "C" fn engine_create_present_dxgi_surface(
    _handle: u64,
    _width: u32,
    _height: u32,
) -> *mut c_void {
    std::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn engine_resize(handle: u64, width: u32, height: u32) -> u8 {
    if engine::resize_renderer(handle, width, height).is_ok() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn engine_request_frame(handle: u64) -> u8 {
    match engine::render_frame(handle) {
        Ok(true) => 1,
        _ => 0,
    }
}

#[repr(C)]
pub struct FfiDmaBufInfo {
    pub fd: i32,
    pub width: u32,
    pub height: u32,
    pub stride: i32,
    pub offset: i32,
    pub fourcc: i32,
    pub modifier_low: u32,
    pub modifier_high: u32,
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub extern "C" fn engine_dmabuf_supported(handle: u64) -> u8 {
    engine::linux_dmabuf_supported(handle)
        .map(|supported| if supported { 1 } else { 0 })
        .unwrap_or(0)
}

#[cfg(not(target_os = "linux"))]
#[no_mangle]
pub extern "C" fn engine_dmabuf_supported(_handle: u64) -> u8 {
    0
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub extern "C" fn engine_ensure_linux_present(handle: u64, width: u32, height: u32) -> u8 {
    if engine::ensure_linux_present(handle, width, height).is_ok() {
        1
    } else {
        0
    }
}

#[cfg(not(target_os = "linux"))]
#[no_mangle]
pub extern "C" fn engine_ensure_linux_present(_handle: u64, _width: u32, _height: u32) -> u8 {
    0
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub extern "C" fn engine_export_dmabuf(handle: u64, out_info: *mut FfiDmaBufInfo) -> u8 {
    if out_info.is_null() {
        return 0;
    }
    match engine::export_dmabuf(handle) {
        Ok(Some(info)) => {
            unsafe {
                (*out_info).fd = info.fd;
                (*out_info).width = info.width;
                (*out_info).height = info.height;
                (*out_info).stride = info.stride;
                (*out_info).offset = info.offset;
                (*out_info).fourcc = info.fourcc;
                (*out_info).modifier_low = (info.modifier & 0xFFFF_FFFF) as u32;
                (*out_info).modifier_high = (info.modifier >> 32) as u32;
            }
            1
        }
        _ => 0,
    }
}

#[cfg(not(target_os = "linux"))]
#[no_mangle]
pub extern "C" fn engine_export_dmabuf(_handle: u64, _out_info: *mut FfiDmaBufInfo) -> u8 {
    0
}
