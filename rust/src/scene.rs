/// Arguments passed to [`Scene::render`] on every frame.
pub struct SceneRenderArgs<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub view: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub dt: f32,
    pub animation_running: bool,
    pub clear_color: [f32; 4],
}

/// Trait that all renderable scenes must implement.
///
/// The plugin plumbing calls [`Scene::render`] every frame after managing the
/// present target. Everything else — shaders, pipelines, buffers, uniforms —
/// stays inside the scene implementation.
pub trait Scene: Send {
    /// Render a single frame into `args.view`.
    fn render(
        &mut self,
        args: &SceneRenderArgs,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), String>;

    /// Called when the surface is resized. Re-create any size-dependent GPU
    /// resources (e.g. depth textures) here.
    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let _ = (device, width, height);
    }

    fn set_float_param(&mut self, _key: &str, _value: f32) {}
    fn set_bool_param(&mut self, _key: &str, _value: bool) {}
    fn set_vec4_param(&mut self, _key: &str, _value: [f32; 4]) {}
    fn invoke_command(&mut self, _command: &str, _payload: &str) -> Result<(), String> {
        Ok(())
    }
}
