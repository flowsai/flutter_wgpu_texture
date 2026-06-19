//! Light domain (≈ Flax `Source/Engine/Level/Actors/Light*.cpp`).
//!
//! Flax-style light model: every light has a `Color` + `Brightness` (a single
//! multiplier, default 3.14 — same as Flax's `Light::Brightness`). The editor
//! pushes these via the scene JSON (`LightDef`); here we map them to Bevy's
//! physical-unit components — lux for `DirectionalLight`, lumens for
//! `PointLight`/`SpotLight`/`RectLight` — using tuned scales so the default
//! reads as a clearly-lit daylight scene under the camera's default exposure
//! (`Exposure::BLENDER`, ev100 9.7 → exposure ≈ 0.001). `light:ambient` maps to
//! the `GlobalAmbientLight` resource (single global fill), like Flax's
//! `SkyLight`/ambient.
//!
//! Each light also gets a `ShowLightGizmo` (Bevy's per-type wireframe icon — the
//! Flax `AddViewportIcon` equivalent) and a hidden proxy mesh child so the
//! editor's mesh picker can select it (Flax `IntersectsItself` equivalent; the
//! directional light returns false in Flax and is selected via its icon — our
//! proxy covers all types uniformly). Moving a light uses the editor's existing
//! transform gizmo, exactly like Flax's move tool on a light Actor.

use bevy::prelude::*;

use crate::level::schema::{LightDef, SceneEntityDef};

/// Flax default light brightness (`Light::Brightness = 3.14f`).
pub const DEFAULT_BRIGHTNESS: f32 = 3.14;

/// lux per brightness unit (directional). `3.14 * 4000 ≈ 12.5k lux` ≈ daylight.
const DIR_SCALE: f32 = 4000.0;
/// lumens per brightness unit (point/spot/rect). `3.14 * 320k ≈ 1M lm` = Bevy's
/// `VERY_LARGE_CINEMA_LIGHT`, which reads brightly at the default exposure.
const POINT_SCALE: f32 = 320_000.0;

/// Marker component for `ensure_default_scene` entities, despawned on the first
/// editor `set_scene` so the fallback scene never doubles up with the editor's.
#[derive(Component)]
pub struct FallbackMarker;

/// Marker on a light's viewport-marker mesh child, so it can be hidden in play.
#[derive(Component)]
pub struct LightProxy;

pub(crate) fn brightness_to_illuminance(b: f32) -> f32 {
    (b * DIR_SCALE).max(0.0)
}
pub(crate) fn brightness_to_lumens(b: f32) -> f32 {
    (b * POINT_SCALE).max(0.0)
}

/// Resolve a light's color from its def, defaulting to white (Flax `Color::White`).
fn light_color(l: Option<&LightDef>) -> Color {
    l.and_then(|l| l.color)
        .map(|c| Color::srgba(c[0], c[1], c[2], c[3]))
        .unwrap_or(Color::WHITE)
}

/// Resolve a light's brightness, defaulting to the Flax default.
fn light_brightness(l: Option<&LightDef>) -> f32 {
    l.and_then(|l| l.brightness)
        .filter(|b| *b >= 0.0)
        .unwrap_or(DEFAULT_BRIGHTNESS)
}

/// Resolve an optional f32 light field with a fallback.
fn field_f32(l: Option<&LightDef>, pick: impl Fn(&LightDef) -> Option<f32>) -> Option<f32> {
    l.and_then(pick).filter(|v| v.is_finite())
}

/// Spawn a light entity from its editor definition. Returns the Bevy entity.
/// Handles `light:directional|point|spot|rect`. (`light:ambient` is a resource,
/// not an entity — see `apply_ambient_light`.)
pub(crate) fn spawn_light(world: &mut World, def: &SceneEntityDef) -> Entity {
    let transform = def.transform.to_bevy();
    let l = def.light.as_ref();
    let brightness = light_brightness(l);

    let light = match def.kind.as_str() {
        "light:directional" => world
            .spawn((
                DirectionalLight {
                    color: light_color(l),
                    illuminance: brightness_to_illuminance(brightness),
                    shadow_maps_enabled: l.and_then(|l| l.shadow_maps_enabled).unwrap_or(true),
                    ..default()
                },
                light_transform(&transform),
                ShowLightGizmo::default(),
            ))
            .id(),
        "light:point" => world
            .spawn((
                PointLight {
                    color: light_color(l),
                    intensity: brightness_to_lumens(brightness),
                    range: field_f32(l, |x| x.range).unwrap_or(20.0),
                    radius: field_f32(l, |x| x.radius).unwrap_or(0.0),
                    shadow_maps_enabled: l.and_then(|l| l.shadow_maps_enabled).unwrap_or(false),
                    ..default()
                },
                Transform::from_translation(transform.translation),
                ShowLightGizmo::default(),
            ))
            .id(),
        "light:spot" => {
            let defaults = SpotLight::default();
            world
                .spawn((
                    SpotLight {
                        color: light_color(l),
                        intensity: brightness_to_lumens(brightness),
                        range: field_f32(l, |x| x.range).unwrap_or(defaults.range),
                        radius: field_f32(l, |x| x.radius).unwrap_or(defaults.radius),
                        inner_angle: field_f32(l, |x| x.inner_angle).unwrap_or(defaults.inner_angle),
                        outer_angle: field_f32(l, |x| x.outer_angle).unwrap_or(defaults.outer_angle),
                        shadow_maps_enabled: l.and_then(|l| l.shadow_maps_enabled).unwrap_or(false),
                        ..default()
                    },
                    light_transform(&transform),
                    ShowLightGizmo::default(),
                ))
                .id()
        }
        "light:rect" => world
            .spawn((
                RectLight {
                    color: light_color(l),
                    intensity: brightness_to_lumens(brightness),
                    range: field_f32(l, |x| x.range).unwrap_or(20.0),
                    width: field_f32(l, |x| x.width).unwrap_or(1.0),
                    height: field_f32(l, |x| x.height).unwrap_or(1.0),
                    ..default()
                },
                transform,
                ShowLightGizmo::default(),
            ))
            .id(),
        other => {
            warn!("light::spawn_light: unknown light kind '{other}', spawning empty");
            world.spawn(transform).id()
        }
    };

    // Visible marker + picker proxy child (Flax IntersectsItself/viewport-icon).
    spawn_light_proxy(world, light);
    light
}

