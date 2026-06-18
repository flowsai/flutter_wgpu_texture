//! Screen-space gizmo handle hit-testing + ray/axis math.

use bevy::camera::Camera;
use bevy::prelude::*;

use super::{GizmoAxis, GizmoMode};

pub(crate) const GIZMO_LEN: f32 = 1.0;
/// Pixel radius for grabbing a gizmo handle. Generous so handles are easy to hit.
pub(crate) const HANDLE_PIXEL_THRESHOLD: f32 = 18.0;

/// 2D distance from point `p` to segment `a`–`b`.
pub(crate) fn point_segment_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < 1e-6 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Signed distance along the axis line `origin + t*dir` of the point closest to
/// the cursor `ray` (the `tb` from a normalized ray-to-ray closest-point solve,
/// matching the `transform-gizmo` crate). `dir` must be unit length.
/// `None` if the ray is ~parallel to the axis.
pub(crate) fn ray_axis_param(ray: Ray3d, origin: Vec3, dir: Vec3) -> Option<f32> {
    let adir = *ray.direction; // unit
    let bdir = dir; // unit
    let b = adir.dot(bdir);
    let w = ray.origin - origin;
    let d = adir.dot(w);
    let e = bdir.dot(w);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    // tb: parameter along the axis (bdir) of its closest point to the ray.
    Some((e - b * d) / denom)
}

/// Which gizmo axis handle (if any) the cursor is over, for the given mode.
/// Translate/scale test the axis segment; rotate tests the projected ring.
pub(crate) fn gizmo_axis_at(
    cursor: Vec2,
    origin: Vec3,
    mode: GizmoMode,
    cam: &Camera,
    cam_xf: &GlobalTransform,
) -> Option<GizmoAxis> {
    if mode == GizmoMode::None {
        return None;
    }
    let to_screen = |w: Vec3| cam.world_to_viewport(cam_xf, w).ok();
    let mut best: Option<(GizmoAxis, f32)> = None;
    for axis in [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
        let dist = match mode {
            GizmoMode::Rotate => {
                circle_screen_dist(cursor, origin, axis.dir(), GIZMO_LEN, &to_screen)
            }
            _ => {
                let (Some(a), Some(b)) =
                    (to_screen(origin), to_screen(origin + axis.dir() * GIZMO_LEN))
                else {
                    continue;
                };
                point_segment_dist(cursor, a, b)
            }
        };
        if dist < HANDLE_PIXEL_THRESHOLD && best.map_or(true, |(_, bd)| dist < bd) {
            best = Some((axis, dist));
        }
    }
    best.map(|(axis, _)| axis)
}

/// Minimum screen-space distance from `cursor` to a world circle of `radius`
/// centered at `center` with normal `axis`, by sampling the circle.
pub(crate) fn circle_screen_dist(
    cursor: Vec2,
    center: Vec3,
    axis: Vec3,
    radius: f32,
    to_screen: &impl Fn(Vec3) -> Option<Vec2>,
) -> f32 {
    let rot = Quat::from_rotation_arc(Vec3::Z, axis.normalize());
    const SAMPLES: usize = 48;
    let mut prev: Option<Vec2> = None;
    let mut min = f32::INFINITY;
    let mut first: Option<Vec2> = None;
    for i in 0..SAMPLES {
        let t = i as f32 / SAMPLES as f32 * std::f32::consts::TAU;
        let local = Vec3::new(t.cos() * radius, t.sin() * radius, 0.0);
        let world = center + rot * local;
        let Some(p) = to_screen(world) else {
            prev = None;
            continue;
        };
        if first.is_none() {
            first = Some(p);
        }
        if let Some(a) = prev {
            min = min.min(point_segment_dist(cursor, a, p));
        }
        prev = Some(p);
    }
    // Close the loop.
    if let (Some(a), Some(b)) = (prev, first) {
        min = min.min(point_segment_dist(cursor, a, b));
    }
    min
}
