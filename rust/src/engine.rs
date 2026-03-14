use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

#[cfg(target_os = "linux")]
use ash::vk;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use metal::foreign_types::ForeignType;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use wgpu_hal::api::Metal;
#[cfg(target_os = "linux")]
use wgpu_hal::api::Vulkan;

use crate::api::BackendInfo;
#[cfg(target_os = "linux")]
use crate::linux_dma_buf;
use crate::present::{self, PresentTextureTarget};
use crate::scene::{Scene, SceneRenderArgs};
use crate::scenes;

pub(crate) const BACKEND_UNKNOWN: u8 = 0;
pub(crate) const BACKEND_METAL: u8 = 1;
pub(crate) const BACKEND_DX12: u8 = 2;
pub(crate) const BACKEND_VULKAN: u8 = 3;

static DEVICE_CONTEXT: OnceLock<Result<EngineDeviceContext, String>> = OnceLock::new();
static RENDERERS: OnceLock<Mutex<HashMap<u64, Arc<Mutex<Renderer>>>>> = OnceLock::new();
static NEXT_HANDLE: OnceLock<Mutex<u64>> = OnceLock::new();

struct EngineDeviceContext {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    backend: u8,
    backend_name: String,
    driver: String,
    device_name: String,
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    mtl_device_ptr: usize,
}

