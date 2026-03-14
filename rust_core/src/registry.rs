use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use wgpu::Device;

use crate::scene::Scene;

type SceneFactory =
    Box<dyn Fn(&Device, u32, u32) -> Result<Box<dyn Scene>, String> + Send + Sync>;

static REGISTRY: OnceLock<Mutex<HashMap<String, SceneFactory>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, SceneFactory>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register a named scene factory.
///
/// Call this from a `#[ctor::ctor]` function so it runs automatically when
/// the dylib is loaded — before any Dart call reaches `create_renderer`.
///
/// ```rust,no_run
/// use flutter_wgpu_texture_core::register_scene;
///
/// #[ctor::ctor]
/// fn _register() {
///     register_scene("my_scene", |device, w, h| {
///         // construct and return your scene
///         Ok(Box::new(todo!()))
///     });
/// }
/// ```
pub fn register_scene(
    name: impl Into<String>,
    factory: impl Fn(&Device, u32, u32) -> Result<Box<dyn Scene>, String> + Send + Sync + 'static,
) {
    registry()
        .lock()
        .expect("scene registry poisoned")
        .insert(name.into(), Box::new(factory));
}

/// Instantiate a scene by name from the registry.
///
/// Returns `None` if the name has not been registered.
pub fn scene_from_registry(
    name: &str,
    device: &Device,
    width: u32,
    height: u32,
) -> Option<Result<Box<dyn Scene>, String>> {
    registry()
        .lock()
        .expect("scene registry poisoned")
        .get(name)
        .map(|f| f(device, width, height))
}
