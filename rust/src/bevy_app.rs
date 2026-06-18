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

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use bevy::app::{App, AppLabel, PluginsState, SubApps};
use bevy::asset::{AssetId, RenderAssetUsages};
use bevy::camera::primitives::Aabb;
use bevy::camera::{Camera, RenderTarget};
use bevy::ecs::resource::Resource;
use bevy::ecs::system::{In, RunSystemOnce};
use bevy::image::Image;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings, RayCastVisibility};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{PollType, TextureFormat};
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::RenderApp;
use bevy::window::ExitCondition;

use crate::scene::{SceneDoc, SceneEntityDef};

// ── Editor ↔ ECS mapping resources ────────────────────────────────────────────

/// Maps the editor's stable string ids to live Bevy entities (both directions).
#[derive(Resource, Default)]
pub(crate) struct EditorIdMap {
    pub(crate) fwd: HashMap<String, Entity>,
    pub(crate) rev: HashMap<Entity, String>,
}

/// Set once the first `set_scene` has been applied; gates the fallback default scene.
#[derive(Resource)]
struct SceneInitialized;

/// Transform gizmo interaction mode (mirrors the Dart `GizmoMode`).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GizmoMode {
    #[default]
    None,
    Translate,
    Rotate,
    Scale,
}

impl GizmoMode {
    fn from_str(s: &str) -> Self {
        match s {
            "translate" => GizmoMode::Translate,
            "rotate" => GizmoMode::Rotate,
            "scale" => GizmoMode::Scale,
            _ => GizmoMode::None,
        }
    }
}

/// Current editor selection + active gizmo mode (drives selection outline + handles).
#[derive(Resource, Default)]
pub(crate) struct EditorSelection {
    pub(crate) selected: Option<Entity>,
    pub(crate) mode: GizmoMode,
}

/// Which gizmo axis is grabbed during a drag.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GizmoAxis {
    X,
    Y,
    Z,
}

impl GizmoAxis {
    fn dir(self) -> Vec3 {
        match self {
            GizmoAxis::X => Vec3::X,
            GizmoAxis::Y => Vec3::Y,
            GizmoAxis::Z => Vec3::Z,
        }
    }
}

/// In-progress gizmo drag state (set on drag_begin, cleared on drag_end).
#[derive(Resource, Default)]
pub(crate) struct DragState {
    pub(crate) active: bool,
    pub(crate) axis: Option<GizmoAxis>,
    pub(crate) start_cursor: Vec2,
    pub(crate) start_transform: Transform,
}

/// Transform returned to Dart after a drag update so the inspector stays in sync.
pub(crate) struct TransformOut {
    pub(crate) translation: [f32; 3],
    pub(crate) rotation: [f32; 4],
    pub(crate) scale: [f32; 3],
}

impl TransformOut {
    fn from_transform(t: &Transform) -> Self {
        Self {
            translation: t.translation.to_array(),
            rotation: t.rotation.to_array(),
            scale: t.scale.to_array(),
        }
    }
}

/// Orbit/pan/zoom camera state for the viewport (Unity-style navigation).
/// `yaw`/`pitch` are spherical angles around `focus` at `distance`.
#[derive(Resource)]
pub(crate) struct OrbitCamera {
    pub(crate) focus: Vec3,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) distance: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        // Frame the default scene from roughly (3,3,6) looking at origin.
        Self {
            focus: Vec3::ZERO,
            yaw: 0.46,    // ~26° around Y
            pitch: 0.42,  // ~24° above horizon
            distance: 7.3,
        }
    }
}

impl OrbitCamera {
    /// Camera world position from the spherical orbit parameters.
    fn position(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.focus + Vec3::new(self.distance * cp * sy, self.distance * sp, self.distance * cp * cy)
    }

    fn transform(&self) -> Transform {
        Transform::from_translation(self.position()).looking_at(self.focus, Vec3::Y)
    }
}

const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.05;

/// Format of the offscreen render target. MUST match the wgpu format of the
/// package's DMA-BUF `shared_texture` (Bgra8Unorm) for copy_texture_to_texture.
const TARGET_FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;

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

