use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

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

pub(crate) const BACKEND_UNKNOWN: u8 = 0;
pub(crate) const BACKEND_METAL: u8 = 1;
pub(crate) const BACKEND_DX12: u8 = 2;
pub(crate) const BACKEND_VULKAN: u8 = 3;

static DEVICE_CONTEXT: OnceLock<Result<EngineDeviceContext, String>> = OnceLock::new();
static RENDERERS: OnceLock<Mutex<HashMap<u64, Arc<Mutex<Renderer>>>>> = OnceLock::new();
static NEXT_HANDLE: OnceLock<Mutex<u64>> = OnceLock::new();

const SHADER: &str = r#"
struct Uniforms {
  mvp: mat4x4<f32>,
  model: mat4x4<f32>,
  cube_color: vec4<f32>,
  clear_color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct VsIn {
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
};

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
};

@vertex
fn vs_main(input: VsIn) -> VsOut {
  var out: VsOut;
  out.position = uniforms.mvp * vec4<f32>(input.position, 1.0);
  out.normal = (uniforms.model * vec4<f32>(input.normal, 0.0)).xyz;
  return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
  let light = normalize(vec3<f32>(0.45, 0.7, 0.55));
  let ndotl = max(dot(normalize(input.normal), light), 0.0);
  let intensity = 0.35 + 0.65 * ndotl;
  return vec4<f32>(uniforms.cube_color.rgb * intensity, uniforms.cube_color.a);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    cube_color: [f32; 4],
    clear_color: [f32; 4],
}

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
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    animation_running: bool,
    rotation_enabled: bool,
    rotation_speed: f32,
    angle: f32,
    last_frame_at: Instant,
    cube_color: [f32; 4],
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

fn cube_vertices() -> (Vec<Vertex>, Vec<u16>) {
    let p = [
        [-1.0, -1.0, 1.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
        [-1.0, -1.0, -1.0],
        [1.0, -1.0, -1.0],
        [1.0, 1.0, -1.0],
        [-1.0, 1.0, -1.0],
    ];
    let faces = [
        ([0, 1, 2, 3], [0.0, 0.0, 1.0]),
        ([5, 4, 7, 6], [0.0, 0.0, -1.0]),
        ([4, 0, 3, 7], [-1.0, 0.0, 0.0]),
        ([1, 5, 6, 2], [1.0, 0.0, 0.0]),
        ([3, 2, 6, 7], [0.0, 1.0, 0.0]),
        ([4, 5, 1, 0], [0.0, -1.0, 0.0]),
    ];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (face_idx, (idx, normal)) in faces.iter().enumerate() {
        let base = (face_idx * 4) as u16;
        for vertex_index in idx {
            vertices.push(Vertex {
                position: p[*vertex_index],
                normal: *normal,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

fn create_depth_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("flutter_wgpu_texture depth"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth24Plus,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = depth.create_view(&wgpu::TextureViewDescriptor::default());
    (depth, view)
}

impl Renderer {
    fn new(width: u32, height: u32) -> Result<Self, String> {
        let ctx = device_context()?;
        let device = ctx.device.as_ref();

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flutter_wgpu_texture shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flutter_wgpu_texture uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flutter_wgpu_texture bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flutter_wgpu_texture bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flutter_wgpu_texture pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flutter_wgpu_texture pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24Plus,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
        });

        let (vertices, indices) = cube_vertices();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flutter_wgpu_texture cube vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flutter_wgpu_texture cube indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let (depth_texture, depth_view) = create_depth_texture(device, width.max(1), height.max(1));

        Ok(Self {
            ctx,
            width,
            height,
            present: None,
            depth_texture,
            depth_view,
            pipeline,
            uniform_buffer,
            bind_group,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            animation_running: true,
            rotation_enabled: true,
            rotation_speed: 1.2,
            angle: 0.0,
            last_frame_at: Instant::now(),
            cube_color: [1.0, 0.9, 0.1, 1.0],
            clear_color: [0.05, 0.2, 0.85, 1.0],
        })
    }

    fn recreate_depth(&mut self) {
        let (texture, view) = create_depth_texture(
            self.ctx.device.as_ref(),
            self.width.max(1),
            self.height.max(1),
        );
        self.depth_texture = texture;
        self.depth_view = view;
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.recreate_depth();
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
            "rotation_enabled" => self.rotation_enabled = value,
            "animation_running" => self.animation_running = value,
            _ => {}
        }
    }

    fn set_float_param(&mut self, key: &str, value: f32) {
        match key {
            "rotation_speed" => self.rotation_speed = value,
            "angle" => self.angle = value,
            _ => {}
        }
    }

    fn set_vec4_param(&mut self, key: &str, value: [f32; 4]) {
        match key {
            "cube_color" => self.cube_color = value,
            "background_color" => self.clear_color = value,
            _ => {}
        }
    }

    fn invoke_command(&mut self, command: &str, _payload: &str) {
        if command == "reset_scene" {
            self.animation_running = true;
            self.rotation_enabled = true;
            self.rotation_speed = 1.2;
            self.angle = 0.0;
            self.cube_color = [1.0, 0.9, 0.1, 1.0];
            self.clear_color = [0.05, 0.2, 0.85, 1.0];
        }
    }

    fn render(&mut self) -> Result<bool, String> {
        let Some(target) = self.present.as_ref() else {
            return Ok(false);
        };
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_frame_at)
            .as_secs_f32();
        self.last_frame_at = now;
        if self.animation_running && self.rotation_enabled {
            self.angle += dt * self.rotation_speed;
        }

        let aspect = (self.width.max(1) as f32) / (self.height.max(1) as f32);
        let model = Mat4::from_rotation_x(0.65) * Mat4::from_rotation_y(self.angle);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 4.6), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh_gl(45f32.to_radians(), aspect, 0.1, 100.0);
        let uniforms = Uniforms {
            mvp: (proj * view * model).to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            cube_color: self.cube_color,
            clear_color: self.clear_color,
        };
        self.ctx
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flutter_wgpu_texture frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("flutter_wgpu_texture render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.render_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.clear_color[0] as f64,
                            g: self.clear_color[1] as f64,
                            b: self.clear_color[2] as f64,
                            a: self.clear_color[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }

        if let Some(shared) = target.shared_texture() {
            encoder.copy_texture_to_texture(
                wgpu::ImageCopyTexture {
                    texture: target.render_texture(),
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
                    width: target.width,
                    height: target.height,
                    depth_or_array_layers: 1,
                },
            );
        }

        self.ctx.queue.submit(Some(encoder.finish()));
        self.ctx.device.poll(wgpu::Maintain::Wait);
        Ok(true)
    }
}

pub(crate) fn engine_create(width: u32, height: u32) -> Result<u64, String> {
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

use wgpu::util::DeviceExt;
