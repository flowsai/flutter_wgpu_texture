use flutter_rust_bridge::frb;

use crate::engine;

#[derive(Clone, Debug)]
pub struct BackendInfo {
    pub backend: String,
    pub device_name: String,
    pub driver: String,
}

#[derive(Clone, Debug)]
pub struct RendererInfo {
    pub handle: u64,
    pub backend: BackendInfo,
}

#[frb(sync)]
pub fn create_renderer(
    width: u32,
    height: u32,
    scene_type: String,
) -> Result<RendererInfo, String> {
    let scene = match scene_type.as_str() {
        "particles" => engine::SceneType::Particles,
        "shader_playground" => engine::SceneType::ShaderPlayground,
        _ => engine::SceneType::Cube,
    };
    let handle = engine::engine_create(width, height, scene)?;
    let backend = engine::renderer_backend_info(handle)?;
    Ok(RendererInfo { handle, backend })
}

#[frb(sync)]
pub fn dispose_renderer(handle: u64) {
    engine::engine_dispose(handle);
}

pub fn request_frame(handle: u64) -> Result<bool, String> {
    engine::render_frame(handle)
}

#[frb(sync)]
pub fn start_animation(handle: u64) -> Result<(), String> {
    engine::set_animation_running(handle, true)
}

#[frb(sync)]
pub fn stop_animation(handle: u64) -> Result<(), String> {
    engine::set_animation_running(handle, false)
}

#[frb(sync)]
pub fn set_bool_param(handle: u64, key: String, value: bool) -> Result<(), String> {
    engine::set_bool_param(handle, &key, value)
}

#[frb(sync)]
pub fn set_float_param(handle: u64, key: String, value: f32) -> Result<(), String> {
    engine::set_float_param(handle, &key, value)
}

#[frb(sync)]
pub fn set_vec4_param(handle: u64, key: String, value: Vec<f32>) -> Result<(), String> {
    let array: [f32; 4] = value
        .try_into()
        .map_err(|_| "set_vec4_param expects exactly 4 floats".to_string())?;
    engine::set_vec4_param(handle, &key, array)
}

#[frb(sync)]
pub fn invoke_command(handle: u64, command: String, payload: String) -> Result<(), String> {
    engine::invoke_command(handle, &command, &payload)
}

#[frb(sync)]
pub fn get_backend_info(handle: u64) -> Result<BackendInfo, String> {
    engine::renderer_backend_info(handle)
}
