//! Scene/Level module: owns the live ECS scene built from the editor's scene
//! tree. The editor pushes its whole tree as JSON via `set_scene`;
//! `rebuild_scene` diffs it against the world (spawn / patch / despawn), keyed
//! by stable editor ids. The editor camera is viewport-owned and never touched
//! here. Light spawn/patch logic lives in the [`crate::light`] module.

pub mod components;
pub mod physics;
pub mod play;
pub mod primitives;
pub mod scene_file;
pub mod schema;
pub mod view_mode;

use std::collections::{HashMap, HashSet};

use bevy::app::SubApps;
use bevy::ecs::resource::Resource;
use bevy::prelude::*;
use bevy::reflect::TypeRegistry;

use bevy::light::{Atmosphere, atmosphere::ScatteringMedium};

use crate::light;
use components::{spawn_components, SceneObjectId, SkyAtmosphere};
use schema::{SceneDoc, SceneEntityDef};

/// Register every component type that may appear on a scene entity, so the
/// scene snapshot (`scene_file::serialize_scene`) captures it. Types not in the
/// registry are silently dropped from the snapshot. `Transform`/`Name`/`ChildOf`
/// and the light types are NOT registered by their Bevy plugins, so they are
/// registered here alongside the editor's own components.
pub(crate) fn register_scene_types(registry: &mut TypeRegistry) {
    registry.register::<SceneObjectId>();
    registry.register::<primitives::PrimitiveMesh>();
    registry.register::<primitives::MaterialColor>();
    registry.register::<physics::RigidBodyDef>();
    registry.register::<Transform>();
    registry.register::<Name>();
    registry.register::<ChildOf>();
    registry.register::<DirectionalLight>();
    registry.register::<PointLight>();
    registry.register::<SpotLight>();
    registry.register::<bevy::light::RectLight>();
    registry.register::<SkyAtmosphere>();
}

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

    // Clone the AppTypeRegistry handle so the read guard borrows the Arc, not
    // the world, and the world stays mutably usable.
    let type_registry_arc = world.resource::<AppTypeRegistry>().clone();
    let type_registry = type_registry_arc.read();

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
                let new_entity = spawn_entity(world, def, &type_registry);
                let mut map = world.resource_mut::<EditorIdMap>();
                map.fwd.insert(def.id.clone(), new_entity);
                map.rev.insert(new_entity, def.id.clone());
            } else {
                update_entity(world, entity, def, &type_registry);
            }
        } else {
            let entity = spawn_entity(world, def, &type_registry);
            let mut map = world.resource_mut::<EditorIdMap>();
            map.fwd.insert(def.id.clone(), entity);
            map.rev.insert(entity, def.id.clone());
        }
    }

    // Wire `ChildOf` from `parent_id` so child transforms are parent-relative.
    let links: Vec<(Entity, Entity)> = {
        let map = world.resource::<EditorIdMap>();
        doc.entities
            .iter()
            .filter_map(|def| {
                let parent_id = def.parent_id.as_ref()?;
                let child = map.fwd.get(&def.id).copied()?;
                let parent = map.fwd.get(parent_id).copied()?;
                Some((child, parent))
            })
            .collect()
    };
    for (child, parent) in links {
        if let Ok(mut e) = world.get_entity_mut(child) {
            e.insert(ChildOf(parent));
        }
    }

    // Despawn entities no longer present in the doc. `ChildOf` cascades
    // despawn to children, so guard against already-despawned entities.
    let to_remove: Vec<(String, Entity)> = world
        .resource::<EditorIdMap>()
        .fwd
        .iter()
        .filter(|(id, _)| !seen.contains(*id))
        .map(|(id, e)| (id.clone(), *e))
        .collect();
    for (id, entity) in to_remove {
        if world.get_entity_mut(entity).is_ok() {
            world.despawn(entity);
        }
        let mut map = world.resource_mut::<EditorIdMap>();
        map.fwd.remove(&id);
        map.rev.remove(&entity);
    }
}

