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

const CUBE_SHADER: &str = r#"
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

const PARTICLES_SHADER: &str = r#"
struct ParticleUniforms {
  viewport: vec4<f32>,
  dynamics: vec4<f32>,
  color1: vec4<f32>,
  color2: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: ParticleUniforms;

struct QuadIn {
  @location(0) corner: vec2<f32>,
  @location(1) position: vec2<f32>,
  @location(2) velocity: vec2<f32>,
  @location(3) life: f32,
  @location(4) seed: f32,
};

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) life: f32,
  @location(1) local: vec2<f32>,
};

@vertex
fn vs_main(input: QuadIn) -> VsOut {
  var out: VsOut;
  let t = uniforms.viewport.z * (0.45 + input.seed * 1.35);
  let spiral = vec2<f32>(cos(t * 0.9 + input.seed * 11.0), sin(t * 1.1 + input.seed * 7.0));
  let drift = input.velocity * sin(t * 1.7 + input.seed * 9.0) * 140.0 * uniforms.dynamics.x;
  let pulse = spiral * (14.0 + 22.0 * input.life) * uniforms.dynamics.x;
  let center = uniforms.viewport.xy * 0.5 + input.position + drift + pulse;
  let size = uniforms.viewport.w * (0.85 + input.life * 1.35);
  let world = center + input.corner * size;
  let ndc_pos = world / uniforms.viewport.xy * 2.0 - 1.0;
  out.position = vec4<f32>(ndc_pos.x, -ndc_pos.y, 0.0, 1.0);
  out.life = input.life;
  out.local = input.corner;
  return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
  let radius = length(input.local);
  let mask = 1.0 - smoothstep(0.0, 1.0, radius);
  let alpha = mask;
  return vec4<f32>(1.0, 1.0, 1.0, alpha);
}
"#;

const SHADER_PLAYGROUND_SHADER: &str = r#"
struct ShaderUniforms {
  viewport: vec4<f32>,
  pointer: vec4<f32>,
  tuning: vec4<f32>,
  primary_color: vec4<f32>,
  secondary_color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: ShaderUniforms;

struct ShaderVertexIn {
  @location(0) position: vec2<f32>,
  @location(1) uv: vec2<f32>,
};

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(input: ShaderVertexIn) -> VsOut {
  var out: VsOut;
  out.position = vec4<f32>(input.position, 0.0, 1.0);
  out.uv = input.uv;
  return out;
}

@fragment
fn fs_main(input: VsOut) -> @location(0) vec4<f32> {
  let time = uniforms.viewport.z;
  let speed = uniforms.viewport.w;
  let noise_scale = uniforms.tuning.x;
  let distortion = uniforms.pointer.w;
  let uv = input.uv;
  let wave = 0.5 + 0.5 * sin(time * (0.6 + speed * 0.9) + uv.x * (3.0 + noise_scale * 2.2) - uv.y * (2.0 + distortion * 3.0));
  let bands = 0.5 + 0.5 * sin(time * (0.8 + speed * 1.4) + uv.y * (5.0 + noise_scale * 3.0) + uv.x * distortion * 4.0);
  let ripple = 0.5 + 0.5 * sin((uv.x + uv.y) * (4.0 + noise_scale * 2.0) - time * (0.4 + speed * 0.7));
  let primary = uniforms.primary_color.rgb;
  let secondary = uniforms.secondary_color.rgb;
  let base = mix(primary, secondary, wave);
  let accent = mix(secondary, primary, ripple) * distortion * 0.25;
  let color = base * (0.45 + bands * 0.55) + accent + vec3<f32>(uv.x * 0.12, uv.y * 0.08, (1.0 - uv.x) * 0.12);
  return vec4<f32>(color, 1.0);
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParticleUniforms {
    viewport: [f32; 4],
    dynamics: [f32; 4],
    color1: [f32; 4],
    color2: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShaderUniforms {
    viewport: [f32; 4],
    pointer: [f32; 4],
    tuning: [f32; 4],
    primary_color: [f32; 4],
    secondary_color: [f32; 4],
}

struct CubeRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    rotation_enabled: bool,
    rotation_speed: f32,
    angle: f32,
    cube_color: [f32; 4],
}

struct ParticlesRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    time: f32,
    point_size: f32,
    motion_scale: f32,
    color1: [f32; 4],
    color2: [f32; 4],
}

struct ShaderPlaygroundRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    time: f32,
    speed: f32,
    noise_scale: f32,
    distortion: f32,
    pointer: [f32; 4],
    primary_color: [f32; 4],
    secondary_color: [f32; 4],
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneType {
    Cube,
    Particles,
    ShaderPlayground,
}

pub(crate) struct Renderer {
    ctx: &'static EngineDeviceContext,
    width: u32,
    height: u32,
    present: Option<PresentTextureTarget>,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    scene_type: SceneType,
    cube: CubeRenderer,
    particles: Option<ParticlesRenderer>,
    shader_playground: Option<ShaderPlaygroundRenderer>,
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

impl CubeRenderer {
    fn new(device: &wgpu::Device) -> Result<Self, String> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flutter_wgpu_texture cube shader"),
            source: wgpu::ShaderSource::Wgsl(CUBE_SHADER.into()),
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flutter_wgpu_texture cube uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flutter_wgpu_texture cube bind group layout"),
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
            label: Some("flutter_wgpu_texture cube bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flutter_wgpu_texture cube pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flutter_wgpu_texture cube pipeline"),
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

