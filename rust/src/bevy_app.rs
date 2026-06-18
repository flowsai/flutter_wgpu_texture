//! Global headless Bevy renderer running on a dedicated thread.
//!
//! Bevy's `App` is `!Send`, so it lives entirely on one render thread. The rest
//! of the crate (Dart-thread FFI calls) talks to it over an mpsc channel.
//!
//! Design (Option A1 from the plan):
//!  - ONE Bevy `App` builds ONE wgpu device (with the DMA-BUF Vulkan extensions
//!    enabled via the `raw_vulkan_init` callback). That device is published so
//!    the package's `engine::device_context()` borrows it.
//!  - Each viewport = one camera rendering to one offscreen `Image`. The render
//!    thread, after pumping a frame, copies the rendered `GpuImage` into the
//!    package's DMA-BUF-backed `shared_texture` on the same device.

use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use bevy::app::{App, PluginsState, SubApps};
use bevy::asset::AssetId;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice, RenderQueue};
use bevy::window::ExitCondition;

use crate::level::{self};
use crate::viewport;
use crate::{gizmo, picking};

/// Transform returned to Dart after a drag update so the inspector stays in sync.
pub(crate) struct TransformOut {
    pub(crate) translation: [f32; 3],
    pub(crate) rotation: [f32; 4],
    pub(crate) scale: [f32; 3],
}

impl TransformOut {
    pub(crate) fn from_transform(t: &Transform) -> Self {
        Self {
            translation: t.translation.to_array(),
            rotation: t.rotation.to_array(),
            scale: t.scale.to_array(),
        }
    }
}

// ── Shared device handle, published once the Bevy app has built it ────────────

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

// ── Commands sent to the render thread ───────────────────────────────────────

pub(crate) enum RenderCmd {
    /// Create a viewport camera + offscreen image; reply with the image AssetId.
    CreateViewport {
        width: u32,
        height: u32,
        reply: Sender<AssetId<Image>>,
    },
    /// Remove a viewport (despawn its camera; drop its image).
    DisposeViewport { image: AssetId<Image> },
    /// Resize a viewport's offscreen image.
    ResizeViewport {
        image: AssetId<Image>,
        width: u32,
        height: u32,
    },
    /// Replace the scene contents (JSON serialized editor scene tree).
    SetScene { json: String },
    /// Raycast from a viewport pixel; reply with the hit editor id (if any).
    Pick {
        image: AssetId<Image>,
        x: f32,
        y: f32,
        reply: Sender<Option<String>>,
    },
    /// Set the current selection by editor id (None clears).
    SelectEntity { id: Option<String> },
    /// Set the active transform gizmo mode ("translate"|"rotate"|"scale"|"none").
    SetGizmoMode { mode: String },
    /// Orbit the camera around its focus (Alt+LMB drag). dx/dy = pixel deltas.
    CameraOrbit { image: AssetId<Image>, dx: f32, dy: f32 },
    /// Pan the camera focus in the view plane (MMB drag).
    CameraPan { image: AssetId<Image>, dx: f32, dy: f32 },
    /// Zoom toward/away from focus (scroll). delta = scroll units.
    CameraZoom { image: AssetId<Image>, delta: f32 },
    /// Free-look: rotate in place (RMB drag), keeping focus ahead of the camera.
    CameraLook { image: AssetId<Image>, dx: f32, dy: f32 },
    /// Fly: move along camera basis (RMB + WASD). f/r/u in [-1,1], dt in seconds.
    CameraFly {
        image: AssetId<Image>,
        forward: f32,
        right: f32,
        up: f32,
        dt: f32,
    },
    /// Begin a gizmo drag at a viewport pixel; reply true if a handle was grabbed.
    DragBegin {
        image: AssetId<Image>,
        x: f32,
        y: f32,
        reply: Sender<bool>,
    },
    /// Continue a gizmo drag; reply with the selected entity's new transform.
    DragUpdate {
        image: AssetId<Image>,
        x: f32,
        y: f32,
        reply: Sender<Option<TransformOut>>,
    },
    /// End the current gizmo drag.
    DragEnd,
    /// Update which gizmo handle is hovered at a viewport pixel (highlight).
    SetHover { image: AssetId<Image>, x: f32, y: f32 },
    /// Render one frame for `image` and copy the result into `dst`.
    RenderFrame {
        image: AssetId<Image>,
        dst: Option<wgpu::Texture>,
        width: u32,
        height: u32,
        reply: Sender<Result<bool, String>>,
    },
    Shutdown,
}

// ── Public API used by engine.rs ─────────────────────────────────────────────

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

// ── Render thread ────────────────────────────────────────────────────────────

