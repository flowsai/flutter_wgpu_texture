//! Renderer registry + device context.
//!
//! The actual rendering is performed by Bevy on a dedicated thread (see
//! `device`/`render_thread`). This module keeps the per-viewport handle
//! bookkeeping, the shared wgpu device borrowed from Bevy, and the Linux
//! DMA-BUF present/export path that bridges the rendered frame to Flutter.

pub(crate) mod device;
pub(crate) mod render_thread;

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};

use bevy::asset::AssetId;
use bevy::image::Image;

use crate::api::BackendInfo;
#[cfg(target_os = "linux")]
use crate::linux_dma_buf;
use crate::present::{self, PresentTextureTarget};
use render_thread::RenderCmd;

pub(crate) const BACKEND_UNKNOWN: u8 = 0;
pub(crate) const BACKEND_METAL: u8 = 1;
pub(crate) const BACKEND_DX12: u8 = 2;
pub(crate) const BACKEND_VULKAN: u8 = 3;

static RENDERERS: OnceLock<Mutex<HashMap<u64, Arc<Mutex<Renderer>>>>> = OnceLock::new();
static NEXT_HANDLE: OnceLock<Mutex<u64>> = OnceLock::new();

/// Borrowed view of the Bevy-owned wgpu device.
pub(crate) struct EngineDeviceContext {
    pub(crate) device: Arc<wgpu::Device>,
    #[allow(dead_code)]
    pub(crate) queue: Arc<wgpu::Queue>,
    pub(crate) backend: u8,
    pub(crate) backend_name: String,
    pub(crate) driver: String,
    pub(crate) device_name: String,
}

pub(crate) struct Renderer {
    ctx: &'static EngineDeviceContext,
    width: u32,
    height: u32,
    present: Option<PresentTextureTarget>,
    /// Bevy offscreen render-target image for this viewport.
    viewport_image: AssetId<Image>,
    animation_running: bool,
    clear_color: [f32; 4],
}

fn renderers() -> &'static Mutex<HashMap<u64, Arc<Mutex<Renderer>>>> {
    RENDERERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle() -> u64 {
    let mut guard = NEXT_HANDLE
        .get_or_init(|| Mutex::new(1))
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let handle = *guard;
    *guard += 1;
    handle
}

fn backend_code(name: &str) -> u8 {
    match name {
        "Metal" => BACKEND_METAL,
        "Dx12" => BACKEND_DX12,
        "Vulkan" => BACKEND_VULKAN,
        _ => BACKEND_UNKNOWN,
    }
}

