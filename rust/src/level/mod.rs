//! Scene/Level module (≈ Flax `Source/Engine/Level/`).
//!
//! Owns the live ECS scene built from the editor's scene tree. The editor pushes
//! its whole tree as JSON via `set_scene`; `rebuild_scene` diffs it against the
//! world (spawn / patch / despawn), keyed by stable editor ids via `EditorIdMap`.
//! The editor camera is viewport-owned and never touched here. Light spawn/patch
//! logic lives in the [`crate::light`] module.

pub mod schema;

use std::collections::{HashMap, HashSet};

use bevy::app::SubApps;
use bevy::ecs::resource::Resource;
use bevy::prelude::*;

use crate::light;
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
/// On the first push, the startup fallback scene is despawned so it never
/// doubles up with the editor's scene (otherwise two overlapping lights/meshes
/// would leave the editor light with no visible effect on the scene).
pub(crate) fn rebuild_scene(sub_apps: &mut SubApps, json: &str) {
    let doc: SceneDoc = match serde_json::from_str(json) {
        Ok(d) => d,
        Err(e) => {
            warn!("set_scene: failed to parse scene JSON: {e}");
            return;
        }
    };

    let world = sub_apps.main.world_mut();
    let first_scene = world.get_resource::<SceneInitialized>().is_none();
    world.init_resource::<EditorIdMap>();
    world.insert_resource(SceneInitialized);
    if first_scene {
        light::despawn_fallback(world);
    }

    let mut seen: HashSet<String> = HashSet::with_capacity(doc.entities.len());

    for def in &doc.entities {
        if def.kind == "camera" {
            continue; // camera is viewport-owned
        }
        if def.kind == "light:ambient" {
            // Ambient light maps to the global resource, not a world entity.
            light::apply_ambient_light(world, def);
            continue;
        }
        seen.insert(def.id.clone());

        let existing = world.resource::<EditorIdMap>().fwd.get(&def.id).copied();

        if let Some(entity) = existing {
            if def.kind.starts_with("light:") {
                // Lights respawn uniformly: handles kind changes + field updates
                // without per-component-type patch branches.
                world.despawn(entity);
                {
                    let mut map = world.resource_mut::<EditorIdMap>();
                    map.rev.remove(&entity);
                }
                let new_entity = spawn_entity(world, def);
                let mut map = world.resource_mut::<EditorIdMap>();
                map.fwd.insert(def.id.clone(), new_entity);
                map.rev.insert(new_entity, def.id.clone());
            } else {
                update_entity(world, entity, def);
            }
        } else {
            let entity = spawn_entity(world, def);
            let mut map = world.resource_mut::<EditorIdMap>();
            map.fwd.insert(def.id.clone(), entity);
            map.rev.insert(entity, def.id.clone());
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
/// Mesh kinds are handled inline; light kinds delegate to [`light::spawn_light`].
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
        k if k.starts_with("light:") => light::spawn_light(world, def),
        other => {
            warn!("set_scene: unknown entity kind '{other}', spawning empty");
            world.spawn(transform).id()
        }
    }
}

/// Patch an existing mesh entity's transform and material color.
/// Lights are respawned by `rebuild_scene` (kind/field changes), not patched here.
fn update_entity(world: &mut World, entity: Entity, def: &SceneEntityDef) {
    if let Some(mut t) = world.get_mut::<Transform>(entity) {
        *t = def.transform.to_bevy();
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
}

/// Fallback scene used only if the editor hasn't pushed a scene yet (avoids a
/// black viewport during startup). Once `set_scene` runs, `SceneInitialized`
/// exists, this becomes a no-op, and the first `set_scene` despawns these
/// fallback entities (tagged with [`light::FallbackMarker`]) so they don't
/// double up with the editor's scene.
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
        light::FallbackMarker,
    ));
    world.spawn((
        Mesh3d(plane),
        MeshMaterial3d(plane_mat),
        Transform::default(),
        light::FallbackMarker,
    ));
    world.spawn((
        DirectionalLight {
            illuminance: light::brightness_to_illuminance(light::DEFAULT_BRIGHTNESS),
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(3.0, 8.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        light::FallbackMarker,
    ));
}