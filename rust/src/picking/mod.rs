//! Mesh picking + selection (≈ Flax editor selection via ray-cast). Headless:
//! the cursor pixel is fed from Flutter; we raycast the offscreen camera.

use bevy::app::SubApps;
use bevy::asset::AssetId;
use bevy::camera::{Camera, RenderTarget};
use bevy::ecs::resource::Resource;
use bevy::ecs::system::{In, RunSystemOnce};
use bevy::image::Image;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings, RayCastVisibility};
use bevy::prelude::*;

use crate::gizmo::GizmoMode;
use crate::level::EditorIdMap;

/// Current editor selection + active gizmo mode (drives selection outline + handles).
#[derive(Resource, Default)]
pub(crate) struct EditorSelection {
    pub(crate) selected: Option<Entity>,
    pub(crate) mode: GizmoMode,
}

/// Raycast from a viewport pixel into the scene, return the hit editor id.
pub(crate) fn pick_entity(
    sub_apps: &mut SubApps,
    image: AssetId<Image>,
    cursor: Vec2,
) -> Option<String> {
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
    parents: Query<&ChildOf>,
) -> Option<String> {
    let (image, cursor) = input.0;
    let (cam, cam_xf, _) = cameras
        .iter()
        .find(|(_, _, rt)| matches!(rt, RenderTarget::Image(h) if h.handle.id() == image))?;
    let ray = cam.viewport_to_world(cam_xf, cursor).ok()?;
    let settings = MeshRayCastSettings::default().with_visibility(RayCastVisibility::Any);
    let hits = ray_cast.cast_ray(ray, &settings);
    let (entity, _) = hits.first()?;
    resolve_editor_id(*entity, &id_map, &parents)
}

/// Walk up the parent chain until an entity mapped to an editor id is found.
/// Lets clicks on a light's hidden proxy child resolve to the light's id.
fn resolve_editor_id(
    mut entity: Entity,
    id_map: &EditorIdMap,
    parents: &Query<&ChildOf>,
) -> Option<String> {
    loop {
        if let Some(id) = id_map.rev.get(&entity) {
            return Some(id.clone());
        }
        let parent = parents.get(entity).ok()?.0;
        if parent == entity {
            return None;
        }
        entity = parent;
    }
}

/// Set the selected entity from an editor id (None clears).
pub(crate) fn set_selection(sub_apps: &mut SubApps, id: Option<String>) {
    let world = sub_apps.main.world_mut();
    world.init_resource::<EditorSelection>();
    let entity = id
        .as_ref()
        .and_then(|id| world.resource::<EditorIdMap>().fwd.get(id).copied());
    world.resource_mut::<EditorSelection>().selected = entity;
}