pub(crate) struct Renderer {
    ctx: &'static EngineDeviceContext,
    width: u32,
    height: u32,
    present: Option<PresentTextureTarget>,
    scene: Box<dyn Scene>,
    animation_running: bool,
    last_frame_at: Instant,
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

fn backend_code(backend: wgpu::Backend) -> u8 {
    match backend {
        wgpu::Backend::Metal => BACKEND_METAL,
        wgpu::Backend::Dx12 => BACKEND_DX12,
        wgpu::Backend::Vulkan => BACKEND_VULKAN,
        _ => BACKEND_UNKNOWN,
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn mtl_device_ptr(device: &wgpu::Device) -> *mut c_void {
    let result = unsafe {
        device.as_hal::<Metal, _, _>(|hal_device| {
            hal_device.map(|hal_device| {
                let raw_device = hal_device.raw_device().lock();
                raw_device.as_ptr() as *mut c_void
            })
        })
    };
    result.flatten().unwrap_or(std::ptr::null_mut())
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
#[allow(dead_code)]
fn mtl_device_ptr(_device: &wgpu::Device) -> *mut c_void {
    std::ptr::null_mut()
}

#[cfg(target_os = "linux")]
fn request_linux_vulkan_device_with_dmabuf(
    adapter: &wgpu::Adapter,
    required_features: wgpu::Features,
    required_limits: &wgpu::Limits,
) -> Result<(wgpu::Device, wgpu::Queue), String> {
    use std::ffi::CStr;
    unsafe {
        adapter.as_hal::<Vulkan, _, _>(|hal_adapter| {
            let Some(hal_adapter) = hal_adapter else {
                return Err("wgpu: Linux Vulkan HAL adapter unavailable".to_string());
            };

            let mut enabled_extensions = hal_adapter.required_device_extensions(required_features);
            for extension in [
                ash::extensions::khr::ExternalMemoryFd::name(),
                vk::ExtExternalMemoryDmaBufFn::name(),
                vk::ExtImageDrmFormatModifierFn::name(),
            ] {
                if !enabled_extensions.contains(&extension) {
                    enabled_extensions.push(extension);
                }
            }

            let available_extensions = hal_adapter
                .shared_instance()
                .raw_instance()
                .enumerate_device_extension_properties(hal_adapter.raw_physical_device())
                .map_err(|err| {
                    format!("wgpu: enumerate_device_extension_properties failed: {err:?}")
                })?;
            for extension in [
                ash::extensions::khr::ExternalMemoryFd::name(),
                vk::ExtExternalMemoryDmaBufFn::name(),
                vk::ExtImageDrmFormatModifierFn::name(),
            ] {
                let wanted = extension.to_string_lossy();
                let found = available_extensions.iter().any(|property| {
                    CStr::from_ptr(property.extension_name.as_ptr()).to_string_lossy() == wanted
                });
                if !found {
                    return Err(format!(
                        "wgpu: Vulkan device extension {wanted} is not available"
                    ));
                }
            }

            let mut enabled_phd_features =
                hal_adapter.physical_device_features(&enabled_extensions, required_features);

            let family_index = 0u32;
            let family_info = vk::DeviceQueueCreateInfo::builder()
                .queue_family_index(family_index)
                .queue_priorities(&[1.0])
                .build();
            let family_infos = [family_info];
            let extension_ptrs = enabled_extensions
                .iter()
                .map(|extension| extension.as_ptr())
                .collect::<Vec<_>>();

            let pre_info = vk::DeviceCreateInfo::builder()
                .queue_create_infos(&family_infos)
                .enabled_extension_names(&extension_ptrs);
            let info = enabled_phd_features
                .add_to_device_create_builder(pre_info)
                .build();
            let raw_device = hal_adapter
                .shared_instance()
                .raw_instance()
                .create_device(hal_adapter.raw_physical_device(), &info, None)
                .map_err(|err| format!("wgpu: vkCreateDevice failed: {err:?}"))?;

            let open_device = hal_adapter
                .device_from_raw(
                    raw_device,
                    true,
                    &enabled_extensions,
                    required_features,
                    family_index,
                    0,
                )
                .map_err(|err| {
                    format!("wgpu: device_from_raw failed for Linux Vulkan DMA-BUF: {err:?}")
                })?;

            adapter
                .create_device_from_hal(
                    open_device,
                    &wgpu::DeviceDescriptor {
                        label: Some("flutter_wgpu_texture device"),
                        required_features,
                        required_limits: required_limits.clone(),
                    },
                    None,
                )
                .map_err(|err| {
                    format!("wgpu: create_device_from_hal failed for Linux Vulkan DMA-BUF: {err:?}")
                })
        })
    }
}

fn device_context() -> Result<&'static EngineDeviceContext, String> {
    DEVICE_CONTEXT
        .get_or_init(|| {
            let backends = if cfg!(any(target_os = "macos", target_os = "ios")) {
                wgpu::Backends::METAL
            } else if cfg!(target_os = "windows") {
                wgpu::Backends::DX12
            } else {
                wgpu::Backends::PRIMARY
            };

            let instance = Arc::new(wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends,
                ..Default::default()
            }));
            let adapter =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                }))
                .ok_or_else(|| "wgpu: no compatible adapter found".to_string())?;
            let info = adapter.get_info();
            let adapter = Arc::new(adapter);
            let required_features = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
            let limits = adapter.limits();
            let (device, queue) = {
                #[cfg(target_os = "linux")]
                {
                    if info.backend == wgpu::Backend::Vulkan {
                        request_linux_vulkan_device_with_dmabuf(
                            adapter.as_ref(),
                            required_features,
                            &limits,
                        )?
                    } else {
                        pollster::block_on(adapter.request_device(
                            &wgpu::DeviceDescriptor {
                                label: Some("flutter_wgpu_texture device"),
                                required_features,
                                required_limits: limits,
                            },
                            None,
                        ))
                        .map_err(|err| format!("wgpu: request_device failed: {err:?}"))?
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    pollster::block_on(adapter.request_device(
                        &wgpu::DeviceDescriptor {
                            label: Some("flutter_wgpu_texture device"),
                            required_features,
                            required_limits: limits,
                        },
                        None,
                    ))
                    .map_err(|err| format!("wgpu: request_device failed: {err:?}"))?
                }
            };
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            let mtl_ptr = mtl_device_ptr(&device) as usize;
            Ok(EngineDeviceContext {
                device: Arc::new(device),
                queue: Arc::new(queue),
                backend: backend_code(info.backend),
                backend_name: format!("{:?}", info.backend),
                driver: info.driver,
                device_name: info.name,
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                mtl_device_ptr: mtl_ptr,
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl Renderer {
    fn new(width: u32, height: u32, scene_type: &str) -> Result<Self, String> {
        let ctx = device_context()?;
        let device = ctx.device.as_ref();
        let scene = scenes::scene_for_type(scene_type, device, width.max(1), height.max(1))?;

        Ok(Self {
            ctx,
            width: width.max(1),
            height: height.max(1),
            present: None,
            scene,
            animation_running: true,
            last_frame_at: Instant::now(),
            clear_color: [0.05, 0.1, 0.15, 1.0],
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.scene
            .resize(self.ctx.device.as_ref(), self.width, self.height);
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    fn attach_metal_texture(
        &mut self,
        mtl_texture_ptr: *mut c_void,
        width: u32,
        height: u32,
        bytes_per_row: u32,
    ) -> Result<(), String> {
        self.resize(width, height);
        self.present = present::attach_present_texture(
            self.ctx.device.as_ref(),
            mtl_texture_ptr,
            width,
            height,
            bytes_per_row,
        );
        if self.present.is_some() {
            Ok(())
        } else {
            Err("failed to attach Metal present texture".to_string())
        }
    }

    #[cfg(target_os = "windows")]
    fn create_dxgi_surface(&mut self, width: u32, height: u32) -> Result<usize, String> {
        self.resize(width, height);
        let mut target =
            present::create_dxgi_shared_present_target(self.ctx.device.as_ref(), width, height)?;
        let handle = target
            .take_dxgi_handle()
            .ok_or_else(|| "DXGI present target missing shared handle".to_string())?;
        self.present = Some(target);
        Ok(handle)
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
        match key {
            "animation_running" => self.animation_running = value,
            _ => self.scene.set_bool_param(key, value),
        }
    }

    fn set_float_param(&mut self, key: &str, value: f32) {
        self.scene.set_float_param(key, value);
    }

    fn set_vec4_param(&mut self, key: &str, value: [f32; 4]) {
        match key {
            "background_color" => self.clear_color = value,
            _ => self.scene.set_vec4_param(key, value),
        }
    }

    fn invoke_command(&mut self, command: &str, payload: &str) {
        if command == "reset_scene" {
            self.animation_running = true;
            self.clear_color = [0.05, 0.1, 0.15, 1.0];
        }
        let _ = self.scene.invoke_command(command, payload);
    }

    fn render(&mut self) -> Result<bool, String> {
        let Some(target) = self.present.take() else {
            return Ok(false);
        };
        let texture = target.render_texture();
        let view = &target.render_view;
        let width = target.width;
        let height = target.height;

        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_frame_at)
            .as_secs_f32();
        self.last_frame_at = now;

        let mut encoder =
            self.ctx
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("flutter_wgpu_texture frame"),
                });

        let args = SceneRenderArgs {
            device: self.ctx.device.as_ref(),
            queue: self.ctx.queue.as_ref(),
            view,
            width,
            height,
            dt,
            animation_running: self.animation_running,
            clear_color: self.clear_color,
        };
        self.scene.render(&args, &mut encoder)?;

        if let Some(shared) = target.shared_texture() {
            encoder.copy_texture_to_texture(
                wgpu::ImageCopyTexture {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::ImageCopyTexture {
                    texture: shared,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }

        self.ctx.queue.submit(Some(encoder.finish()));
        self.ctx.device.poll(wgpu::Maintain::Wait);
        self.present = Some(target);
        Ok(true)
    }
}

pub(crate) fn engine_create(width: u32, height: u32, scene_type: &str) -> Result<u64, String> {
    let renderer = Renderer::new(width.max(1), height.max(1), scene_type)?;
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

#[cfg(target_os = "windows")]
pub(crate) fn create_dxgi_surface(handle: u64, width: u32, height: u32) -> Result<usize, String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .create_dxgi_surface(width, height)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn attach_metal_texture(
    handle: u64,
    mtl_texture_ptr: *mut c_void,
    width: u32,
    height: u32,
    bytes_per_row: u32,
) -> Result<(), String> {
    let renderer =
        lookup_renderer(handle).ok_or_else(|| "renderer handle not found".to_string())?;
    let result = renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .attach_metal_texture(mtl_texture_ptr, width, height, bytes_per_row);
    result
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
    renderer
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .ensure_linux_present(width, height)
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
    Ok(linux_dma_buf::dma_buf_supported(
        renderer.ctx.device.as_ref(),
    ))
}

pub(crate) fn backend_code_for_handle(handle: u64) -> u8 {
    lookup_renderer(handle)
        .and_then(|renderer| renderer.lock().ok().map(|renderer| renderer.ctx.backend))
        .unwrap_or(BACKEND_UNKNOWN)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn mtl_device_ptr_for_handle(handle: u64) -> *mut c_void {
    lookup_renderer(handle)
        .and_then(|renderer| {
            renderer
                .lock()
                .ok()
                .map(|renderer| renderer.ctx.mtl_device_ptr as *mut c_void)
        })
        .unwrap_or(std::ptr::null_mut())
}