fn render_thread_main(ready_tx: Sender<Result<(), String>>) {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<RenderCmd>();

    let build = build_app();
    let (mut sub_apps, gpu) = match build {
        Ok(v) => v,
        Err(e) => {
            let _ = SHARED_GPU.set(Err(e.clone()));
            let _ = ready_tx.send(Err(e));
            return;
        }
    };

    // Publish the shared device + command sender. After this, engine.rs can run.
    let _ = SHARED_GPU.set(Ok(gpu));
    let _ = CMD_TX.set(cmd_tx);
    let _ = ready_tx.send(Ok(()));

    let mut viewports: Vec<viewport::Viewport> = Vec::new();

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            RenderCmd::CreateViewport {
                width,
                height,
                reply,
            } => {
                let (image, camera) = viewport::spawn_viewport(&mut sub_apps, width, height);
                viewports.push(viewport::Viewport {
                    image,
                    camera,
                    width,
                    height,
                });
                let _ = reply.send(image);
            }
            RenderCmd::DisposeViewport { image } => {
                if let Some(pos) = viewports.iter().position(|v| v.image == image) {
                    let v = viewports.remove(pos);
                    sub_apps.main.world_mut().despawn(v.camera);
                    sub_apps
                        .main
                        .world_mut()
                        .resource_mut::<Assets<Image>>()
                        .remove(v.image);
                }
            }
            RenderCmd::ResizeViewport {
                image,
                width,
                height,
            } => {
                if let Some(v) = viewports.iter_mut().find(|v| v.image == image) {
                    viewport::resize_viewport_image(&mut sub_apps, image, width, height);
                    v.width = width;
                    v.height = height;
                }
            }
            RenderCmd::SetScene { json } => {
                level::rebuild_scene(&mut sub_apps, &json);
            }
            RenderCmd::Pick { image, x, y, reply } => {
                let id = picking::pick_entity(&mut sub_apps, image, Vec2::new(x, y));
                let _ = reply.send(id);
            }
            RenderCmd::SelectEntity { id } => {
                picking::set_selection(&mut sub_apps, id);
            }
            RenderCmd::SetGizmoMode { mode } => {
                let world = sub_apps.main.world_mut();
                world.init_resource::<picking::EditorSelection>();
                world.resource_mut::<picking::EditorSelection>().mode = gizmo::GizmoMode::from_str(&mode);
            }
            RenderCmd::CameraOrbit { image, dx, dy } => {
                viewport::camera::camera_orbit(&mut sub_apps, image, dx, dy);
            }
            RenderCmd::CameraPan { image, dx, dy } => {
                viewport::camera::camera_pan(&mut sub_apps, image, dx, dy);
            }
            RenderCmd::CameraZoom { image, delta } => {
                viewport::camera::camera_zoom(&mut sub_apps, image, delta);
            }
            RenderCmd::CameraLook { image, dx, dy } => {
                viewport::camera::camera_look(&mut sub_apps, image, dx, dy);
            }
            RenderCmd::CameraFly {
                image,
                forward,
                right,
                up,
                dt,
            } => {
                viewport::camera::camera_fly(&mut sub_apps, image, forward, right, up, dt);
            }
            RenderCmd::DragBegin { image, x, y, reply } => {
                let grabbed = gizmo::drag::drag_begin(&mut sub_apps, image, Vec2::new(x, y));
                let _ = reply.send(grabbed);
            }
            RenderCmd::DragUpdate { image, x, y, reply } => {
                let out = gizmo::drag::drag_update(&mut sub_apps, image, Vec2::new(x, y));
                let _ = reply.send(out);
            }
            RenderCmd::DragEnd => {
                let world = sub_apps.main.world_mut();
                world.init_resource::<gizmo::DragState>();
                world.resource_mut::<gizmo::DragState>().active = false;
            }
            RenderCmd::SetHover { image, x, y } => {
                gizmo::drag::set_hover(&mut sub_apps, image, Vec2::new(x, y));
            }
            RenderCmd::RenderFrame {
                image,
                dst,
                width,
                height,
                reply,
            } => {
                let result = viewport::render_one_frame(&mut sub_apps, image, dst.as_ref(), width, height);
                let _ = reply.send(result);
            }
            RenderCmd::Shutdown => break,
        }
    }
}

fn build_app() -> Result<(SubApps, SharedGpu), String> {
    use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
    use bevy::render::RenderPlugin;
    use bevy::window::WindowPlugin;

    let mut app = App::new();

    let mut wgpu_settings = WgpuSettings::default();
    wgpu_settings.backends = Some(Backends::VULKAN);

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

    // Gizmos for selection outline + transform handles. GizmoPlugin is already
    // in DefaultPlugins (via the `bevy_gizmos` feature); just add our draw system.
    app.init_resource::<picking::EditorSelection>();
    app.add_systems(Update, gizmo::draw::draw_editor_gizmos);

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

    // Make gizmos draw on top of geometry (selection outline always visible).
    {
        use bevy::gizmos::config::{DefaultGizmoConfigGroup, GizmoConfigStore};
        let mut store = app.world_mut().resource_mut::<GizmoConfigStore>();
        let (config, _) = store.config_mut::<DefaultGizmoConfigGroup>();
        config.depth_bias = -1.0;
        config.line.width = 2.5;
    }

    // Orbit camera state for the viewport (one viewport in v1).
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
