use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::scene::{Scene, SceneRenderArgs};

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
struct ShaderUniforms {
    viewport: [f32; 4],
    pointer: [f32; 4],
    tuning: [f32; 4],
    primary_color: [f32; 4],
    secondary_color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShaderVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

pub struct ShaderPlaygroundScene {
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

impl ShaderPlaygroundScene {
    pub fn new(device: &wgpu::Device) -> Result<Self, String> {
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
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

impl Scene for ShaderPlaygroundScene {
    fn render(
        &mut self,
        args: &SceneRenderArgs,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), String> {
        if args.animation_running {
            self.time += args.dt;
        }

        let uniforms = ShaderUniforms {
            viewport: [args.width as f32, args.height as f32, self.time, self.speed],
            pointer: [
                self.pointer[0],
                self.pointer[1],
                self.pointer[2],
                self.distortion,
            ],
            tuning: [self.noise_scale, 0.0, 0.0, 0.0],
            primary_color: self.primary_color,
            secondary_color: self.secondary_color,
        };
        args.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flutter_wgpu_texture shader playground render pass"),
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
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.vertex_count, 0..1);
        Ok(())
    }

    fn set_float_param(&mut self, key: &str, value: f32) {
        match key {
            "speed" => self.speed = value,
            "noise_scale" => self.noise_scale = value,
            "distortion" => self.distortion = value,
            "time" => self.time = value,
            _ => {}
        }
    }

    fn set_vec4_param(&mut self, key: &str, value: [f32; 4]) {
        match key {
            "primary_color" => self.primary_color = value,
            "secondary_color" => self.secondary_color = value,
            "pointer" => self.pointer = value,
            _ => {}
        }
    }

    fn invoke_command(&mut self, command: &str, _payload: &str) -> Result<(), String> {
        if command == "reset_scene" {
            self.time = 0.0;
            self.speed = 1.0;
            self.noise_scale = 2.4;
            self.distortion = 1.0;
            self.pointer = [0.5, 0.5, 0.0, 1.0];
            self.primary_color = [1.0, 0.46, 0.16, 1.0];
            self.secondary_color = [0.14, 0.77, 1.0, 1.0];
        }
        Ok(())
    }
}
