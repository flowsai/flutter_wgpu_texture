//! Orbit/pan/zoom/fly camera navigation (Unity-style), driven by Flutter-fed
//! pixel deltas. Headless: there is no winit input; every command is a
//! `RenderCmd` carrying deltas.

use bevy::app::SubApps;
use bevy::asset::AssetId;
use bevy::camera::RenderTarget;
use bevy::ecs::resource::Resource;
use bevy::image::Image;
use bevy::prelude::*;

/// Orbit/pan/zoom camera state for the viewport (Unity-style navigation).
/// `yaw`/`pitch` are spherical angles around `focus` at `distance`.
#[derive(Resource)]
pub(crate) struct OrbitCamera {
    pub(crate) focus: Vec3,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) distance: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        // Frame the default scene from roughly (3,3,6) looking at origin.
        Self {
            focus: Vec3::ZERO,
            yaw: 0.46,   // ~26° around Y
            pitch: 0.42, // ~24° above horizon
            distance: 7.3,
        }
    }
}

impl OrbitCamera {
    /// Camera world position from the spherical orbit parameters.
    fn position(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.focus + Vec3::new(self.distance * cp * sy, self.distance * sp, self.distance * cp * cy)
    }

    pub(crate) fn transform(&self) -> Transform {
        Transform::from_translation(self.position()).looking_at(self.focus, Vec3::Y)
    }
}

const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.05;

/// Find the camera entity rendering to `image`.
pub(crate) fn camera_for_image(world: &mut World, image: AssetId<Image>) -> Option<Entity> {
    let mut q = world.query::<(Entity, &RenderTarget)>();
    q.iter(world)
        .find(|(_, rt)| matches!(rt, RenderTarget::Image(h) if h.handle.id() == image))
        .map(|(e, _)| e)
}

/// Write the orbit camera's transform onto the camera entity.
pub(crate) fn apply_orbit_camera(world: &mut World, camera: Entity) {
    let orbit = world.resource::<OrbitCamera>();
    let new_xf = orbit.transform();
    if let Some(mut t) = world.get_mut::<Transform>(camera) {
        *t = new_xf;
    }
}

pub(crate) fn camera_orbit(sub_apps: &mut SubApps, image: AssetId<Image>, dx: f32, dy: f32) {
    let world = sub_apps.main.world_mut();
    let Some(camera) = camera_for_image(world, image) else {
        return;
    };
    {
        let mut orbit = world.resource_mut::<OrbitCamera>();
        orbit.yaw -= dx * 0.008;
        orbit.pitch = (orbit.pitch + dy * 0.008).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }
    apply_orbit_camera(world, camera);
}

pub(crate) fn camera_pan(sub_apps: &mut SubApps, image: AssetId<Image>, dx: f32, dy: f32) {
    let world = sub_apps.main.world_mut();
    let Some(camera) = camera_for_image(world, image) else {
        return;
    };
    // Pan in the camera's right/up plane; speed scales with distance.
    let (right, up, dist) = {
        let xf = world.get::<Transform>(camera).copied().unwrap_or_default();
        let orbit = world.resource::<OrbitCamera>();
        (xf.right(), xf.up(), orbit.distance)
    };
    let k = dist * 0.0015;
    {
        let mut orbit = world.resource_mut::<OrbitCamera>();
        orbit.focus += (-right * dx + up * dy) * k;
    }
    apply_orbit_camera(world, camera);
}

pub(crate) fn camera_zoom(sub_apps: &mut SubApps, image: AssetId<Image>, delta: f32) {
    let world = sub_apps.main.world_mut();
    let Some(camera) = camera_for_image(world, image) else {
        return;
    };
    {
        let mut orbit = world.resource_mut::<OrbitCamera>();
        // Exponential zoom feels natural; scroll up (negative delta) = zoom in.
        orbit.distance = (orbit.distance * (delta * 0.001).exp()).clamp(0.5, 500.0);
    }
    apply_orbit_camera(world, camera);
}

pub(crate) fn camera_look(sub_apps: &mut SubApps, image: AssetId<Image>, dx: f32, dy: f32) {
    let world = sub_apps.main.world_mut();
    let Some(camera) = camera_for_image(world, image) else {
        return;
    };
    // Free-look rotates the orbit angles but keeps the focus ahead of the camera
    // so subsequent orbit/pan behave intuitively.
    {
        let mut orbit = world.resource_mut::<OrbitCamera>();
        orbit.yaw -= dx * 0.006;
        orbit.pitch = (orbit.pitch + dy * 0.006).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }
    apply_orbit_camera(world, camera);
}

pub(crate) fn camera_fly(
    sub_apps: &mut SubApps,
    image: AssetId<Image>,
    forward: f32,
    right: f32,
    up: f32,
    dt: f32,
) {
    let world = sub_apps.main.world_mut();
    let Some(camera) = camera_for_image(world, image) else {
        return;
    };
    let (fwd, rgt, dist) = {
        let xf = world.get::<Transform>(camera).copied().unwrap_or_default();
        let orbit = world.resource::<OrbitCamera>();
        (xf.forward(), xf.right(), orbit.distance)
    };
    // Move the focus (and thus the camera) along the camera basis. Speed scales
    // with distance so it feels consistent at any zoom level.
    let speed = (dist * 1.5).max(2.0);
    let motion = (fwd * forward + rgt * right + Vec3::Y * up) * speed * dt;
    {
        let mut orbit = world.resource_mut::<OrbitCamera>();
        orbit.focus += motion;
    }
    apply_orbit_camera(world, camera);
}