/// Borrow the Bevy-owned device, starting the render thread on first use.
fn device_context() -> Result<&'static EngineDeviceContext, String> {
    static DEVICE_CONTEXT: OnceLock<Result<EngineDeviceContext, String>> = OnceLock::new();
    DEVICE_CONTEXT
        .get_or_init(|| {
            let gpu = device::ensure_started()?;
            Ok(EngineDeviceContext {
                device: gpu.device.clone(),
                queue: gpu.queue.clone(),
                backend: backend_code(&gpu.backend_name),
                backend_name: gpu.backend_name.clone(),
                driver: gpu.driver.clone(),
                device_name: gpu.device_name.clone(),
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl Renderer {
    fn new(width: u32, height: u32) -> Result<Self, String> {
        let ctx = device_context()?;

        // Create the viewport (camera + offscreen image) on the render thread.
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        device::send(RenderCmd::CreateViewport {
            width: width.max(1),
            height: height.max(1),
            reply: reply_tx,
        })?;
        let viewport_image = reply_rx
            .recv()
            .map_err(|_| "render thread did not return a viewport".to_string())?;

        Ok(Self {
            ctx,
            width: width.max(1),
            height: height.max(1),
            present: None,
            viewport_image,
            animation_running: true,
            clear_color: [0.05, 0.1, 0.15, 1.0],
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        let _ = device::send(RenderCmd::ResizeViewport {
            image: self.viewport_image,
            width: self.width,
            height: self.height,
        });
    }

    #[cfg(target_os = "linux")]
    fn ensure_linux_present(&mut self, width: u32, height: u32) -> Result<(), String> {
        let needs_recreate = match self.present.as_ref() {
            Some(target) => target.width != width || target.height != height,
            None => true,
        };
        if needs_recreate {
            self.resize(width, height);
            self.present = Some(present::create_linux_present_target(
                self.ctx.device.as_ref(),
                width,
                height,
            )?);
        }
        Ok(())
    }

    fn set_bool_param(&mut self, key: &str, value: bool) {
        if key == "animation_running" {
            self.animation_running = value;
        }
    }

    fn set_float_param(&mut self, _key: &str, _value: f32) {}

    fn set_vec4_param(&mut self, key: &str, value: [f32; 4]) {
        if key == "background_color" {
            self.clear_color = value;
        }
    }

    fn invoke_command(&mut self, command: &str, _payload: &str) {
        if command == "reset_scene" {
            self.animation_running = true;
            self.clear_color = [0.05, 0.1, 0.15, 1.0];
        }
    }

    fn set_scene(&mut self, json: &str) {
        let _ = device::send(RenderCmd::SetScene {
            json: json.to_string(),
        });
    }

    fn list_component_types(&mut self) -> Result<String, String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        device::send(RenderCmd::ListComponentTypes { reply: reply_tx })?;
        reply_rx
            .recv()
            .map_err(|_| "render thread dropped list_component_types reply".to_string())
    }

    fn describe_component(&mut self, type_path: &str) -> Result<Option<String>, String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        device::send(RenderCmd::DescribeComponent {
            type_path: type_path.to_string(),
            reply: reply_tx,
        })?;
        reply_rx
            .recv()
            .map_err(|_| "render thread dropped describe_component reply".to_string())
    }

    fn save_scene(&mut self, path: &str) -> Result<(), String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        device::send(RenderCmd::SaveScene {
            path: path.to_string(),
            reply: reply_tx,
        })?;
        reply_rx
            .recv()
            .map_err(|_| "render thread dropped save_scene reply".to_string())?
    }

    fn load_scene(&mut self, path: &str) -> Result<(), String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        device::send(RenderCmd::LoadScene {
            path: path.to_string(),
            reply: reply_tx,
        })?;
        reply_rx
            .recv()
            .map_err(|_| "render thread dropped load_scene reply".to_string())?
    }

    /// Raycast a viewport pixel; returns the hit editor id (blocking).
    fn pick(&mut self, x: f32, y: f32) -> Result<Option<String>, String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        device::send(RenderCmd::Pick {
            image: self.viewport_image,
            x,
            y,
            reply: reply_tx,
        })?;
        reply_rx
            .recv()
            .map_err(|_| "render thread dropped pick reply".to_string())
    }

    fn select_entity(&mut self, id: Option<String>) {
        let _ = device::send(RenderCmd::SelectEntity { id });
    }

    fn set_gizmo_mode(&mut self, mode: &str) {
        let _ = device::send(RenderCmd::SetGizmoMode {
            mode: mode.to_string(),
        });
    }

    fn set_play_mode(&mut self, mode: &str) {
        let _ = device::send(RenderCmd::SetPlayMode {
            mode: mode.to_string(),
        });
    }

    fn set_view_mode(&mut self, mode: &str) {
        let _ = device::send(RenderCmd::SetViewMode {
            mode: mode.to_string(),
        });
    }

    fn get_scene(&mut self) -> Result<String, String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        device::send(RenderCmd::GetScene { reply: reply_tx })?;
        reply_rx
            .recv()
            .map_err(|_| "render thread dropped get_scene reply".to_string())
    }

    fn camera_orbit(&mut self, dx: f32, dy: f32) {
        let _ = device::send(RenderCmd::CameraOrbit {
            image: self.viewport_image,
            dx,
            dy,
        });
    }

    fn camera_pan(&mut self, dx: f32, dy: f32) {
        let _ = device::send(RenderCmd::CameraPan {
            image: self.viewport_image,
            dx,
            dy,
        });
    }

    fn camera_zoom(&mut self, delta: f32) {
        let _ = device::send(RenderCmd::CameraZoom {
            image: self.viewport_image,
            delta,
        });
    }

    fn camera_look(&mut self, dx: f32, dy: f32) {
        let _ = device::send(RenderCmd::CameraLook {
            image: self.viewport_image,
            dx,
            dy,
        });
    }

    fn camera_fly(&mut self, forward: f32, right: f32, up: f32, dt: f32) {
        let _ = device::send(RenderCmd::CameraFly {
            image: self.viewport_image,
            forward,
            right,
            up,
            dt,
        });
    }

    fn drag_begin(&mut self, x: f32, y: f32) -> Result<bool, String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        device::send(RenderCmd::DragBegin {
            image: self.viewport_image,
            x,
            y,
            reply: reply_tx,
        })?;
        reply_rx
            .recv()
            .map_err(|_| "render thread dropped drag_begin reply".to_string())
    }

    fn drag_update(&mut self, x: f32, y: f32) -> Result<Option<render_thread::TransformOut>, String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        device::send(RenderCmd::DragUpdate {
            image: self.viewport_image,
            x,
            y,
            reply: reply_tx,
        })?;
        reply_rx
            .recv()
            .map_err(|_| "render thread dropped drag_update reply".to_string())
    }

    fn drag_end(&mut self) {
        let _ = device::send(RenderCmd::DragEnd);
    }

    fn set_hover(&mut self, x: f32, y: f32) {
        let _ = device::send(RenderCmd::SetHover {
            image: self.viewport_image,
            x,
            y,
        });
    }

    /// Render one frame and copy it into the present target's shared texture.
    fn render(&mut self) -> Result<bool, String> {
        let dst = self
            .present
            .as_ref()
            .and_then(|t| t.shared_texture().cloned());

        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        device::send(RenderCmd::RenderFrame {
            image: self.viewport_image,
            dst,
            width: self.width,
            height: self.height,
            reply: reply_tx,
        })?;
        reply_rx
            .recv()
            .map_err(|_| "render thread dropped frame reply".to_string())?
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = device::send(RenderCmd::DisposeViewport {
            image: self.viewport_image,
        });
    }
}

