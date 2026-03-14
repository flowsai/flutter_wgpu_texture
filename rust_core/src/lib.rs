pub mod registry;
pub mod scene;

pub use registry::{register_scene, scene_from_registry};
pub use scene::{Scene, SceneRenderArgs};
