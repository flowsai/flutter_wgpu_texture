mod cube;
mod particles;
mod shader_playground;

pub use cube::CubeScene;
pub use particles::ParticlesScene;
pub use shader_playground::ShaderPlaygroundScene;

use crate::scene::Scene;

/// Instantiate a scene by name.
///
/// The registry (populated via `#[ctor]` registration) is checked first so
/// that user-provided scenes from a combined workspace take priority over the
/// built-in ones. Falls back to the three built-in scenes for backwards
/// compatibility with existing examples.
pub fn scene_for_type(
    name: &str,
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Result<Box<dyn Scene>, String> {
    if let Some(result) =
        flutter_wgpu_texture_core::scene_from_registry(name, device, width, height)
    {
        return result;
    }
    match name {
        "particles" => Ok(Box::new(ParticlesScene::new(device)?)),
        "shader_playground" => Ok(Box::new(ShaderPlaygroundScene::new(device)?)),
        _ => Ok(Box::new(CubeScene::new(device, width, height)?)),
    }
}