// ── Public registry API (called from api/mod.rs) ─────────────────────────────

pub(crate) fn engine_create(width: u32, height: u32, _scene_type: &str) -> Result<u64, String> {
    let renderer = Renderer::new(width.max(1), height.max(1))?;
    let handle = next_handle();
    renderers()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(handle, Arc::new(Mutex::new(renderer)));
    Ok(handle)
}

pub(crate) fn engine_dispose(handle: u64) {
    renderers()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .remove(&handle);
}

pub(crate) fn lookup_renderer(handle: u64) -> Option<Arc<Mutex<Renderer>>> {
    renderers()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .get(&handle)
        .cloned()
}

pub(crate) fn renderer_backend_info(handle: u64) -> Result<BackendInfo, String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let renderer = renderer.lock().unwrap_or_else(|err| err.into_inner());
    Ok(BackendInfo {
        backend: renderer.ctx.backend_name.clone(),
        device_name: renderer.ctx.device_name.clone(),
        driver: renderer.ctx.driver.clone(),
    })
}

pub(crate) fn render_frame(handle: u64) -> Result<bool, String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let result = renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .render();
    result
}

pub(crate) fn set_animation_running(handle: u64, running: bool) -> Result<(), String> {
    set_bool_param(handle, "animation_running", running)
}

pub(crate) fn set_bool_param(handle: u64, key: &str, value: bool) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .set_bool_param(key, value);
    Ok(())
}

pub(crate) fn set_float_param(handle: u64, key: &str, value: f32) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .set_float_param(key, value);
    Ok(())
}

pub(crate) fn set_vec4_param(handle: u64, key: &str, value: [f32; 4]) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .set_vec4_param(key, value);
    Ok(())
}

pub(crate) fn invoke_command(handle: u64, command: &str, payload: &str) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .invoke_command(command, payload);
    Ok(())
}

pub(crate) fn set_scene(handle: u64, json: &str) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .set_scene(json);
    Ok(())
}

pub(crate) fn list_component_types(handle: u64) -> Result<String, String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let result = renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .list_component_types();
    result
}

pub(crate) fn describe_component(handle: u64, type_path: &str) -> Result<Option<String>, String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let result = renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .describe_component(type_path);
    result
}

pub(crate) fn save_scene(handle: u64, path: &str) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let result = renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .save_scene(path);
    result
}

pub(crate) fn load_scene(handle: u64, path: &str) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let result = renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .load_scene(path);
    result
}

