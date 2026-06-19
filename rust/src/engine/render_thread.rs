//! The render-thread command protocol (`RenderCmd`) and the loop that owns the
//! Bevy `App` (`!Send`) and dispatches commands to the engine sub-modules.

use std::sync::mpsc::Sender;

use bevy::asset::AssetId;
use bevy::image::Image;
use bevy::prelude::*;

use super::device::{self, build_app, SharedGpu};
use crate::{gizmo, level, picking, registry, viewport};

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
    /// Serialize the editor scene to a `.scn.ron` file at `path`.
    SaveScene { path: String, reply: Sender<Result<(), String>> },
    /// Load a `.scn`/`.scn.ron` file into the live world.
    LoadScene { path: String, reply: Sender<Result<(), String>> },
    /// Reply with JSON listing every registered component type (registry::list_component_types).
    ListComponentTypes { reply: Sender<String> },
    /// Reply with JSON describing one component type (registry::describe_component), or None if unknown.
    DescribeComponent { type_path: String, reply: Sender<Option<String>> },
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
    /// Switch the editor play mode ("play" enters play, "edit" returns to editing).
    SetPlayMode { mode: String },
    /// Switch the viewport view mode ("Lit" | "Unlit" | "Wireframe").
    SetViewMode { mode: String },
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

/// The render thread: owns the Bevy `App`, builds the device, then dispatches
/// `RenderCmd`s to the engine sub-modules until shutdown.
pub(super) fn render_thread_main(ready_tx: Sender<Result<(), String>>) {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<RenderCmd>();

    let (mut sub_apps, gpu) = match build_app() {
        Ok(v) => v,
        Err(e) => {
            device::publish_gpu(Err(e.clone()));
            let _ = ready_tx.send(Err(e));
            return;
        }
    };

    // Publish the shared device + command sender. After this, engine.rs can run.
    device::publish_gpu(Ok(gpu));
    device::publish_cmd_tx(cmd_tx);
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
            RenderCmd::SaveScene { path, reply } => {
                let result = level::scene_file::save_scene(sub_apps.main.world_mut(), &path);
                let _ = reply.send(result);
            }
            RenderCmd::LoadScene { path, reply } => {
                let result = level::scene_file::load_scene(sub_apps.main.world_mut(), &path);
                let _ = reply.send(result);
            }
            RenderCmd::ListComponentTypes { reply } => {
                let world = sub_apps.main.world();
                let type_registry = world.resource::<AppTypeRegistry>().read();
                let _ = reply.send(registry::list_component_types(&type_registry));
            }
            RenderCmd::DescribeComponent { type_path, reply } => {
                let world = sub_apps.main.world();
                let type_registry = world.resource::<AppTypeRegistry>().read();
                let _ = reply.send(registry::describe_component(&type_registry, &type_path));
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
                world.resource_mut::<picking::EditorSelection>().mode =
                    gizmo::GizmoMode::from_str(&mode);
            }
            RenderCmd::SetPlayMode { mode } => {
                level::play::set_play_mode(sub_apps.main.world_mut(), &mode);
            }
            RenderCmd::SetViewMode { mode } => {
                level::view_mode::set_view_mode(sub_apps.main.world_mut(), &mode);
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
                let result =
                    viewport::render_one_frame(&mut sub_apps, image, dst.as_ref(), width, height);
                let _ = reply.send(result);
            }
            RenderCmd::Shutdown => break,
        }
    }
}