/// Spawn a small unlit marker mesh as a child of a light entity so the user
/// can SEE the light position in the viewport (Flax `AddViewportIcon`
/// equivalent — a 3D marker) and the editor's mesh picker can hit it (Flax
/// `IntersectsItself` equivalent). It is `unlit` + `NotShadowCaster` so it never
/// affects scene lighting/shadows. `pick_system` walks up `ChildOf` to resolve
/// the editor id. The light's own visibility (which gates illumination) is
/// unaffected. Keep `ShowLightGizmo` for the per-type shape/direction icon.
fn spawn_light_proxy(world: &mut World, light: Entity) {
    let mesh = {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        meshes.add(Sphere::new(0.2).mesh().uv(12, 8))
    };
    let material = {
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
        materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.85, 0.3),
            unlit: true,
            ..default()
        })
    };
    world.entity_mut(light).with_children(|c| {
        c.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            bevy::light::NotShadowCaster,
            Transform::default(),
            LightProxy,
        ));
    });
}

/// Show or hide all editor overlays for lights: the per-type direction/shape
/// gizmos (`ShowLightGizmo`, drawn by Bevy) and the marker-mesh proxies. Hidden
/// while playing so the viewport shows only the game, like Flax/Unity.
pub(crate) fn set_light_overlays_visible(world: &mut World, visible: bool) {
    {
        use bevy::gizmos::config::GizmoConfigStore;
        // The gizmo store is absent in headless tests.
        if let Some(mut store) = world.get_resource_mut::<GizmoConfigStore>() {
            store.config_mut::<LightGizmoConfigGroup>().0.enabled = visible;
        }
    }
    let proxies: Vec<Entity> = world
        .query_filtered::<Entity, With<LightProxy>>()
        .iter(world)
        .collect();
    let vis = if visible { Visibility::Inherited } else { Visibility::Hidden };
    for proxy in proxies {
        if let Ok(mut e) = world.get_entity_mut(proxy) {
            e.insert(vis);
        }
    }
}

/// Re-attach the editor gizmo + picker proxy to light entities that lack them.
/// Lights brought in by the scene deserializer carry only their reflected light
/// component; the proxy child and `ShowLightGizmo` are editor-side and not part
/// of the scene, so they must be recreated when a scene is loaded or restored.
pub(crate) fn reestablish_light_proxies(world: &mut World) {
    // The proxy needs mesh + material storages; without them (headless tests)
    // there is nothing to draw, so skip.
    if !world.contains_resource::<Assets<Mesh>>()
        || !world.contains_resource::<Assets<StandardMaterial>>()
    {
        return;
    }
    let lights: Vec<Entity> = world
        .query_filtered::<Entity, Or<(
            With<DirectionalLight>,
            With<PointLight>,
            With<SpotLight>,
            With<RectLight>,
        )>>()
        .iter(world)
        .collect();
    for light in lights {
        let has_proxy = world.get::<Children>(light).is_some_and(|c| !c.is_empty());
        if has_proxy {
            continue;
        }
        world.entity_mut(light).insert(ShowLightGizmo::default());
        spawn_light_proxy(world, light);
    }
}

/// A `DirectionalLight`/`SpotLight` shines along its local -Z (rotation), not
/// its position. If the editor gives an (almost) identity rotation, aim it at
/// the scene origin so default lights actually illuminate the cube at the
/// center (a spot at [0,5,0] points straight down at the cube; a directional at
/// [3,8,5] points toward the cube). Otherwise honor the editor's rotation (Flax
/// lights shine along `GetForward()`).
pub(crate) fn light_transform(transform: &Transform) -> Transform {
    if transform.rotation.abs_diff_eq(Quat::IDENTITY, 1e-4) {
        Transform::from_translation(transform.translation).looking_at(Vec3::ZERO, Vec3::Y)
    } else {
        *transform
    }
}

/// `light:ambient` maps to the global `GlobalAmbientLight` resource (single
/// ambient per scene). Only applied when an ambient entity is present; absence
/// leaves the resource untouched (preserves the device.rs default).
pub(crate) fn apply_ambient_light(world: &mut World, def: &SceneEntityDef) {
    let Some(l) = def.light.as_ref() else { return; };
    world.init_resource::<GlobalAmbientLight>();
    let mut ambient = world.resource_mut::<GlobalAmbientLight>();
    if let Some(c) = l.color {
        ambient.color = Color::srgba(c[0], c[1], c[2], c[3]);
    }
    if let Some(b) = l.brightness {
        if b.is_finite() {
            ambient.brightness = b;
        }
    }
}

/// Despawn every `FallbackMarker` entity (called on the first editor
/// `set_scene` so the startup fallback scene is replaced, not doubled).
pub(crate) fn despawn_fallback(world: &mut World) {
    let fallbacks: Vec<Entity> = world
        .query_filtered::<Entity, With<FallbackMarker>>()
        .iter(world)
        .collect();
    for e in fallbacks {
        world.despawn(e);
    }
}