/// Per-viewport bookkeeping kept on the render thread.
struct Viewport {
    image: AssetId<Image>,
    camera: Entity,
    width: u32,
    height: u32,
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

    let mut viewports: Vec<Viewport> = Vec::new();

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            RenderCmd::CreateViewport {
                width,
                height,
                reply,
            } => {
                let (image, camera) = spawn_viewport(&mut sub_apps, width, height);
                viewports.push(Viewport {
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
                    resize_viewport_image(&mut sub_apps, image, width, height);
                    v.width = width;
                    v.height = height;
                }
            }
            RenderCmd::SetScene { json } => {
                rebuild_scene(&mut sub_apps, &json);
            }
            RenderCmd::Pick { image, x, y, reply } => {
                let id = pick_entity(&mut sub_apps, image, Vec2::new(x, y));
                let _ = reply.send(id);
            }
            RenderCmd::SelectEntity { id } => {
                set_selection(&mut sub_apps, id);
            }
            RenderCmd::SetGizmoMode { mode } => {
                let world = sub_apps.main.world_mut();
                world.init_resource::<EditorSelection>();
                world.resource_mut::<EditorSelection>().mode = GizmoMode::from_str(&mode);
            }
            RenderCmd::CameraOrbit { image, dx, dy } => {
                camera_orbit(&mut sub_apps, image, dx, dy);
            }
            RenderCmd::CameraPan { image, dx, dy } => {
                camera_pan(&mut sub_apps, image, dx, dy);
            }
            RenderCmd::CameraZoom { image, delta } => {
                camera_zoom(&mut sub_apps, image, delta);
            }
            RenderCmd::CameraLook { image, dx, dy } => {
                camera_look(&mut sub_apps, image, dx, dy);
            }
            RenderCmd::CameraFly {
                image,
                forward,
                right,
                up,
                dt,
            } => {
                camera_fly(&mut sub_apps, image, forward, right, up, dt);
            }
            RenderCmd::DragBegin { image, x, y, reply } => {
                let grabbed = drag_begin(&mut sub_apps, image, Vec2::new(x, y));
                let _ = reply.send(grabbed);
            }
            RenderCmd::DragUpdate { image, x, y, reply } => {
                let out = drag_update(&mut sub_apps, image, Vec2::new(x, y));
                let _ = reply.send(out);
            }
            RenderCmd::DragEnd => {
                let world = sub_apps.main.world_mut();
                world.init_resource::<DragState>();
                world.resource_mut::<DragState>().active = false;
            }
            RenderCmd::RenderFrame {
                image,
                dst,
                width,
                height,
                reply,
            } => {
                let result = render_one_frame(&mut sub_apps, image, dst.as_ref(), width, height);
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
    app.init_resource::<EditorSelection>();
    app.add_systems(Update, draw_editor_gizmos);

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
    app.world_mut().init_resource::<OrbitCamera>();

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

// ── Scene + viewport management (render thread only) ──────────────────────────

fn make_target_image(width: u32, height: u32) -> Image {
    let mut img = Image::new_target_texture(width.max(1), height.max(1), TARGET_FORMAT, None);
    // new_target_texture omits COPY_SRC; required for the cross-copy into shared_texture.
    img.texture_descriptor.usage |= wgpu::TextureUsages::COPY_SRC;
    img.asset_usage = RenderAssetUsages::RENDER_WORLD;
    img
}

fn spawn_viewport(sub_apps: &mut SubApps, width: u32, height: u32) -> (AssetId<Image>, Entity) {
    let world = sub_apps.main.world_mut();
    let img = make_target_image(width, height);
    let handle = world.resource_mut::<Assets<Image>>().add(img);
    let image_id = handle.id();
    // In this Bevy, RenderTarget is its own component (not Camera.target).
    world.init_resource::<OrbitCamera>();
    let cam_xf = world.resource::<OrbitCamera>().transform();
    let camera = world
        .spawn((
            Camera3d::default(),
            RenderTarget::Image(handle.into()),
            cam_xf,
        ))
        .id();
    (image_id, camera)
}

fn resize_viewport_image(sub_apps: &mut SubApps, image: AssetId<Image>, width: u32, height: u32) {
    let world = sub_apps.main.world_mut();
    let new_image = make_target_image(width, height);
    let mut images = world.resource_mut::<Assets<Image>>();
    let _ = images.insert(image, new_image);
}

/// Apply the editor scene tree (JSON) by diffing against the live world.
/// Spawns new entities, patches changed transforms/materials, despawns removed.
/// The editor camera is viewport-owned and never touched here.
fn rebuild_scene(sub_apps: &mut SubApps, json: &str) {
    let doc: SceneDoc = match serde_json::from_str(json) {
        Ok(d) => d,
        Err(e) => {
            warn!("set_scene: failed to parse scene JSON: {e}");
            return;
        }
    };

    let world = sub_apps.main.world_mut();
    world.init_resource::<EditorIdMap>();
    world.insert_resource(SceneInitialized);

    let mut seen: HashSet<String> = HashSet::with_capacity(doc.entities.len());

    for def in &doc.entities {
        if def.kind == "camera" {
            continue; // camera is viewport-owned
        }
        seen.insert(def.id.clone());

        let existing = world
            .resource::<EditorIdMap>()
            .fwd
            .get(&def.id)
            .copied();

        match existing {
            Some(entity) => update_entity(world, entity, def),
            None => {
                let entity = spawn_entity(world, def);
                let mut map = world.resource_mut::<EditorIdMap>();
                map.fwd.insert(def.id.clone(), entity);
                map.rev.insert(entity, def.id.clone());
            }
        }
    }

    // Despawn entities no longer present in the doc.
    let to_remove: Vec<(String, Entity)> = world
        .resource::<EditorIdMap>()
        .fwd
        .iter()
        .filter(|(id, _)| !seen.contains(*id))
        .map(|(id, e)| (id.clone(), *e))
        .collect();
    for (id, entity) in to_remove {
        world.despawn(entity);
        let mut map = world.resource_mut::<EditorIdMap>();
        map.fwd.remove(&id);
        map.rev.remove(&entity);
    }
}

/// Spawn a brand-new entity from its editor definition. Returns the Bevy entity.
fn spawn_entity(world: &mut World, def: &SceneEntityDef) -> Entity {
    let transform = def.transform.to_bevy();

    match def.kind.as_str() {
        "mesh:cube" | "mesh:plane" => {
            let mesh = {
                let mut meshes = world.resource_mut::<Assets<Mesh>>();
                if def.kind == "mesh:plane" {
                    meshes.add(Plane3d::default().mesh().size(10.0, 10.0))
                } else {
                    meshes.add(Cuboid::default())
                }
            };
            let color = def
                .material
                .as_ref()
                .map(|m| m.color)
                .unwrap_or([0.8, 0.8, 0.8, 1.0]);
            let material = {
                let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
                materials.add(StandardMaterial {
                    base_color: Color::srgba(color[0], color[1], color[2], color[3]),
                    ..default()
                })
            };
            world
                .spawn((Mesh3d(mesh), MeshMaterial3d(material), transform))
                .id()
        }
        "light:directional" => {
            let illuminance = def.light.as_ref().map(|l| l.illuminance).unwrap_or(10_000.0);
            world
                .spawn((
                    DirectionalLight {
                        illuminance,
                        shadow_maps_enabled: true,
                        ..default()
                    },
                    light_transform(&transform),
                ))
                .id()
        }
        other => {
            warn!("set_scene: unknown entity kind '{other}', spawning empty");
            world.spawn(transform).id()
        }
    }
}

/// A `DirectionalLight` shines along its local -Z (rotation), not its position.
/// If the editor gives an (almost) identity rotation, aim it into the scene from
/// above so multiple faces are lit; otherwise honor the editor's rotation.
fn light_transform(transform: &Transform) -> Transform {
    const DEFAULT_DIR: Vec3 = Vec3::new(-1.0, -2.0, -1.0);
    if transform.rotation.abs_diff_eq(Quat::IDENTITY, 1e-4) {
        Transform::from_translation(transform.translation)
            .looking_to(DEFAULT_DIR.normalize(), Vec3::Y)
    } else {
        *transform
    }
}

/// Patch an existing entity's transform and (for meshes) material color.
fn update_entity(world: &mut World, entity: Entity, def: &SceneEntityDef) {
    let is_light = world.get::<DirectionalLight>(entity).is_some();
    if let Some(mut t) = world.get_mut::<Transform>(entity) {
        let new_t = def.transform.to_bevy();
        *t = if is_light {
            light_transform(&new_t)
        } else {
            new_t
        };
    }

    if let (Some(mat_def), Some(handle)) = (
        def.material.as_ref(),
        world.get::<MeshMaterial3d<StandardMaterial>>(entity).map(|m| m.0.clone()),
    ) {
        let c = mat_def.color;
        if let Some(mut material) = world
            .resource_mut::<Assets<StandardMaterial>>()
            .get_mut(&handle)
        {
            material.base_color = Color::srgba(c[0], c[1], c[2], c[3]);
        }
    }

    if let Some(light_def) = def.light.as_ref() {
        if let Some(mut light) = world.get_mut::<DirectionalLight>(entity) {
            light.illuminance = light_def.illuminance;
        }
    }
}

/// Fallback scene used only if the editor hasn't pushed a scene yet (avoids a
/// black viewport during startup). Once `set_scene` runs, `SceneInitialized`
/// exists and this becomes a no-op.
fn ensure_default_scene(sub_apps: &mut SubApps) {
    let world = sub_apps.main.world_mut();
    if world.get_resource::<SceneInitialized>().is_some() {
        return; // editor scene already drives the world
    }

    #[derive(Resource)]
    struct DefaultSceneSpawned;
    if world.get_resource::<DefaultSceneSpawned>().is_some() {
        return;
    }
    world.insert_resource(DefaultSceneSpawned);

    let mut meshes = world.resource_mut::<Assets<Mesh>>();
    let cube = meshes.add(Cuboid::default());
    let plane = meshes.add(Plane3d::default().mesh().size(10.0, 10.0));
    let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
    let cube_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.4, 0.6, 0.9),
        ..default()
    });
    let plane_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.3, 0.3, 0.3),
        ..default()
    });

    world.spawn((
        Mesh3d(cube),
        MeshMaterial3d(cube_mat),
        Transform::from_xyz(0.0, 0.5, 0.0),
    ));
    world.spawn((Mesh3d(plane), MeshMaterial3d(plane_mat), Transform::default()));
    world.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(3.0, 8.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

// ── Picking + selection ───────────────────────────────────────────────────────

/// Raycast from a viewport pixel into the scene, return the hit editor id.
fn pick_entity(sub_apps: &mut SubApps, image: AssetId<Image>, cursor: Vec2) -> Option<String> {
    let world = sub_apps.main.world_mut();
    world
        .run_system_once_with(pick_system, (image, cursor))
        .ok()
        .flatten()
}

fn pick_system(
    input: In<(AssetId<Image>, Vec2)>,
    cameras: Query<(&Camera, &GlobalTransform, &RenderTarget)>,
    mut ray_cast: MeshRayCast,
    id_map: Res<EditorIdMap>,
) -> Option<String> {
    let (image, cursor) = input.0;
    let (cam, cam_xf, _) = cameras
        .iter()
        .find(|(_, _, rt)| matches!(rt, RenderTarget::Image(h) if h.handle.id() == image))?;
    let ray = cam.viewport_to_world(cam_xf, cursor).ok()?;
    let settings = MeshRayCastSettings::default().with_visibility(RayCastVisibility::Any);
    let hits = ray_cast.cast_ray(ray, &settings);
    let (entity, _) = hits.first()?;
    id_map.rev.get(entity).cloned()
}

/// Set the selected entity from an editor id (None clears).
fn set_selection(sub_apps: &mut SubApps, id: Option<String>) {
    let world = sub_apps.main.world_mut();
    world.init_resource::<EditorSelection>();
    let entity = id
        .as_ref()
        .and_then(|id| world.resource::<EditorIdMap>().fwd.get(id).copied());
    world.resource_mut::<EditorSelection>().selected = entity;
}

// ── Camera navigation (Unity-style; driven by Flutter-fed deltas) ─────────────

/// Find the camera entity rendering to `image`.
fn camera_for_image(world: &mut World, image: AssetId<Image>) -> Option<Entity> {
    let mut q = world.query::<(Entity, &RenderTarget)>();
    q.iter(world)
        .find(|(_, rt)| matches!(rt, RenderTarget::Image(h) if h.handle.id() == image))
        .map(|(e, _)| e)
}

/// Write the orbit camera's transform onto the camera entity.
fn apply_orbit_camera(world: &mut World, camera: Entity) {
    let orbit = world.resource::<OrbitCamera>();
    let new_xf = orbit.transform();
    if let Some(mut t) = world.get_mut::<Transform>(camera) {
        *t = new_xf;
    }
}

fn camera_orbit(sub_apps: &mut SubApps, image: AssetId<Image>, dx: f32, dy: f32) {
    let world = sub_apps.main.world_mut();
    let Some(camera) = camera_for_image(world, image) else {
        return;
    };
    {
        let mut orbit = world.resource_mut::<OrbitCamera>();
        orbit.yaw -= dx * 0.008;
        orbit.pitch = (orbit.pitch + dy * 0.008).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }
    apply_orbit_camera(world, camera);
}

fn camera_pan(sub_apps: &mut SubApps, image: AssetId<Image>, dx: f32, dy: f32) {
    let world = sub_apps.main.world_mut();
    let Some(camera) = camera_for_image(world, image) else {
        return;
    };
    // Pan in the camera's right/up plane; speed scales with distance.
    let (right, up, dist) = {
        let xf = world.get::<Transform>(camera).copied().unwrap_or_default();
        let orbit = world.resource::<OrbitCamera>();
        (xf.right(), xf.up(), orbit.distance)
    };
    let k = dist * 0.0015;
    {
        let mut orbit = world.resource_mut::<OrbitCamera>();
        orbit.focus += (-right * dx + up * dy) * k;
    }
    apply_orbit_camera(world, camera);
}

fn camera_zoom(sub_apps: &mut SubApps, image: AssetId<Image>, delta: f32) {
    let world = sub_apps.main.world_mut();
    let Some(camera) = camera_for_image(world, image) else {
        return;
    };
    {
        let mut orbit = world.resource_mut::<OrbitCamera>();
        // Exponential zoom feels natural; scroll up (negative delta) = zoom in.
        orbit.distance = (orbit.distance * (delta * 0.001).exp()).clamp(0.5, 500.0);
    }
    apply_orbit_camera(world, camera);
}

fn camera_look(sub_apps: &mut SubApps, image: AssetId<Image>, dx: f32, dy: f32) {
    let world = sub_apps.main.world_mut();
    let Some(camera) = camera_for_image(world, image) else {
        return;
    };
    // Free-look rotates the orbit angles but keeps the focus ahead of the camera
    // so subsequent orbit/pan behave intuitively.
    {
        let mut orbit = world.resource_mut::<OrbitCamera>();
        orbit.yaw -= dx * 0.006;
        orbit.pitch = (orbit.pitch + dy * 0.006).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }
    apply_orbit_camera(world, camera);
}

fn camera_fly(
    sub_apps: &mut SubApps,
    image: AssetId<Image>,
    forward: f32,
    right: f32,
    up: f32,
    dt: f32,
) {
    let world = sub_apps.main.world_mut();
    let Some(camera) = camera_for_image(world, image) else {
        return;
    };
    let (fwd, rgt, dist) = {
        let xf = world.get::<Transform>(camera).copied().unwrap_or_default();
        let orbit = world.resource::<OrbitCamera>();
        (xf.forward(), xf.right(), orbit.distance)
    };
    // Move the focus (and thus the camera) along the camera basis. Speed scales
    // with distance so it feels consistent at any zoom level.
    let speed = (dist * 1.5).max(2.0);
    let motion = (fwd * forward + rgt * right + Vec3::Y * up) * speed * dt;
    {
        let mut orbit = world.resource_mut::<OrbitCamera>();
        orbit.focus += motion;
    }
    apply_orbit_camera(world, camera);
}

/// Draw the selection outline + transform gizmo (immediate mode, into the
/// offscreen camera). Runs in `Update` every frame.
fn draw_editor_gizmos(
    selection: Res<EditorSelection>,
    transforms: Query<&GlobalTransform>,
    aabbs: Query<&Aabb>,
    mut gizmos: Gizmos,
) {
    let Some(entity) = selection.selected else {
        return;
    };
    let Ok(xf) = transforms.get(entity) else {
        return;
    };

    // Selection outline: an orange box around the entity's AABB (or unit cube).
    let outline_color = Color::srgb(1.0, 0.6, 0.0);
    if let Ok(aabb) = aabbs.get(entity) {
        let center = xf.transform_point(Vec3::from(aabb.center));
        let half = Vec3::from(aabb.half_extents) * xf.scale();
        gizmos.cube(
            Transform::from_translation(center)
                .with_rotation(xf.rotation())
                .with_scale(half * 2.0),
            outline_color,
        );
    } else {
        gizmos.cube(Transform::from(*xf), outline_color);
    }

    // Transform handles per mode (drawn in stage 3; selection outline is enough here).
    draw_mode_gizmos(&mut gizmos, xf, selection.mode);
}

/// Draw translate/rotate/scale handles at the selected entity.
fn draw_mode_gizmos(gizmos: &mut Gizmos, xf: &GlobalTransform, mode: GizmoMode) {
    let p = xf.translation();
    let len = 1.0_f32;
    let red = Color::srgb(1.0, 0.25, 0.25);
    let green = Color::srgb(0.25, 1.0, 0.25);
    let blue = Color::srgb(0.3, 0.5, 1.0);

    match mode {
        GizmoMode::None => {}
        GizmoMode::Translate => {
            gizmos.arrow(p, p + Vec3::X * len, red);
            gizmos.arrow(p, p + Vec3::Y * len, green);
            gizmos.arrow(p, p + Vec3::Z * len, blue);
        }
        GizmoMode::Rotate => {
            // One ring per axis, in the plane perpendicular to that axis
            // (ring normal = axis), colors matching X=red, Y=green, Z=blue.
            for (axis, col) in [(Vec3::X, red), (Vec3::Y, green), (Vec3::Z, blue)] {
                let rot = Quat::from_rotation_arc(Vec3::Z, axis);
                gizmos.circle(Isometry3d::new(p, rot), len, col);
            }
        }
        GizmoMode::Scale => {
            for (dir, col) in [(Vec3::X, red), (Vec3::Y, green), (Vec3::Z, blue)] {
                gizmos.line(p, p + dir * len, col);
                gizmos.cube(
                    Transform::from_translation(p + dir * len).with_scale(Vec3::splat(0.12)),
                    col,
                );
            }
        }
    }
}

// ── Draggable transform gizmos ────────────────────────────────────────────────

const GIZMO_LEN: f32 = 1.0;
/// Pixel radius for grabbing a gizmo handle. Generous so handles are easy to hit.
const HANDLE_PIXEL_THRESHOLD: f32 = 18.0;

/// 2D distance from point `p` to segment `a`–`b`.
fn point_segment_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < 1e-6 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

fn drag_begin(sub_apps: &mut SubApps, image: AssetId<Image>, cursor: Vec2) -> bool {
    let world = sub_apps.main.world_mut();
    world.init_resource::<DragState>();
    world
        .run_system_once_with(drag_begin_system, (image, cursor))
        .unwrap_or(false)
}

fn drag_begin_system(
    input: In<(AssetId<Image>, Vec2)>,
    cameras: Query<(&Camera, &GlobalTransform, &RenderTarget)>,
    transforms: Query<&GlobalTransform>,
    selection: Res<EditorSelection>,
    mut drag: ResMut<DragState>,
) -> bool {
    let (image, cursor) = input.0;
    let Some(entity) = selection.selected else {
        return false;
    };
    if selection.mode == GizmoMode::None {
        return false;
    }
    let Ok(obj_xf) = transforms.get(entity) else {
        return false;
    };
    let Some((cam, cam_xf, _)) = cameras
        .iter()
        .find(|(_, _, rt)| matches!(rt, RenderTarget::Image(h) if h.handle.id() == image))
    else {
        return false;
    };

    let origin = obj_xf.translation();
    let to_screen = |w: Vec3| cam.world_to_viewport(cam_xf, w).ok();

    // Hit-test each axis handle in screen space; pick the nearest within threshold.
    let mut best: Option<(GizmoAxis, f32)> = None;
    for axis in [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
        let dist = match selection.mode {
            GizmoMode::Rotate => {
                // Distance to the projected ring (perpendicular to the axis).
                circle_screen_dist(cursor, origin, axis.dir(), GIZMO_LEN, &to_screen)
            }
            _ => {
                // Distance to the axis segment (translate / scale).
                let (Some(a), Some(b)) =
                    (to_screen(origin), to_screen(origin + axis.dir() * GIZMO_LEN))
                else {
                    continue;
                };
                point_segment_dist(cursor, a, b)
            }
        };
        if dist < HANDLE_PIXEL_THRESHOLD && best.map_or(true, |(_, bd)| dist < bd) {
            best = Some((axis, dist));
        }
    }

    let Some((axis, _)) = best else {
        return false;
    };

    drag.active = true;
    drag.axis = Some(axis);
    drag.start_cursor = cursor;
    drag.start_transform = obj_xf.compute_transform();
    true
}

/// Minimum screen-space distance from `cursor` to a world circle of `radius`
/// centered at `center` with normal `axis`, by sampling the circle.
fn circle_screen_dist(
    cursor: Vec2,
    center: Vec3,
    axis: Vec3,
    radius: f32,
    to_screen: &impl Fn(Vec3) -> Option<Vec2>,
) -> f32 {
    let rot = Quat::from_rotation_arc(Vec3::Z, axis.normalize());
    const SAMPLES: usize = 48;
    let mut prev: Option<Vec2> = None;
    let mut min = f32::INFINITY;
    let mut first: Option<Vec2> = None;
    for i in 0..SAMPLES {
        let t = i as f32 / SAMPLES as f32 * std::f32::consts::TAU;
        let local = Vec3::new(t.cos() * radius, t.sin() * radius, 0.0);
        let world = center + rot * local;
        let Some(p) = to_screen(world) else {
            prev = None;
            continue;
        };
        if first.is_none() {
            first = Some(p);
        }
        if let Some(a) = prev {
            min = min.min(point_segment_dist(cursor, a, p));
        }
        prev = Some(p);
    }
    // Close the loop.
    if let (Some(a), Some(b)) = (prev, first) {
        min = min.min(point_segment_dist(cursor, a, b));
    }
    min
}

fn drag_update(
    sub_apps: &mut SubApps,
    image: AssetId<Image>,
    cursor: Vec2,
) -> Option<TransformOut> {
    let world = sub_apps.main.world_mut();
    world
        .run_system_once_with(drag_update_system, (image, cursor))
        .ok()
        .flatten()
}

fn drag_update_system(
    input: In<(AssetId<Image>, Vec2)>,
    cameras: Query<(&Camera, &GlobalTransform, &RenderTarget)>,
    mut transforms: Query<&mut Transform>,
    selection: Res<EditorSelection>,
    drag: Res<DragState>,
) -> Option<TransformOut> {
    if !drag.active {
        return None;
    }
    let (image, cursor) = input.0;
    let entity = selection.selected?;
    let axis = drag.axis?;
    let (cam, cam_xf, _) = cameras
        .iter()
        .find(|(_, _, rt)| matches!(rt, RenderTarget::Image(h) if h.handle.id() == image))?;

    let axis_dir = axis.dir();
    let start = &drag.start_transform;
    let mut new_t = *start;

    // Screen-space direction of the +axis at the object origin (pixels).
    // Used to map intuitive cursor motion to signed amounts along the axis.
    let origin2d = cam.world_to_viewport(cam_xf, start.translation).ok()?;
    let axis_end2d = cam
        .world_to_viewport(cam_xf, start.translation + axis_dir * GIZMO_LEN)
        .ok()?;
    let axis2d = (axis_end2d - origin2d).normalize_or_zero();
    let cursor_delta = cursor - drag.start_cursor;

    match selection.mode {
        GizmoMode::Translate => {
            // Move along the axis by how far the cursor dragged in the handle's
            // ON-SCREEN direction, converted to world units via the projected
            // length of one world unit of the axis. This makes "drag the arrow
            // the way it points" always move along +axis (intuitive, no inversion).
            let axis_px_per_unit = (axis_end2d - origin2d).length().max(1.0);
            let drag_px = cursor_delta.dot(axis2d);
            let world_delta = drag_px / axis_px_per_unit * GIZMO_LEN;
            new_t.translation = start.translation + axis_dir * world_delta;
        }
        GizmoMode::Rotate => {
            // Signed angle swept by the cursor around the projected center.
            // Vec2::angle_to gives a CCW angle in screen space (Y-down), so a
            // positive screen angle is CW visually — negate for intuitive feel.
            let a0 = drag.start_cursor - origin2d;
            let a1 = cursor - origin2d;
            if a0.length_squared() < 4.0 || a1.length_squared() < 4.0 {
                return None;
            }
            let screen_angle = a0.angle_to(a1); // CCW-positive in math space
            // Axis facing the camera (dot with forward < 0) rotates the same
            // visual direction as the screen swing; facing away flips it.
            let facing = -axis_dir.dot(*cam_xf.forward());
            let angle = screen_angle * facing.signum();
            new_t.rotation = (start.rotation * Quat::from_axis_angle(axis_dir, angle)).normalize();
        }
        GizmoMode::Scale => {
            // Drag along the axis's screen direction = grow; opposite = shrink.
            let signed = cursor_delta.dot(axis2d);
            let factor = (1.0 + signed * 0.01).max(0.05);
            let mut scale = start.scale;
            match axis {
                GizmoAxis::X => scale.x *= factor,
                GizmoAxis::Y => scale.y *= factor,
                GizmoAxis::Z => scale.z *= factor,
            }
            new_t.scale = scale;
        }
        GizmoMode::None => return None,
    }

    let mut t = transforms.get_mut(entity).ok()?;
    *t = new_t;
    Some(TransformOut::from_transform(&new_t))
}

fn render_one_frame(
    sub_apps: &mut SubApps,
    image: AssetId<Image>,
    dst: Option<&wgpu::Texture>,
    width: u32,
    height: u32,
) -> Result<bool, String> {
    ensure_default_scene(sub_apps);

    // Pump one full frame (Extract -> render-world Render schedule -> submit).
    sub_apps.update();

    let gpu = SHARED_GPU
        .get()
        .and_then(|r| r.as_ref().ok())
        .ok_or_else(|| "shared gpu missing".to_string())?;

    // Wait for the GPU to finish this frame's submissions before copying.
    gpu.device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| format!("device.poll failed: {e:?}"))?;

    let Some(dst) = dst else {
        return Ok(true);
    };

    // Reach into the render sub-app world for the rendered GpuImage texture.
    let render_app = sub_apps
        .sub_apps
        .get_mut(&RenderApp.intern())
        .ok_or_else(|| "RenderApp sub-app missing".to_string())?;
    let gpu_images = render_app
        .world()
        .get_resource::<RenderAssets<GpuImage>>()
        .ok_or_else(|| "RenderAssets<GpuImage> missing".to_string())?;
    let Some(gpu_image) = gpu_images.get(image) else {
        // Image not prepared yet (first frame). Not an error; just no copy.
        return Ok(false);
    };

    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bevy->dmabuf cross copy"),
        });
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &gpu_image.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: dst,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    gpu.device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| format!("device.poll (copy) failed: {e:?}"))?;

    Ok(true)
}