pub(crate) fn pick(handle: u64, x: f32, y: f32) -> Result<Option<String>, String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let result = renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .pick(x, y);
    result
}

pub(crate) fn select_entity(handle: u64, id: Option<String>) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .select_entity(id);
    Ok(())
}

pub(crate) fn set_gizmo_mode(handle: u64, mode: &str) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .set_gizmo_mode(mode);
    Ok(())
}

pub(crate) fn set_play_mode(handle: u64, mode: &str) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .set_play_mode(mode);
    Ok(())
}

pub(crate) fn set_view_mode(handle: u64, mode: &str) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .set_view_mode(mode);
    Ok(())
}

pub(crate) fn get_scene(handle: u64) -> Result<String, String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let result = renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .get_scene();
    result
}

macro_rules! camera_fn {
    ($name:ident, $method:ident, $($arg:ident: $ty:ty),*) => {
        pub(crate) fn $name(handle: u64, $($arg: $ty),*) -> Result<(), String> {
            let renderer =
                lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
            renderer
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .$method($($arg),*);
            Ok(())
        }
    };
}

camera_fn!(camera_orbit, camera_orbit, dx: f32, dy: f32);
camera_fn!(camera_pan, camera_pan, dx: f32, dy: f32);
camera_fn!(camera_zoom, camera_zoom, delta: f32);
camera_fn!(camera_look, camera_look, dx: f32, dy: f32);
camera_fn!(camera_fly, camera_fly, forward: f32, right: f32, up: f32, dt: f32);

pub(crate) fn drag_begin(handle: u64, x: f32, y: f32) -> Result<bool, String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let result = renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .drag_begin(x, y);
    result
}

pub(crate) fn drag_update(
    handle: u64,
    x: f32,
    y: f32,
) -> Result<Option<render_thread::TransformOut>, String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let result = renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .drag_update(x, y);
    result
}

pub(crate) fn drag_end(handle: u64) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .drag_end();
    Ok(())
}

pub(crate) fn set_hover(handle: u64, x: f32, y: f32) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .set_hover(x, y);
    Ok(())
}

pub(crate) fn resize_renderer(handle: u64, width: u32, height: u32) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .resize(width, height);
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn ensure_linux_present(handle: u64, width: u32, height: u32) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let result = renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .ensure_linux_present(width, height);
    result
}

#[cfg(target_os = "linux")]
pub(crate) fn export_dmabuf(handle: u64) -> Result<Option<linux_dma_buf::DmaBufInfo>, String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let renderer = renderer.lock().unwrap_or_else(|err| err.into_inner());
    let Some(target) = renderer.present.as_ref() else {
        return Ok(None);
    };
    let Some(image) = target.dma_buf_image() else {
        return Ok(None);
    };
    linux_dma_buf::export_dmabuf(renderer.ctx.device.as_ref(), image).map(Some)
}

#[cfg(target_os = "linux")]
pub(crate) fn linux_dmabuf_supported(handle: u64) -> Result<bool, String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let renderer = renderer.lock().unwrap_or_else(|err| err.into_inner());
    Ok(linux_dma_buf::dma_buf_supported(renderer.ctx.device.as_ref()))
}

pub(crate) fn backend_code_for_handle(handle: u64) -> u8 {
    lookup_renderer(handle)
        .and_then(|renderer| renderer.lock().ok().map(|renderer| renderer.ctx.backend))
        .unwrap_or(BACKEND_UNKNOWN)
}

// Metal / DXGI paths are not supported in the Bevy-shared-device build yet.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn attach_metal_texture(
    _handle: u64,
    _mtl_texture_ptr: *mut c_void,
    _width: u32,
    _height: u32,
    _bytes_per_row: u32,
) -> Result<(), String> {
    Err("Metal present not yet supported with Bevy renderer".to_string())
}

#[cfg(target_os = "windows")]
pub(crate) fn create_dxgi_surface(_handle: u64, _width: u32, _height: u32) -> Result<usize, String> {
    Err("DXGI present not yet supported with Bevy renderer".to_string())
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn mtl_device_ptr_for_handle(_handle: u64) -> *mut c_void {
    std::ptr::null_mut()
}

// Silence unused import warnings on non-linux where c_void is only used by stubs.
#[allow(unused_imports)]
use c_void as _;