/// Spawn a brand-new entity from its editor definition. Returns the Bevy entity.
/// Mesh kinds are handled inline; light kinds delegate to [`light::spawn_light`].
/// Every entity also gets a [`SceneObjectId`], a `Name`, and its reflected
/// add-on components.
fn spawn_entity(world: &mut World, def: &SceneEntityDef, type_registry: &TypeRegistry) -> Entity {
    let transform = def.transform.to_bevy();
    let id = SceneObjectId(def.id.clone());
    let name = Name::new(def.name.clone());

    let mut entity_world_mut = match def.kind.as_str() {
        "mesh:cube" | "mesh:plane" => {
            let prim = if def.kind == "mesh:plane" { "plane" } else { "cube" };
            let color = def
                .material
                .as_ref()
                .map(|m| m.color)
                .unwrap_or([0.8, 0.8, 0.8, 1.0]);
            world.spawn((
                transform,
                primitives::PrimitiveMesh(prim.to_string()),
                primitives::MaterialColor(color),
            ))
        }
        k if k.starts_with("light:") => {
            let entity = light::spawn_light(world, def);
            world.entity_mut(entity)
        }
        "sky:atmosphere" => {
            let atmo = make_atmosphere(world);
            world.spawn((SkyAtmosphere, atmo))
        }
        "actor:empty" => world.spawn(transform),
        other => {
            warn!("set_scene: unknown entity kind '{other}', spawning empty");
            world.spawn(transform)
        }
    };

    entity_world_mut.insert((id, name));
    spawn_components(&mut entity_world_mut, &def.components, type_registry);
    entity_world_mut.id()
}

/// Build an `Atmosphere` with a freshly registered scattering medium. The medium
/// is an asset handle and is not part of the serialized scene, so it must be
/// recreated whenever an atmosphere entity is spawned or restored.
fn make_atmosphere(world: &mut World) -> Atmosphere {
    let medium = world
        .resource_mut::<Assets<ScatteringMedium>>()
        .add(ScatteringMedium::default());
    Atmosphere::earth(medium)
}

/// Re-attach the `Atmosphere` component to `SkyAtmosphere`-marked entities that
/// lack it. The atmosphere's `medium` is an asset handle, not serialized into
/// the scene, so entities brought in by the scene deserializer (load or
/// play-mode restore) carry only the `SkyAtmosphere` marker and must have their
/// `Atmosphere` recreated.
pub(crate) fn reestablish_atmosphere(world: &mut World) {
    if !world.contains_resource::<Assets<ScatteringMedium>>() {
        return;
    }
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, (With<SkyAtmosphere>, Without<Atmosphere>)>()
        .iter(world)
        .collect();
    for entity in entities {
        let atmo = make_atmosphere(world);
        world.entity_mut(entity).insert(atmo);
    }
}

/// Patch an existing entity's transform, material color, and reflected add-on
/// components. Lights are respawned by `rebuild_scene`, not patched here.
/// Re-parenting is handled by the hierarchy pass in `rebuild_scene`.
fn update_entity(world: &mut World, entity: Entity, def: &SceneEntityDef, type_registry: &TypeRegistry) {
    if let Some(mut t) = world.get_mut::<Transform>(entity) {
        *t = def.transform.to_bevy();
    }

    if let Some(mat_def) = def.material.as_ref() {
        let c = mat_def.color;
        if let Some(mut mc) = world.get_mut::<primitives::MaterialColor>(entity) {
            mc.0 = c;
        } else {
            world.entity_mut(entity).insert(primitives::MaterialColor(c));
        }
    }

    // Re-apply arbitrary reflected components (replace by re-inserting).
    if !def.components.is_empty() {
        if let Ok(mut entity_world_mut) = world.get_entity_mut(entity) {
            spawn_components(&mut entity_world_mut, &def.components, type_registry);
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

    world.spawn((
        primitives::PrimitiveMesh("cube".to_string()),
        primitives::MaterialColor([0.4, 0.6, 0.9, 1.0]),
        Transform::from_xyz(0.0, 6.0, 0.0),
        light::FallbackMarker,
    ));
    world.spawn((
        primitives::PrimitiveMesh("plane".to_string()),
        primitives::MaterialColor([0.3, 0.3, 0.3, 1.0]),
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