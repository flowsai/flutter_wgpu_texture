//! Native Bevy scene file (`.scn.ron`) save/load via `DynamicWorld`.
//!
//! `save_scene` serializes the editor's scene entities (those carrying a
//! `SceneObjectId`) to the Bevy world format. `load_scene` deserializes a
//! `.scn`/`.scn.ron` back into the live world and rebuilds the `EditorIdMap`
//! from the persisted `SceneObjectId` components. The runtime loads these same
//! files with the standard Bevy loader.

use bevy::asset::LoadFromPath;
use bevy::ecs::entity::EntityHashMap;
use bevy::prelude::*;
use bevy::reflect::TypeRegistry;
use bevy::world_serialization::serde::WorldDeserializer;
use bevy::world_serialization::{DynamicWorld, DynamicWorldBuilder, WorldFilter};
use serde::de::DeserializeSeed;

use super::components::SceneObjectId;
use super::EditorIdMap;

/// Components that are editor-internal, derived, or asset-handle-based and must
/// not be serialized into the scene file.
fn scene_component_filter() -> WorldFilter {
    WorldFilter::allow_all()
        .deny::<Children>()
        .deny::<GlobalTransform>()
        .deny::<ShowLightGizmo>()
        .deny::<crate::light::FallbackMarker>()
        .deny::<Mesh3d>()
        .deny::<MeshMaterial3d<StandardMaterial>>()
}

/// Serialize the editor scene (entities with a `SceneObjectId`) to a `.scn.ron`
/// string.
pub fn serialize_scene(world: &mut World) -> Result<String, String> {
    let type_registry_arc = world.resource::<AppTypeRegistry>().clone();
    let type_registry = type_registry_arc.read();

    let scene_entities: Vec<Entity> = world
        .query_filtered::<Entity, With<SceneObjectId>>()
        .iter(world)
        .collect();

    let dynamic = DynamicWorldBuilder::from_world(world, &type_registry)
        .extract_entities(scene_entities.into_iter())
        .with_component_filter(scene_component_filter())
        .build();

    dynamic.serialize(&type_registry).map_err(|e| e.to_string())
}

/// Restore a scene from a `.scn.ron` string into the live world, replacing the
/// current scene entities and rebuilding the `EditorIdMap`.
pub fn restore_scene(world: &mut World, ron: &str) -> Result<(), String> {
    let type_registry_arc = world.resource::<AppTypeRegistry>().clone();
    let type_registry = type_registry_arc.read();

    let dynamic = {
        let asset_server = world.resource::<AssetServer>();
        let mut lfp: &AssetServer = &*asset_server;
        load_dynamic_world(ron.as_bytes(), &type_registry, &mut lfp)?
    };
    spawn_dynamic_world(world, &dynamic, &type_registry)
}

/// Serialize the editor scene (entities with a `SceneObjectId`) to `.scn.ron`.
pub fn save_scene(world: &mut World, path: &str) -> Result<(), String> {
    let ron = serialize_scene(world)?;
    std::fs::write(path, ron).map_err(|e| e.to_string())
}

/// Deserialize a `.scn`/`.scn.ron` byte slice into a [`DynamicWorld`].
pub fn load_dynamic_world(
    bytes: &[u8],
    type_registry: &TypeRegistry,
    load_from_path: &mut dyn LoadFromPath,
) -> Result<DynamicWorld, String> {
    let mut de = ron::de::Deserializer::from_bytes(bytes).map_err(|e| e.to_string())?;
    let scene_de = WorldDeserializer {
        type_registry,
        load_from_path,
    };
    scene_de
        .deserialize(&mut de)
        .map_err(|e| format!("{e}"))
}

/// Clear the current editor scene and spawn a [`DynamicWorld`] into the live
/// world, rebuilding the `EditorIdMap` from the persisted `SceneObjectId`s.
pub fn spawn_dynamic_world(
    world: &mut World,
    dynamic: &DynamicWorld,
    type_registry: &TypeRegistry,
) -> Result<(), String> {
    // Despawn the current scene (ChildOf cascades to children).
    let to_despawn: Vec<Entity> = world
        .query_filtered::<Entity, With<SceneObjectId>>()
        .iter(world)
        .collect();
    for entity in to_despawn {
        if world.get_entity_mut(entity).is_ok() {
            world.despawn(entity);
        }
    }
    {
        let mut map = world.resource_mut::<EditorIdMap>();
        map.fwd.clear();
        map.rev.clear();
    }

    let mut entity_map: EntityHashMap<Entity> = default();
    dynamic
        .write_to_world_with(world, &mut entity_map, type_registry)
        .map_err(|e| format!("{e}"))?;

    // Rebuild the id map from the persisted stable ids.
    let ids: Vec<(Entity, String)> = world
        .query::<(Entity, &SceneObjectId)>()
        .iter(world)
        .map(|(e, s)| (e, s.0.clone()))
        .collect();
    let mut map = world.resource_mut::<EditorIdMap>();
    for (entity, id) in ids {
        map.fwd.insert(id.clone(), entity);
        map.rev.insert(entity, id);
    }
    Ok(())
}

