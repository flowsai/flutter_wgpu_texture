use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::scene::{Scene, SceneRenderArgs};

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
struct ParticleVertex {
    corner: [f32; 2],
    position: [f32; 2],
    velocity: [f32; 2],
    life: f32,
    seed: f32,
}

pub struct ParticlesScene {
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

impl ParticlesScene {
    pub fn new(device: &wgpu::Device) -> Result<Self, String> {
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
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32, 4 => Float32
                    ],
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

impl Scene for ParticlesScene {
    fn render(
        &mut self,
        args: &SceneRenderArgs,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), String> {
        if args.animation_running {
            self.time += args.dt;
        }

        let uniforms = ParticleUniforms {
            viewport: [
                args.width as f32,
                args.height as f32,
                self.time,
                self.point_size,
            ],
            dynamics: [self.motion_scale, 0.0, 0.0, 0.0],
            color1: self.color1,
            color2: self.color2,
        };
        args.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flutter_wgpu_texture particles render pass"),
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
            "point_size" => self.point_size = value,
            "motion_scale" => self.motion_scale = value,
            "time" => self.time = value,
            _ => {}
        }
    }

    fn set_vec4_param(&mut self, key: &str, value: [f32; 4]) {
        match key {
            "color1" => self.color1 = value,
            "color2" => self.color2 = value,
            _ => {}
        }
    }

    fn invoke_command(&mut self, command: &str, _payload: &str) -> Result<(), String> {
        if command == "reset_scene" {
            self.time = 0.0;
            self.point_size = 14.0;
            self.motion_scale = 1.0;
            self.color1 = [1.0, 0.4, 0.1, 1.0];
            self.color2 = [0.1, 0.6, 1.0, 1.0];
        }
        Ok(())
    }
}
