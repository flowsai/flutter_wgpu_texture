use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::scene::{Scene, SceneRenderArgs};

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

pub struct CubeScene {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    rotation_enabled: bool,
    rotation_speed: f32,
    angle: f32,
    cube_color: [f32; 4],
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

impl CubeScene {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Result<Self, String> {
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
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        let (depth_texture, depth_view) =
            create_depth_texture(device, width.max(1), height.max(1));

        Ok(Self {
            pipeline,
            uniform_buffer,
            bind_group,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            depth_texture,
            depth_view,
            rotation_enabled: true,
            rotation_speed: 1.2,
            angle: 0.0,
            cube_color: [1.0, 0.9, 0.1, 1.0],
        })
    }
}

impl Scene for CubeScene {
    fn render(
        &mut self,
        args: &SceneRenderArgs,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), String> {
        if args.animation_running && self.rotation_enabled {
            self.angle += args.dt * self.rotation_speed;
        }

        let aspect = (args.width.max(1) as f32) / (args.height.max(1) as f32);
        let model = Mat4::from_rotation_x(0.65) * Mat4::from_rotation_y(self.angle);
        let camera = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 4.6), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh_gl(45f32.to_radians(), aspect, 0.1, 100.0);
        let uniforms = Uniforms {
            mvp: (proj * camera * model).to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            cube_color: self.cube_color,
            clear_color: args.clear_color,
        };
        args.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flutter_wgpu_texture cube render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: args.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: args.clear_color[0] as f64,
                        g: args.clear_color[1] as f64,
                        b: args.clear_color[2] as f64,
                        a: args.clear_color[3] as f64,
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
        Ok(())
    }

    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (texture, view) = create_depth_texture(device, width.max(1), height.max(1));
        self.depth_texture = texture;
        self.depth_view = view;
    }

    fn set_float_param(&mut self, key: &str, value: f32) {
        match key {
            "rotation_speed" => self.rotation_speed = value,
            "angle" => self.angle = value,
            _ => {}
        }
    }

    fn set_bool_param(&mut self, key: &str, value: bool) {
        if key == "rotation_enabled" {
            self.rotation_enabled = value;
        }
    }

    fn set_vec4_param(&mut self, key: &str, value: [f32; 4]) {
        if key == "cube_color" {
            self.cube_color = value;
        }
    }

    fn invoke_command(&mut self, command: &str, _payload: &str) -> Result<(), String> {
        if command == "reset_scene" {
            self.rotation_enabled = true;
            self.rotation_speed = 1.2;
            self.angle = 0.0;
            self.cube_color = [1.0, 0.9, 0.1, 1.0];
        }
        Ok(())
    }
}
