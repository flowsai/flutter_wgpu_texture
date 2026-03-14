mod cube;
mod particles;
mod shader_playground;

pub use cube::CubeScene;
pub use particles::ParticlesScene;
pub use shader_playground::ShaderPlaygroundScene;

use crate::scene::Scene;

/// Instantiate a built-in scene by name.
///
/// This is only used by the examples bundled with this repository. When
/// building your own renderer, implement [`Scene`] directly and pass your
/// type to the engine instead of using this factory.
pub fn scene_for_type(
    name: &str,
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Result<Box<dyn Scene>, String> {
    match name {
        "particles" => Ok(Box::new(ParticlesScene::new(device)?)),
        "shader_playground" => Ok(Box::new(ShaderPlaygroundScene::new(device)?)),
        _ => Ok(Box::new(CubeScene::new(device, width, height)?)),
    }
}
