use bytemuck::{Pod, Zeroable};
use flutter_wgpu_texture_core::{register_scene, Scene, SceneRenderArgs};
use wgpu::util::DeviceExt;

// ── shader ────────────────────────────────────────────────────────────────────

const SHADER: &str = r#"
struct Uniforms {
    time:    f32,
    _pad0:   f32,
    _pad1:   f32,
    _pad2:   f32,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct V2F {
    @builtin(position) pos: vec4<f32>,
    @location(0)       uv:  vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> V2F {
    var pos = array<vec2<f32>, 4>(
        vec2<f32>(-1., -1.), vec2<f32>(1., -1.),
        vec2<f32>(-1.,  1.), vec2<f32>(1.,  1.),
    );
    var uv = array<vec2<f32>, 4>(
        vec2<f32>(0., 1.), vec2<f32>(1., 1.),
        vec2<f32>(0., 0.), vec2<f32>(1., 0.),
    );
    return V2F(vec4<f32>(pos[i], 0., 1.), uv[i]);
}

@fragment
fn fs(in: V2F) -> @location(0) vec4<f32> {
    let wave = sin(in.uv.y * 3.14159 + u.time * 0.8) * 0.15;
    let t    = clamp(in.uv.x + wave, 0., 1.);
    return mix(u.color_a, u.color_b, t);
}
"#;

// ── uniforms (must be 16-byte aligned) ───────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    time: f32,
    _pad: [f32; 3],
    color_a: [f32; 4],
    color_b: [f32; 4],
}

// ── scene ─────────────────────────────────────────────────────────────────────

pub struct GradientScene {
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    time: f32,
    color_a: [f32; 4],
    color_b: [f32; 4],
}

impl GradientScene {
    pub fn new(device: &wgpu::Device) -> Result<Self, String> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gradient"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let color_a = [0.08, 0.26, 0.78, 1.0]; // deep blue
        let color_b = [0.85, 0.18, 0.52, 1.0]; // vivid pink

        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gradient_uniforms"),
            contents: bytemuck::bytes_of(&Uniforms {
                time: 0.0,
                _pad: [0.0; 3],
                color_a,
                color_b,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gradient_bgl"),
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
            label: Some("gradient_bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gradient_pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[&bgl],
                    push_constant_ranges: &[],
                }),
            ),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Ok(Self {
            pipeline,
            uniform_buf,
            bind_group,
            time: 0.0,
            color_a,
            color_b,
        })
    }
}

impl Scene for GradientScene {
    fn render(
        &mut self,
        args: &SceneRenderArgs,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), String> {
        if args.animation_running {
            self.time += args.dt;
        }

        args.queue.write_buffer(
            &self.uniform_buf,
            0,
            bytemuck::bytes_of(&Uniforms {
                time: self.time,
                _pad: [0.0; 3],
                color_a: self.color_a,
                color_b: self.color_b,
            }),
        );

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gradient_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: args.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..4, 0..1);
        Ok(())
    }

    fn set_vec4_param(&mut self, key: &str, value: [f32; 4]) {
        match key {
            "color_a" => self.color_a = value,
            "color_b" => self.color_b = value,
            _ => {}
        }
    }
}

// ── registration ──────────────────────────────────────────────────────────────

/// Runs automatically when the dylib is loaded — before any Dart call arrives.
#[ctor::ctor]
fn _register_gradient() {
    register_scene("gradient", |device, _w, _h| {
        GradientScene::new(device)
            .map(|s| Box::new(s) as Box<dyn flutter_wgpu_texture_core::Scene>)
    });
}
