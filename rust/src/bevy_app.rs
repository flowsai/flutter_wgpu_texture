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
use bevy::camera::primitives::Aabb;
use bevy::camera::{Camera, RenderTarget};
use bevy::ecs::resource::Resource;
use bevy::ecs::system::{In, RunSystemOnce};
use bevy::image::Image;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings, RayCastVisibility};
use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice, RenderQueue};
use bevy::window::ExitCondition;

use crate::level::{self, EditorIdMap};
use crate::viewport;

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

/// Which gizmo handle the cursor is hovering (for Unity-style highlight).
#[derive(Resource, Default)]
pub(crate) struct GizmoHover {
    pub(crate) axis: Option<GizmoAxis>,
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
            RenderCmd::SetHover { image, x, y } => {
                set_hover(&mut sub_apps, image, Vec2::new(x, y));
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
    app.world_mut().init_resource::<viewport::camera::OrbitCamera>();
    app.world_mut().init_resource::<GizmoHover>();
    app.world_mut().init_resource::<DragState>();

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

/// Draw the selection outline + transform gizmo (immediate mode, into the
/// offscreen camera). Runs in `Update` every frame.
fn draw_editor_gizmos(
    selection: Res<EditorSelection>,
    hover: Option<Res<GizmoHover>>,
    drag: Option<Res<DragState>>,
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

    // Highlight the hovered handle (or the grabbed one while dragging).
    let active_drag = drag.as_ref().filter(|d| d.active).and_then(|d| d.axis);
    let highlight = active_drag.or_else(|| hover.as_ref().and_then(|h| h.axis));

    draw_mode_gizmos(&mut gizmos, xf, selection.mode, highlight);
}

/// Draw translate/rotate/scale handles at the selected entity. `highlight` is
/// the hovered/grabbed axis, drawn in yellow (Unity-style).
fn draw_mode_gizmos(
    gizmos: &mut Gizmos,
    xf: &GlobalTransform,
    mode: GizmoMode,
    highlight: Option<GizmoAxis>,
) {
    let p = xf.translation();
    let len = 1.0_f32;
    let yellow = Color::srgb(1.0, 0.95, 0.2);
    let base = |axis: GizmoAxis| -> Color {
        if Some(axis) == highlight {
            yellow
        } else {
            match axis {
                GizmoAxis::X => Color::srgb(1.0, 0.25, 0.25),
                GizmoAxis::Y => Color::srgb(0.25, 1.0, 0.25),
                GizmoAxis::Z => Color::srgb(0.3, 0.5, 1.0),
            }
        }
    };

    match mode {
        GizmoMode::None => {}
        GizmoMode::Translate => {
            for axis in [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
                gizmos.arrow(p, p + axis.dir() * len, base(axis));
            }
        }
        GizmoMode::Rotate => {
            for axis in [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
                let rot = Quat::from_rotation_arc(Vec3::Z, axis.dir());
                gizmos.circle(Isometry3d::new(p, rot), len, base(axis));
            }
        }
        GizmoMode::Scale => {
            for axis in [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
                let dir = axis.dir();
                gizmos.line(p, p + dir * len, base(axis));
                gizmos.cube(
                    Transform::from_translation(p + dir * len).with_scale(Vec3::splat(0.12)),
                    base(axis),
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

/// Signed distance along the axis line `origin + t*dir` of the point closest to
/// the cursor `ray` (the `tb` from a normalized ray-to-ray closest-point solve,
/// matching the `transform-gizmo` crate). `dir` must be unit length.
/// `None` if the ray is ~parallel to the axis.
fn ray_axis_param(ray: Ray3d, origin: Vec3, dir: Vec3) -> Option<f32> {
    let adir = *ray.direction; // unit
    let bdir = dir; // unit
    let b = adir.dot(bdir);
    let w = ray.origin - origin;
    let d = adir.dot(w);
    let e = bdir.dot(w);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    // tb: parameter along the axis (bdir) of its closest point to the ray.
    Some((e - b * d) / denom)
}

/// Which gizmo axis handle (if any) the cursor is over, for the given mode.
/// Translate/scale test the axis segment; rotate tests the projected ring.
fn gizmo_axis_at(
    cursor: Vec2,
    origin: Vec3,
    mode: GizmoMode,
    cam: &Camera,
    cam_xf: &GlobalTransform,
) -> Option<GizmoAxis> {
    if mode == GizmoMode::None {
        return None;
    }
    let to_screen = |w: Vec3| cam.world_to_viewport(cam_xf, w).ok();
    let mut best: Option<(GizmoAxis, f32)> = None;
    for axis in [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
        let dist = match mode {
            GizmoMode::Rotate => circle_screen_dist(cursor, origin, axis.dir(), GIZMO_LEN, &to_screen),
            _ => {
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
    best.map(|(axis, _)| axis)
}

fn drag_begin(sub_apps: &mut SubApps, image: AssetId<Image>, cursor: Vec2) -> bool {
    let world = sub_apps.main.world_mut();
    world.init_resource::<DragState>();
    world
        .run_system_once_with(drag_begin_system, (image, cursor))
        .unwrap_or(false)
}

fn set_hover(sub_apps: &mut SubApps, image: AssetId<Image>, cursor: Vec2) {
    let world = sub_apps.main.world_mut();
    world.init_resource::<GizmoHover>();
    let _ = world.run_system_once_with(hover_system, (image, cursor));
}

fn hover_system(
    input: In<(AssetId<Image>, Vec2)>,
    cameras: Query<(&Camera, &GlobalTransform, &RenderTarget)>,
    transforms: Query<&GlobalTransform>,
    selection: Res<EditorSelection>,
    drag: Res<DragState>,
    mut hover: ResMut<GizmoHover>,
) {
    // Don't change the highlight mid-drag (the grabbed axis stays highlighted).
    if drag.active {
        return;
    }
    let (image, cursor) = input.0;
    let axis = (|| {
        let entity = selection.selected?;
        let obj_xf = transforms.get(entity).ok()?;
        let (cam, cam_xf, _) = cameras
            .iter()
            .find(|(_, _, rt)| matches!(rt, RenderTarget::Image(h) if h.handle.id() == image))?;
        gizmo_axis_at(cursor, obj_xf.translation(), selection.mode, cam, cam_xf)
    })();
    hover.axis = axis;
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

    let Some(axis) = gizmo_axis_at(cursor, obj_xf.translation(), selection.mode, cam, cam_xf)
    else {
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

    // Signed world distance the object should move along its axis: intersect the
    // cursor ray with the plane through the object containing the axis and most
    // facing the camera, then take the component along the axis. This is
    // geometrically exact and sign-correct for every axis and camera angle.
    let along_axis = {
        let ray0 = cam.viewport_to_world(cam_xf, drag.start_cursor);
        let ray1 = cam.viewport_to_world(cam_xf, cursor);
        match (ray0, ray1) {
            (Ok(r0), Ok(r1)) => {
                let p0 = ray_axis_param(r0, start.translation, axis_dir);
                let p1 = ray_axis_param(r1, start.translation, axis_dir);
                match (p0, p1) {
                    (Some(a), Some(b)) => b - a,
                    _ => cursor_delta.dot(axis2d) / (axis_end2d - origin2d).length().max(1.0),
                }
            }
            _ => cursor_delta.dot(axis2d) / (axis_end2d - origin2d).length().max(1.0),
        }
    };

    match selection.mode {
        GizmoMode::Translate => {
            new_t.translation = start.translation + axis_dir * along_axis;
        }
        GizmoMode::Rotate => {
            // Angle of cursor around the projected gizmo center (screen space),
            // following the `transform-gizmo` crate's convention:
            //   angle = atan2(now) - atan2(start)
            //   flip when the camera views the axis from behind (forward·axis < 0)
            //   delta_quat = from_axis_angle(world_axis, angle); new = delta * start
            let a0 = drag.start_cursor - origin2d;
            let a1 = cursor - origin2d;
            if a0.length_squared() < 4.0 || a1.length_squared() < 4.0 {
                return None;
            }
            let mut angle = a1.y.atan2(a1.x) - a0.y.atan2(a0.x);
            if cam_xf.forward().dot(axis_dir) < 0.0 {
                angle = -angle;
            }
            new_t.rotation =
                (Quat::from_axis_angle(axis_dir, angle) * start.rotation).normalize();
        }
        GizmoMode::Scale => {
            // Same signed on-screen amount as translate → grow when dragging the
            // handle the way it points, shrink the other way (per-axis correct).
            let factor = (1.0 + along_axis).max(0.05);
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
