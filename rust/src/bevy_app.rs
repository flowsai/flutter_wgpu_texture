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
            gizmos.circle(
                Isometry3d::new(p, Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
                len,
                red,
            );
            gizmos.circle(Isometry3d::new(p, Quat::IDENTITY), len, green);
            gizmos.circle(
                Isometry3d::new(p, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                len,
                blue,
            );
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
