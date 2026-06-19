//! Physics for play mode. Scene entities declare their physics intent with a
//! [`RigidBodyDef`] component (authored in the editor, inert while editing and
//! persisted with the scene). Entering play instantiates the simulation bodies
//! from those descriptors and unpauses the physics clock; exiting play pauses it
//! and restores the authored scene, which removes the simulation-only bodies.

use avian3d::prelude::*;
use bevy::prelude::*;

/// How a body responds to simulation, mirroring the authored choice.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyType {
    #[default]
    Dynamic,
    Static,
    Kinematic,
}

/// Collider shape authored for an entity. `Auto` derives a box from the
/// entity's transform scale (a unit cube scaled by the transform).
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq)]
pub enum ColliderShape {
    #[default]
    Auto,
    Cuboid,
    Sphere,
    HalfSpace,
}

/// Authored physics intent on a scene entity. Inert while editing; the body and
/// collider are created from it when entering play.
#[derive(Component, Reflect, Default, Debug, Clone)]
#[reflect(Component)]
pub struct RigidBodyDef {
    pub body: BodyType,
    pub collider: ColliderShape,
}

fn body_component(body: BodyType) -> RigidBody {
    match body {
        BodyType::Dynamic => RigidBody::Dynamic,
        BodyType::Static => RigidBody::Static,
        BodyType::Kinematic => RigidBody::Kinematic,
    }
}

fn collider_component(shape: ColliderShape, scale: Vec3) -> Collider {
    match shape {
        ColliderShape::Sphere => Collider::sphere(0.5 * scale.x.abs().max(f32::EPSILON)),
        ColliderShape::HalfSpace => Collider::half_space(Vec3::Y),
        ColliderShape::Cuboid | ColliderShape::Auto => {
            let half = (scale * 0.5).abs();
            Collider::cuboid(
                half.x.max(f32::EPSILON) * 2.0,
                half.y.max(f32::EPSILON) * 2.0,
                half.z.max(f32::EPSILON) * 2.0,
            )
        }
    }
}

/// Create the simulation body + collider for every entity that declares a
/// [`RigidBodyDef`] but has no live body yet.
pub(crate) fn attach_play_bodies(world: &mut World) {
    let targets: Vec<(Entity, RigidBodyDef, Vec3)> = world
        .query_filtered::<(Entity, &RigidBodyDef, Option<&Transform>), Without<RigidBody>>()
        .iter(world)
        .map(|(e, def, xf)| (e, def.clone(), xf.map(|t| t.scale).unwrap_or(Vec3::ONE)))
        .collect();

    for (entity, def, scale) in targets {
        let Ok(mut e) = world.get_entity_mut(entity) else {
            continue;
        };
        e.insert((body_component(def.body), collider_component(def.collider, scale)));
    }
}

/// Start stepping the physics clock (called on enter play).
pub(crate) fn resume_simulation(world: &mut World) {
    if let Some(mut time) = world.get_resource_mut::<Time<Physics>>() {
        time.unpause();
    }
}

/// Stop stepping the physics clock (called on exit play / in editing).
pub(crate) fn pause_simulation(world: &mut World) {
    if let Some(mut time) = world.get_resource_mut::<Time<Physics>>() {
        time.pause();
    }
}
