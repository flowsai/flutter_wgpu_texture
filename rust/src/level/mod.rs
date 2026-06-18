//! Scene/Level module (≈ Flax `Source/Engine/Level/`).
//!
//! Owns the live ECS scene built from the editor's scene tree. The editor pushes
//! its whole tree as JSON via `set_scene`; `rebuild_scene` diffs it against the
//! world (spawn / patch / despawn), keyed by stable editor ids via `EditorIdMap`.
//! The editor camera is viewport-owned and never touched here.

pub mod schema;

use std::collections::{HashMap, HashSet};

use bevy::app::SubApps;
use bevy::ecs::resource::Resource;
use bevy::prelude::*;

use schema::{SceneDoc, SceneEntityDef};

/// Maps the editor's stable string ids to live Bevy entities (both directions).
#[derive(Resource, Default)]
pub(crate) struct EditorIdMap {
    pub(crate) fwd: HashMap<String, Entity>,
    pub(crate) rev: HashMap<Entity, String>,
}

/// Set once the first `set_scene` has been applied; gates the fallback default scene.
#[derive(Resource)]
pub(crate) struct SceneInitialized;

/// Apply the editor scene tree (JSON) by diffing against the live world.
/// Spawns new entities, patches changed transforms/materials, despawns removed.
pub(crate) fn rebuild_scene(sub_apps: &mut SubApps, json: &str) {
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

        let existing = world.resource::<EditorIdMap>().fwd.get(&def.id).copied();

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
        world
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .map(|m| m.0.clone()),
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
pub(crate) fn ensure_default_scene(sub_apps: &mut SubApps) {
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