        Ok(Self {
            pipeline,
            uniform_buffer,
            bind_group,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            rotation_enabled: true,
            rotation_speed: 1.2,
            angle: 0.0,
            cube_color: [1.0, 0.9, 0.1, 1.0],
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParticleVertex {
    corner: [f32; 2],
    position: [f32; 2],
    velocity: [f32; 2],
    life: f32,
    seed: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShaderVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

impl ParticlesRenderer {
    fn new(device: &wgpu::Device, _width: u32, _height: u32) -> Result<Self, String> {
        const PARTICLE_COUNT: u32 = 2000;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flutter_wgpu_texture particles shader"),
            source: wgpu::ShaderSource::Wgsl(PARTICLES_SHADER.into()),
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flutter_wgpu_texture particles uniforms"),
            size: std::mem::size_of::<ParticleUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flutter_wgpu_texture particles bind group layout"),
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
            label: Some("flutter_wgpu_texture particles bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flutter_wgpu_texture particles pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flutter_wgpu_texture particles pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ParticleVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32, 4 => Float32],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
        });

        let corners = [
            [-1.0, -1.0],
            [1.0, -1.0],
            [1.0, 1.0],
            [-1.0, -1.0],
            [1.0, 1.0],
            [-1.0, 1.0],
        ];
        let mut particles = Vec::with_capacity(PARTICLE_COUNT as usize * 6);
        for i in 0..PARTICLE_COUNT {
            let angle = (i as f32 / PARTICLE_COUNT as f32) * std::f32::consts::PI * 2.0;
            let band = i as f32 / PARTICLE_COUNT as f32;
            let radius = 70.0 + band * 260.0;
            let position = [angle.cos() * radius, angle.sin() * radius];
            let velocity = [
                angle.cos() * (0.45 + band * 0.9),
                angle.sin() * (0.45 + band * 0.9),
            ];
            let life = band;
            let seed = band.sqrt();
            for corner in corners {
                particles.push(ParticleVertex {
                    corner,
                    position,
                    velocity,
                    life,
                    seed,
                });
            }
        }
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flutter_wgpu_texture particles vertices"),
            contents: bytemuck::cast_slice(&particles),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Ok(Self {
            pipeline,
            uniform_buffer,
            bind_group,
            vertex_buffer,
            vertex_count: particles.len() as u32,
            time: 0.0,
            point_size: 14.0,
            motion_scale: 1.0,
            color1: [1.0, 0.4, 0.1, 1.0],
            color2: [0.1, 0.6, 1.0, 1.0],
        })
    }
}

impl ShaderPlaygroundRenderer {
    fn new(device: &wgpu::Device) -> Result<Self, String> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flutter_wgpu_texture shader playground shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_PLAYGROUND_SHADER.into()),
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flutter_wgpu_texture shader playground uniforms"),
            size: std::mem::size_of::<ShaderUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flutter_wgpu_texture shader playground bind group layout"),
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
            label: Some("flutter_wgpu_texture shader playground bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flutter_wgpu_texture shader playground pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flutter_wgpu_texture shader playground pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ShaderVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2],
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
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
        });

        let vertices = [
            ShaderVertex {
                position: [-1.0, -1.0],
                uv: [0.0, 1.0],
            },
            ShaderVertex {
                position: [1.0, -1.0],
                uv: [1.0, 1.0],
            },
            ShaderVertex {
                position: [1.0, 1.0],
                uv: [1.0, 0.0],
            },
            ShaderVertex {
                position: [-1.0, -1.0],
                uv: [0.0, 1.0],
            },
            ShaderVertex {
                position: [1.0, 1.0],
                uv: [1.0, 0.0],
            },
            ShaderVertex {
                position: [-1.0, 1.0],
                uv: [0.0, 0.0],
            },
        ];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flutter_wgpu_texture shader playground vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Ok(Self {
            pipeline,
            uniform_buffer,
            bind_group,
            vertex_buffer,
            vertex_count: vertices.len() as u32,
            time: 0.0,
            speed: 1.0,
            noise_scale: 2.4,
            distortion: 1.0,
            pointer: [0.5, 0.5, 0.0, 1.0],
            primary_color: [1.0, 0.46, 0.16, 1.0],
            secondary_color: [0.14, 0.77, 1.0, 1.0],
        })
    }
}

impl Renderer {
    fn new(width: u32, height: u32, scene_type: SceneType) -> Result<Self, String> {
        let ctx = device_context()?;
        let device = ctx.device.as_ref();

        let (depth_texture, depth_view) = create_depth_texture(device, width.max(1), height.max(1));

        let (cube, particles, shader_playground) = match scene_type {
            SceneType::Cube => (CubeRenderer::new(device)?, None, None),
            SceneType::Particles => (
                CubeRenderer::new(device)?,
                Some(ParticlesRenderer::new(device, width, height)?),
                None,
            ),
            SceneType::ShaderPlayground => (
                CubeRenderer::new(device)?,
                None,
                Some(ShaderPlaygroundRenderer::new(device)?),
            ),
        };

        Ok(Self {
            ctx,
            width,
            height,
            present: None,
            depth_texture,
            depth_view,
            scene_type,
            cube,
            particles,
            shader_playground,
            animation_running: true,
            last_frame_at: Instant::now(),
            clear_color: [0.05, 0.1, 0.15, 1.0],
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
            "animation_running" => self.animation_running = value,
            _ => {}
        }
        if self.particles.is_some() {
            match key {
                "animation_running" => self.animation_running = value,
                _ => {}
            }
        }
    }

    fn set_float_param(&mut self, key: &str, value: f32) {
        if let Some(ref mut cube) = Some(&mut self.cube) {
            match key {
                "rotation_speed" => cube.rotation_speed = value,
                "angle" => cube.angle = value,
                _ => {}
            }
        }
        if let Some(ref mut particles) = self.particles {
            match key {
                "point_size" => particles.point_size = value,
                "motion_scale" => particles.motion_scale = value,
                "time" => particles.time = value,
                _ => {}
            }
        }
        if let Some(ref mut shader) = self.shader_playground {
            match key {
                "speed" => shader.speed = value,
                "noise_scale" => shader.noise_scale = value,
                "distortion" => shader.distortion = value,
                "time" => shader.time = value,
                _ => {}
            }
        }
    }

    fn set_vec4_param(&mut self, key: &str, value: [f32; 4]) {
        match key {
            "background_color" => self.clear_color = value,
            _ => {}
        }
        if let Some(ref mut cube) = Some(&mut self.cube) {
            match key {
                "cube_color" => cube.cube_color = value,
                _ => {}
            }
        }
        if let Some(ref mut particles) = self.particles {
            match key {
                "color1" => particles.color1 = value,
                "color2" => particles.color2 = value,
                _ => {}
            }
        }
        if let Some(ref mut shader) = self.shader_playground {
            match key {
                "primary_color" => shader.primary_color = value,
                "secondary_color" => shader.secondary_color = value,
                "pointer" => shader.pointer = value,
                _ => {}
            }
        }
    }

    fn invoke_command(&mut self, command: &str, _payload: &str) {
        if command == "reset_scene" {
            self.animation_running = true;
            self.clear_color = [0.05, 0.1, 0.15, 1.0];
            if let Some(ref mut cube) = Some(&mut self.cube) {
                cube.rotation_enabled = true;
                cube.rotation_speed = 1.2;
                cube.angle = 0.0;
                cube.cube_color = [1.0, 0.9, 0.1, 1.0];
            }
            if let Some(ref mut particles) = self.particles {
                particles.time = 0.0;
                particles.point_size = 14.0;
                particles.motion_scale = 1.0;
                particles.color1 = [1.0, 0.4, 0.1, 1.0];
                particles.color2 = [0.1, 0.6, 1.0, 1.0];
            }
            if let Some(ref mut shader) = self.shader_playground {
                shader.time = 0.0;
                shader.speed = 1.0;
                shader.noise_scale = 2.4;
                shader.distortion = 1.0;
                shader.pointer = [0.5, 0.5, 0.0, 1.0];
                shader.primary_color = [1.0, 0.46, 0.16, 1.0];
                shader.secondary_color = [0.14, 0.77, 1.0, 1.0];
            }
        }
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

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flutter_wgpu_texture frame"),
            });