/// Load a `.scn`/`.scn.ron` file into the live world.
pub fn load_scene(world: &mut World, path: &str) -> Result<(), String> {
    let type_registry_arc = world.resource::<AppTypeRegistry>().clone();
    let type_registry = type_registry_arc.read();

    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;

    let dynamic = {
        let asset_server = world.resource::<AssetServer>();
        let mut lfp: &AssetServer = &*asset_server;
        load_dynamic_world(&bytes, &type_registry, &mut lfp)?
    };

    spawn_dynamic_world(world, &dynamic, &type_registry)
}

fn default<T: Default + Clone>() -> T {
    T::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::{AssetPath, UntypedHandle};
    use bevy::ecs::reflect::AppTypeRegistry;

    /// `LoadFromPath` that returns a default handle. The round-trip scene has no
    /// asset-path references, so this is never meaningfully called.
    struct NoOpLoad;
    impl LoadFromPath for NoOpLoad {
        fn load_from_path_erased(
            &mut self,
            type_id: std::any::TypeId,
            _path: AssetPath<'static>,
        ) -> UntypedHandle {
            UntypedHandle::default_for_type(type_id)
        }
    }

    fn world_with_registry() -> World {
        let mut world = World::new();
        let atr = AppTypeRegistry::default();
        {
            let mut w = atr.write();
            w.register::<SceneObjectId>();
            w.register::<Transform>();
            w.register::<Name>();
            w.register::<ChildOf>();
            w.register::<crate::level::primitives::PrimitiveMesh>();
            w.register::<crate::level::primitives::MaterialColor>();
        }
        world.insert_resource(atr);
        world.init_resource::<EditorIdMap>();
        world
    }

    #[test]
    fn round_trips_scene_object_id_transform_and_hierarchy() {
        let mut world = world_with_registry();

        let parent = world
            .spawn((
                SceneObjectId("parent".to_string()),
                Name::new("Parent"),
                Transform::from_xyz(1.0, 2.0, 3.0),
            ))
            .id();
        let child = world
            .spawn((
                SceneObjectId("child".to_string()),
                Name::new("Child"),
                Transform::from_xyz(0.0, 1.0, 0.0),
                ChildOf(parent),
            ))
            .id();

        let dir = std::env::temp_dir();
        let path = dir.join("bevyflow_t04_roundtrip.scn.ron");
        save_scene(&mut world, path.to_str().unwrap()).expect("save");

        // Load into a fresh world.
        let mut dst = world_with_registry();
        let bytes = std::fs::read(&path).expect("read");
        let type_registry = dst.resource::<AppTypeRegistry>().clone();
        let guard = type_registry.read();
        let mut load = NoOpLoad;
        let dynamic = load_dynamic_world(&bytes, &guard, &mut load).expect("deserialize");
        spawn_dynamic_world(&mut dst, &dynamic, &guard).expect("spawn");

        let map = dst.resource::<EditorIdMap>();
        let loaded_parent = map.fwd["parent"];
        let loaded_child = map.fwd["child"];
        assert_ne!(loaded_parent, parent, "entity ids are remapped");
        assert_eq!(
            dst.get::<Transform>(loaded_parent)
                .unwrap()
                .translation
                .x,
            1.0
        );
        assert_eq!(
            dst.get::<ChildOf>(loaded_child).unwrap().parent(),
            loaded_parent,
            "hierarchy preserved"
        );
        assert_eq!(dst.get::<Name>(loaded_child).unwrap().as_str(), "Child");

        let _ = std::fs::remove_file(&path);
        // Silence unused `world` entity ids.
        let _ = child;
    }

    #[test]
    fn round_trips_primitive_mesh_and_material_color() {
        use crate::level::primitives::{MaterialColor, PrimitiveMesh};

        let mut world = world_with_registry();
        world
            .spawn((
                SceneObjectId("cube1".to_string()),
                Name::new("Cube1"),
                Transform::from_xyz(1.0, 2.0, 3.0),
                PrimitiveMesh("cube".to_string()),
                MaterialColor([0.4, 0.6, 0.9, 1.0]),
            ))
            .id();

        let dir = std::env::temp_dir();
        let path = dir.join("bevyflow_t05_assets.scn.ron");
        save_scene(&mut world, path.to_str().unwrap()).expect("save");

        let mut dst = world_with_registry();
        let bytes = std::fs::read(&path).expect("read");
        let type_registry = dst.resource::<AppTypeRegistry>().clone();
        let guard = type_registry.read();
        let mut load = NoOpLoad;
        let dynamic = load_dynamic_world(&bytes, &guard, &mut load).expect("deserialize");
        spawn_dynamic_world(&mut dst, &dynamic, &guard).expect("spawn");

        let map = dst.resource::<EditorIdMap>();
        let cube = map.fwd["cube1"];
        assert_eq!(dst.get::<PrimitiveMesh>(cube).unwrap().0, "cube");
        assert_eq!(dst.get::<MaterialColor>(cube).unwrap().0, [0.4, 0.6, 0.9, 1.0]);
        // Mesh3d/MeshMaterial3d are NOT in the file (denied) — the materialize
        // system rebuilds them at runtime.
        assert!(dst.get::<Mesh3d>(cube).is_none());

        let _ = std::fs::remove_file(&path);
    }
}