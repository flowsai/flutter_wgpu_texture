use flutter_rust_bridge::frb;

use crate::engine;

#[derive(Clone, Debug)]
pub struct BackendInfo {
    pub backend: String,
    pub device_name: String,
    pub driver: String,
}

#[derive(Clone, Debug)]
pub struct RendererInfo {
    pub handle: u64,
    pub backend: BackendInfo,
}

#[derive(Clone, Debug)]
pub struct DmaBufExport {
    pub fd: i32,
    pub width: u32,
    pub height: u32,
    pub stride: i32,
    pub offset: i32,
    pub fourcc: i32,
    pub modifier_low: u32,
    pub modifier_high: u32,
}

#[frb(sync)]
pub fn create_renderer(
    width: u32,
    height: u32,
    scene_type: String,
) -> Result<RendererInfo, String> {
    let handle = engine::engine_create(width, height, &scene_type)?;
    let backend = engine::renderer_backend_info(handle)?;
    Ok(RendererInfo { handle, backend })
}

#[frb(sync)]
pub fn dispose_renderer(handle: u64) {
    engine::engine_dispose(handle);
}

pub fn request_frame(handle: u64) -> Result<bool, String> {
    engine::render_frame(handle)
}

#[frb(sync)]
pub fn start_animation(handle: u64) -> Result<(), String> {
    engine::set_animation_running(handle, true)
}

#[frb(sync)]
pub fn stop_animation(handle: u64) -> Result<(), String> {
    engine::set_animation_running(handle, false)
}

#[frb(sync)]
pub fn set_bool_param(handle: u64, key: String, value: bool) -> Result<(), String> {
    engine::set_bool_param(handle, &key, value)
}

#[frb(sync)]
pub fn set_float_param(handle: u64, key: String, value: f32) -> Result<(), String> {
    engine::set_float_param(handle, &key, value)
}

#[frb(sync)]
pub fn set_vec4_param(handle: u64, key: String, value: Vec<f32>) -> Result<(), String> {
    let array: [f32; 4] = value
        .try_into()
        .map_err(|_| "set_vec4_param expects exactly 4 floats".to_string())?;
    engine::set_vec4_param(handle, &key, array)
}

#[frb(sync)]
pub fn invoke_command(handle: u64, command: String, payload: String) -> Result<(), String> {
    engine::invoke_command(handle, &command, &payload)
}

#[frb(sync)]
pub fn get_backend_info(handle: u64) -> Result<BackendInfo, String> {
    engine::renderer_backend_info(handle)
}

#[frb(sync)]
pub fn resize_renderer(handle: u64, width: u32, height: u32) -> Result<(), String> {
    engine::resize_renderer(handle, width, height)
}

/// Attach a Metal texture to the renderer (macOS / iOS only).
///
/// `mtl_texture_ptr` is the raw pointer value of an `id<MTLTexture>` cast to
/// `usize`. The native Swift bridge creates the texture and passes its address
/// back to Dart, which forwards it here so Rust can render into it.
#[frb(sync)]
pub fn attach_metal_texture(
    handle: u64,
    mtl_texture_ptr: usize,
    width: u32,
    height: u32,
    bytes_per_row: u32,
) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        use std::ffi::c_void;
        engine::attach_metal_texture(
            handle,
            mtl_texture_ptr as *mut c_void,
            width,
            height,
            bytes_per_row,
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        let _ = (handle, mtl_texture_ptr, width, height, bytes_per_row);
        Err("Metal not available on this platform".to_string())
    }
}

/// Create a DXGI shared-handle present surface (Windows only).
///
/// Returns the raw HANDLE value cast to `usize`. The native Windows bridge
/// receives this value via the method channel and passes it to the Flutter
/// GPU surface texture system.
#[frb(sync)]
pub fn create_dxgi_surface(handle: u64, width: u32, height: u32) -> Result<usize, String> {
    #[cfg(target_os = "windows")]
    {
        engine::create_dxgi_surface(handle, width, height)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (handle, width, height);
        Err("DXGI not available on this platform".to_string())
    }
}

/// Ensure the Linux Vulkan DMA-BUF present target exists and is the right size.
#[frb(sync)]
pub fn ensure_linux_present(handle: u64, width: u32, height: u32) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        engine::ensure_linux_present(handle, width, height)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (handle, width, height);
        Err("Linux DMA-BUF not available on this platform".to_string())
    }
}

/// Export the current frame as a DMA-BUF file descriptor (Linux only).
///
/// The native Linux bridge receives these values via the method channel and
/// uses them to import the buffer into an EGL image / GL texture.
#[frb(sync)]
pub fn export_dmabuf(handle: u64) -> Result<Option<DmaBufExport>, String> {
    #[cfg(target_os = "linux")]
    {
        engine::export_dmabuf(handle).map(|opt| {
            opt.map(|info| DmaBufExport {
                fd: info.fd,
                width: info.width,
                height: info.height,
                stride: info.stride,
                offset: info.offset,
                fourcc: info.fourcc,
                modifier_low: (info.modifier & 0xFFFF_FFFF) as u32,
                modifier_high: (info.modifier >> 32) as u32,
            })
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = handle;
        Ok(None)
    }
}

/// Returns `true` if DMA-BUF export is supported by the current Vulkan device.
#[frb(sync)]
pub fn linux_dmabuf_supported(handle: u64) -> bool {
    #[cfg(target_os = "linux")]
    {
        engine::linux_dmabuf_supported(handle).unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = handle;
        false
    }
}