        match self.scene_type {
            SceneType::Cube => {
                self.render_cube(&mut encoder, view, dt)?;
            }
            SceneType::Particles => {
                self.render_particles(&mut encoder, view, dt)?;
            }
            SceneType::ShaderPlayground => {
                self.render_shader_playground(&mut encoder, view, dt)?;
            }
        }

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

    fn render_cube(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        dt: f32,
    ) -> Result<(), String> {
        let cube = &mut self.cube;
        if self.animation_running && cube.rotation_enabled {
            cube.angle += dt * cube.rotation_speed;
        }

        let aspect = (self.width.max(1) as f32) / (self.height.max(1) as f32);
        let model = Mat4::from_rotation_x(0.65) * Mat4::from_rotation_y(cube.angle);
        let camera = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 4.6), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh_gl(45f32.to_radians(), aspect, 0.1, 100.0);
        let uniforms = Uniforms {
            mvp: (proj * camera * model).to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            cube_color: cube.cube_color,
            clear_color: self.clear_color,
        };
        self.ctx
            .queue
            .write_buffer(&cube.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flutter_wgpu_texture cube render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: view,
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
        pass.set_pipeline(&cube.pipeline);
        pass.set_bind_group(0, &cube.bind_group, &[]);
        pass.set_vertex_buffer(0, cube.vertex_buffer.slice(..));
        pass.set_index_buffer(cube.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..cube.index_count, 0, 0..1);
        Ok(())
    }

