//! Shared wgpu device + Bevy app construction, and the render-thread lifecycle
//! entry points (`ensure_started`, `send`). The device is built once with the
//! DMA-BUF Vulkan extensions enabled (via `raw_vulkan_init`) and published so the
//! rest of the crate can borrow it.

use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use bevy::app::{App, PluginsState, SubApps};
use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice, RenderQueue};
use bevy::window::ExitCondition;

use super::render_thread::{render_thread_main, RenderCmd};
use crate::{gizmo, picking, viewport};

/// Shared GPU handle, published once the Bevy app has built it.
pub(crate) struct SharedGpu {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub backend_name: String,
    pub device_name: String,
    pub driver: String,
}

static SHARED_GPU: OnceLock<Result<SharedGpu, String>> = OnceLock::new();
static CMD_TX: OnceLock<Sender<RenderCmd>> = OnceLock::new();

/// Borrow the shared GPU handle once the Bevy app has built it. Lets other
/// modules (e.g. `viewport`) reach the device without knowing the OnceLock.
pub(crate) fn shared_gpu() -> Option<&'static SharedGpu> {
    SHARED_GPU.get().and_then(|r| r.as_ref().ok())
}

/// Publish the built GPU handle (called by the render thread on startup).
pub(super) fn publish_gpu(result: Result<SharedGpu, String>) {
    let _ = SHARED_GPU.set(result);
}

/// Publish the command sender (called by the render thread on startup).
pub(super) fn publish_cmd_tx(tx: Sender<RenderCmd>) {
    let _ = CMD_TX.set(tx);
}

/// Ensure the global Bevy render thread is running and the shared device built.
/// Returns the shared GPU handle (borrowed device/queue) or an init error.
pub(crate) fn ensure_started() -> Result<&'static SharedGpu, String> {
    // Start the thread once. The thread builds the app, publishes SHARED_GPU and
    // CMD_TX, then loops on the command channel.
    if CMD_TX.get().is_none() {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        std::thread::Builder::new()
            .name("bevy_render".into())
            .spawn(move || render_thread_main(ready_tx))
            .map_err(|e| format!("failed to spawn bevy render thread: {e}"))?;
        // Block until the thread has either published the device or failed.
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(format!("bevy render thread died during init: {e}")),
        }
    }
    SHARED_GPU
        .get()
        .ok_or_else(|| "shared gpu not initialized".to_string())?
        .as_ref()
        .map_err(Clone::clone)
}

pub(crate) fn send(cmd: RenderCmd) -> Result<(), String> {
    CMD_TX
        .get()
        .ok_or_else(|| "bevy render thread not started".to_string())?
        .send(cmd)
        .map_err(|_| "bevy render thread channel closed".to_string())
}

/// Build the headless Bevy app + shared wgpu device (with DMA-BUF extensions).
pub(super) fn build_app() -> Result<(SubApps, SharedGpu), String> {
    use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
    use bevy::render::RenderPlugin;
    use bevy::window::WindowPlugin;

    let mut app = App::new();

    let mut wgpu_settings = WgpuSettings::default();
    // Backend per-OS: the present/share path differs (DMA-BUF on Linux, IOSurface/
    // Metal on macOS, DXGI on Windows), so the wgpu device must match.
    #[cfg(target_os = "linux")]
    {
        wgpu_settings.backends = Some(Backends::VULKAN);
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        wgpu_settings.backends = Some(Backends::METAL);
    }
    #[cfg(target_os = "windows")]
    {
        wgpu_settings.backends = Some(Backends::DX12);
    }

    // Install the DMA-BUF device-extension callback before RenderPlugin builds.
    #[cfg(target_os = "linux")]
    {
        use bevy::render::renderer::raw_vulkan_init::RawVulkanInitSettings;
        use std::ffi::CStr;
        let mut s = RawVulkanInitSettings::default();
        // SAFETY: we only ADD extensions; never remove features. wgpu-hal verifies
        // availability before enabling.
        unsafe {
            s.add_create_device_callback(|args, _adapter, _features| {
                const WANTED: [&CStr; 3] = [
                    ash::khr::external_memory_fd::NAME,
                    ash::ext::external_memory_dma_buf::NAME,
                    ash::ext::image_drm_format_modifier::NAME,
                ];
                for ext in WANTED {
                    if !args.extensions.contains(&ext) {
                        args.extensions.push(ext);
                    }
                }
            });
        }
        app.insert_resource(s);
    }

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(wgpu_settings)),
                synchronous_pipeline_compilation: true,
                ..default()
            }),
    );

    // Physics. The simulation clock starts paused so bodies stay inert while
    // editing; entering play unpauses it.
    app.add_plugins(avian3d::PhysicsPlugins::default());

    // Wireframe overlay (toggled by set_view_mode("Wireframe")).
    app.add_plugins(bevy::pbr::wireframe::WireframePlugin::default());

    // Gizmos for selection outline + transform handles. GizmoPlugin is already
    // in DefaultPlugins (via the `bevy_gizmos` feature); just add our draw system.
    app.init_resource::<picking::EditorSelection>();
    app.add_systems(Update, gizmo::draw::draw_editor_gizmos);

    // Register every type that may live on a scene entity so the play snapshot
    // captures it. Several of these (Transform, Name, ChildOf, the light types)
    // are not registered by their Bevy plugins.
    {
        let registry = app.world().resource::<AppTypeRegistry>().clone();
        crate::level::register_scene_types(&mut registry.write());
    }
    app.add_systems(
        Update,
        (
            crate::level::primitives::materialize_meshes,
            crate::level::primitives::sync_material_colors,
        )
            .chain(),
    );

    // Ambient light so faces not facing the directional light aren't pure black.
    app.insert_resource(bevy::light::GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 200.0,
        ..default()
    });

    // Drive plugin build to completion (async device creation finishes here).
    while app.plugins_state() == PluginsState::Adding {
        bevy::tasks::tick_global_task_pools_on_main_thread();
    }
    app.finish();
    app.cleanup();

    // Start in editing mode: physics is set up but does not step until play.
    crate::level::physics::pause_simulation(app.world_mut());

    // Make gizmos draw on top of geometry (selection outline always visible).
    {
        use bevy::gizmos::config::{DefaultGizmoConfigGroup, GizmoConfigStore};
        let mut store = app.world_mut().resource_mut::<GizmoConfigStore>();
        let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
        config.depth_bias = -1.0;
        config.line.width = 2.5;
    }

    // Orbit camera state for the viewport.
    app.world_mut().init_resource::<viewport::camera::OrbitCamera>();
    app.world_mut().init_resource::<gizmo::GizmoHover>();
    app.world_mut().init_resource::<gizmo::DragState>();

    // Extract the shared device/queue/adapter info from the built app.
    let world = app.world();
    let device = world.resource::<RenderDevice>().wgpu_device().clone();
    let queue = world.resource::<RenderQueue>().0.clone();
    let info = world.resource::<RenderAdapterInfo>();
    let gpu = SharedGpu {
        device: Arc::new(device),
        // RenderQueue holds Arc<WgpuWrapper<Queue>>; deref to wgpu::Queue and re-Arc.
        queue: Arc::new((**queue).clone()),
        backend_name: format!("{:?}", info.0.backend),
        device_name: info.0.name.clone(),
        driver: info.0.driver.clone(),
    };

    let sub_apps = std::mem::take(app.sub_apps_mut());
    Ok((sub_apps, gpu))
}
