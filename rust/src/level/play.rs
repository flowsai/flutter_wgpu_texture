//! Editor play state. The editor runs in `Editing` (scene authored, gameplay
//! systems and physics inert, editor gizmos drawn) or `Playing` (gameplay
//! systems and physics step, editor gizmos hidden). Entering play snapshots the
//! authored scene; exiting play restores it, so changes made while playing are
//! discarded.

use bevy::prelude::*;

use super::components::SceneObjectId;
use super::{physics, scene_file, EditorIdMap};
use crate::picking::EditorSelection;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum PlayMode {
    #[default]
    Editing,
    Playing,
}

/// Current editor play mode plus the authored-scene snapshot captured on enter,
/// restored on exit.
#[derive(Resource, Default)]
pub(crate) struct PlayState {
    pub(crate) mode: PlayMode,
    snapshot: Option<String>,
}

impl PlayState {
    pub(crate) fn is_playing(&self) -> bool {
        self.mode == PlayMode::Playing
    }
}

/// Play lifecycle phases, triggered around the enter/exit sequence so gameplay
/// systems can initialize and tear down. `Beginning`/`Ending` fire before the
/// scene is set up / torn down; `Begin`/`End` fire after.
#[derive(Event, Clone, Copy, Debug)]
pub(crate) enum PlayLifecycle {
    Beginning,
    Begin,
    Ending,
    End,
}

/// Enter play mode: snapshot the authored scene and switch to `Playing`.
/// A no-op if already playing.
pub(crate) fn enter_play(world: &mut World) {
    world.init_resource::<PlayState>();
    if world.resource::<PlayState>().is_playing() {
        return;
    }
    world.trigger(PlayLifecycle::Beginning);
    match scene_file::serialize_scene(world) {
        Ok(snapshot) => {
            {
                let mut state = world.resource_mut::<PlayState>();
                state.snapshot = Some(snapshot);
                state.mode = PlayMode::Playing;
            }
            physics::attach_play_bodies(world);
            physics::resume_simulation(world);
            world.trigger(PlayLifecycle::Begin);
        }
        Err(e) => warn!("enter_play: failed to snapshot scene: {e}"),
    }
}

/// Exit play mode: restore the authored scene from the snapshot and switch back
/// to `Editing`. A no-op if not playing.
pub(crate) fn exit_play(world: &mut World) {
    world.init_resource::<PlayState>();
    if !world.resource::<PlayState>().is_playing() {
        return;
    }
    world.trigger(PlayLifecycle::Ending);
    physics::pause_simulation(world);

    // Restoring despawns and respawns every scene entity, so the selection's
    // entity reference goes stale. Cache the selected stable id, restore, then
    // re-resolve it against the rebuilt id map.
    let selected_id = selected_object_id(world);

    // Discard anything spawned during play, then replace the authored scene from
    // the snapshot, so nothing from the play session leaks into edit mode.
    scene_file::despawn_play_spawned(world);
    let snapshot = world.resource_mut::<PlayState>().snapshot.take();
    if let Some(ron) = snapshot {
        if let Err(e) = scene_file::restore_scene(world, &ron) {
            warn!("exit_play: failed to restore scene: {e}");
        }
    }

    restore_selection(world, selected_id);
    world.resource_mut::<PlayState>().mode = PlayMode::Editing;
    world.trigger(PlayLifecycle::End);
}

/// The stable id of the currently selected entity, if any.
fn selected_object_id(world: &mut World) -> Option<String> {
    let selected = world.get_resource::<EditorSelection>()?.selected?;
    world.get::<SceneObjectId>(selected).map(|s| s.0.clone())
}

/// Re-point the selection at the entity now carrying `id`, or clear it.
fn restore_selection(world: &mut World, id: Option<String>) {
    let entity = id.and_then(|id| world.resource::<EditorIdMap>().fwd.get(&id).copied());
    if let Some(mut sel) = world.get_resource_mut::<EditorSelection>() {
        sel.selected = entity;
    }
}

/// Set the play mode from a string command (`"play"` | `"edit"`).
pub(crate) fn set_play_mode(world: &mut World, mode: &str) {
    match mode {
        "play" => enter_play(world),
        "edit" => exit_play(world),
        other => warn!("set_play_mode: unknown mode '{other}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::components::SceneObjectId;
    use crate::level::{register_scene_types, EditorIdMap};
    use bevy::asset::AssetPlugin;
    use bevy::ecs::reflect::AppTypeRegistry;

    // Seed the registry the same way the running app does, so the snapshot
    // captures exactly what production captures (catches missing registrations).
    fn test_world() -> World {
        // AssetServer is required by restore_scene's deserialize path.
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        let world = app.world_mut();
        let atr = world.get_resource_or_init::<AppTypeRegistry>().clone();
        register_scene_types(&mut atr.write());
        world.init_resource::<EditorIdMap>();
        std::mem::take(world)
    }

    #[test]
    fn play_then_stop_restores_authored_transform() {
        let mut world = test_world();
        let cube = world
            .spawn((
                SceneObjectId("cube".to_string()),
                Name::new("Cube"),
                Transform::from_xyz(0.0, 5.0, 0.0),
            ))
            .id();

        enter_play(&mut world);
        assert!(world.resource::<PlayState>().is_playing());

        // Simulate the cube falling during play (play runs on the live entities).
        world.get_mut::<Transform>(cube).unwrap().translation.y = 0.0;

        exit_play(&mut world);
        assert!(!world.resource::<PlayState>().is_playing());

        // The authored position is restored (the entity is respawned, so look it
        // up again via the rebuilt id map).
        let restored = world.resource::<EditorIdMap>().fwd["cube"];
        assert_eq!(
            world.get::<Transform>(restored).unwrap().translation.y,
            5.0,
            "authored transform restored on stop"
        );
    }

    #[test]
    fn play_spawned_entities_are_discarded_on_stop() {
        let mut world = test_world();
        world.spawn((
            SceneObjectId("cube".to_string()),
            Name::new("Cube"),
            Transform::from_xyz(0.0, 5.0, 0.0),
        ));

        enter_play(&mut world);
        // A projectile spawned at runtime during play (no SceneObjectId).
        let projectile = world.spawn((Name::new("Projectile"), Transform::default())).id();
        exit_play(&mut world);

        assert!(
            world.get_entity(projectile).is_err(),
            "runtime-spawned entity is discarded on stop"
        );
        assert!(
            world.resource::<EditorIdMap>().fwd.contains_key("cube"),
            "authored entity is restored"
        );
    }

    #[test]
    fn lights_stay_constant_across_play_cycles() {
        let mut world = test_world();
        world.spawn((
            SceneObjectId("sun".to_string()),
            Name::new("Sun"),
            Transform::from_xyz(3.0, 8.0, 5.0),
            DirectionalLight::default(),
        ));

        for _ in 0..5 {
            enter_play(&mut world);
            exit_play(&mut world);
            let count = world
                .query_filtered::<(), With<DirectionalLight>>()
                .iter(&world)
                .count();
            assert_eq!(count, 1, "exactly one light after each play/stop cycle");
        }
    }
}