    fn render_particles(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        dt: f32,
    ) -> Result<(), String> {
        let particles = self.particles.as_mut().unwrap();
        if self.animation_running {
            particles.time += dt;
        }

        let uniforms = ParticleUniforms {
            viewport: [
                self.width as f32,
                self.height as f32,
                particles.time,
                particles.point_size,
            ],
            dynamics: [particles.motion_scale, 0.0, 0.0, 0.0],
            color1: particles.color1,
            color2: particles.color2,
        };
        self.ctx
            .queue
            .write_buffer(&particles.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flutter_wgpu_texture particles render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: view,
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
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&particles.pipeline);
        pass.set_bind_group(0, &particles.bind_group, &[]);
        pass.set_vertex_buffer(0, particles.vertex_buffer.slice(..));
        pass.draw(0..particles.vertex_count, 0..1);
        Ok(())
    }

    fn render_shader_playground(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        dt: f32,
    ) -> Result<(), String> {
        let shader = self.shader_playground.as_mut().unwrap();
        if self.animation_running {
            shader.time += dt;
        }

        let uniforms = ShaderUniforms {
            viewport: [
                self.width as f32,
                self.height as f32,
                shader.time,
                shader.speed,
            ],
            pointer: [
                shader.pointer[0],
                shader.pointer[1],
                shader.pointer[2],
                shader.distortion,
            ],
            tuning: [shader.noise_scale, 0.0, 0.0, 0.0],
            primary_color: shader.primary_color,
            secondary_color: shader.secondary_color,
        };
        self.ctx
            .queue
            .write_buffer(&shader.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flutter_wgpu_texture shader playground render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: view,
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
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&shader.pipeline);
        pass.set_bind_group(0, &shader.bind_group, &[]);
        pass.set_vertex_buffer(0, shader.vertex_buffer.slice(..));
        pass.draw(0..shader.vertex_count, 0..1);
        Ok(())
    }
}

pub(crate) fn engine_create(width: u32, height: u32, scene_type: SceneType) -> Result<u64, String> {
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
