//! Draggable transform gizmos: begin/update/hover one-shot systems and the
//! per-mode transform math (translate/rotate/scale), constrained to the grabbed
//! axis. Sign conventions follow the `transform-gizmo` crate.

use bevy::app::SubApps;
use bevy::asset::AssetId;
use bevy::camera::{Camera, RenderTarget};
use bevy::ecs::system::{In, RunSystemOnce};
use bevy::image::Image;
use bevy::prelude::*;

use super::hit::{gizmo_axis_at, ray_axis_param, GIZMO_LEN};
use super::{DragState, GizmoAxis, GizmoHover, GizmoMode};
use crate::engine::render_thread::TransformOut;
use crate::picking::EditorSelection;

pub(crate) fn drag_begin(sub_apps: &mut SubApps, image: AssetId<Image>, cursor: Vec2) -> bool {
    let world = sub_apps.main.world_mut();
    world.init_resource::<DragState>();
    world
        .run_system_once_with(drag_begin_system, (image, cursor))
        .unwrap_or(false)
}

pub(crate) fn set_hover(sub_apps: &mut SubApps, image: AssetId<Image>, cursor: Vec2) {
    let world = sub_apps.main.world_mut();
    world.init_resource::<GizmoHover>();
    let _ = world.run_system_once_with(hover_system, (image, cursor));
}

fn hover_system(
    input: In<(AssetId<Image>, Vec2)>,
    cameras: Query<(&Camera, &GlobalTransform, &RenderTarget)>,
    transforms: Query<&GlobalTransform>,
    selection: Res<EditorSelection>,
    drag: Res<DragState>,
    mut hover: ResMut<GizmoHover>,
) {
    // Don't change the highlight mid-drag (the grabbed axis stays highlighted).
    if drag.active {
        return;
    }
    let (image, cursor) = input.0;
    let axis = (|| {
        let entity = selection.selected?;
        let obj_xf = transforms.get(entity).ok()?;
        let (cam, cam_xf, _) = cameras
            .iter()
            .find(|(_, _, rt)| matches!(rt, RenderTarget::Image(h) if h.handle.id() == image))?;
        gizmo_axis_at(cursor, obj_xf.translation(), selection.mode, cam, cam_xf)
    })();
    hover.axis = axis;
}

fn drag_begin_system(
    input: In<(AssetId<Image>, Vec2)>,
    cameras: Query<(&Camera, &GlobalTransform, &RenderTarget)>,
    transforms: Query<&GlobalTransform>,
    selection: Res<EditorSelection>,
    mut drag: ResMut<DragState>,
) -> bool {
    let (image, cursor) = input.0;
    let Some(entity) = selection.selected else {
        return false;
    };
    if selection.mode == GizmoMode::None {
        return false;
    }
    let Ok(obj_xf) = transforms.get(entity) else {
        return false;
    };
    let Some((cam, cam_xf, _)) = cameras
        .iter()
        .find(|(_, _, rt)| matches!(rt, RenderTarget::Image(h) if h.handle.id() == image))
    else {
        return false;
    };

    let Some(axis) = gizmo_axis_at(cursor, obj_xf.translation(), selection.mode, cam, cam_xf)
    else {
        return false;
    };

    drag.active = true;
    drag.axis = Some(axis);
    drag.start_cursor = cursor;
    drag.start_transform = obj_xf.compute_transform();
    true
}

pub(crate) fn drag_update(
    sub_apps: &mut SubApps,
    image: AssetId<Image>,
    cursor: Vec2,
) -> Option<TransformOut> {
    let world = sub_apps.main.world_mut();
    world
        .run_system_once_with(drag_update_system, (image, cursor))
        .ok()
        .flatten()
}

fn drag_update_system(
    input: In<(AssetId<Image>, Vec2)>,
    cameras: Query<(&Camera, &GlobalTransform, &RenderTarget)>,
    mut transforms: Query<&mut Transform>,
    selection: Res<EditorSelection>,
    drag: Res<DragState>,
) -> Option<TransformOut> {
    if !drag.active {
        return None;
    }
    let (image, cursor) = input.0;
    let entity = selection.selected?;
    let axis = drag.axis?;
    let (cam, cam_xf, _) = cameras
        .iter()
        .find(|(_, _, rt)| matches!(rt, RenderTarget::Image(h) if h.handle.id() == image))?;

    let axis_dir = axis.dir();
    let start = &drag.start_transform;
    let mut new_t = *start;

    // Screen-space direction of the +axis at the object origin (pixels).
    // Used to map intuitive cursor motion to signed amounts along the axis.
    let origin2d = cam.world_to_viewport(cam_xf, start.translation).ok()?;
    let axis_end2d = cam
        .world_to_viewport(cam_xf, start.translation + axis_dir * GIZMO_LEN)
        .ok()?;
    let axis2d = (axis_end2d - origin2d).normalize_or_zero();
    let cursor_delta = cursor - drag.start_cursor;

    // Signed world distance the object should move along its axis: intersect the
    // cursor ray with the plane through the object containing the axis and most
    // facing the camera, then take the component along the axis. This is
    // geometrically exact and sign-correct for every axis and camera angle.
    let along_axis = {
        let ray0 = cam.viewport_to_world(cam_xf, drag.start_cursor);
        let ray1 = cam.viewport_to_world(cam_xf, cursor);
        match (ray0, ray1) {
            (Ok(r0), Ok(r1)) => {
                let p0 = ray_axis_param(r0, start.translation, axis_dir);
                let p1 = ray_axis_param(r1, start.translation, axis_dir);
                match (p0, p1) {
                    (Some(a), Some(b)) => b - a,
                    _ => cursor_delta.dot(axis2d) / (axis_end2d - origin2d).length().max(1.0),
                }
            }
            _ => cursor_delta.dot(axis2d) / (axis_end2d - origin2d).length().max(1.0),
        }
    };

    match selection.mode {
        GizmoMode::Translate => {
            new_t.translation = start.translation + axis_dir * along_axis;
        }
        GizmoMode::Rotate => {
            // Angle of cursor around the projected gizmo center (screen space),
            // following the `transform-gizmo` crate's convention:
            //   angle = atan2(now) - atan2(start)
            //   flip when the camera views the axis from behind (forward·axis < 0)
            //   delta_quat = from_axis_angle(world_axis, angle); new = delta * start
            let a0 = drag.start_cursor - origin2d;
            let a1 = cursor - origin2d;
            if a0.length_squared() < 4.0 || a1.length_squared() < 4.0 {
                return None;
            }
            let mut angle = a1.y.atan2(a1.x) - a0.y.atan2(a0.x);
            if cam_xf.forward().dot(axis_dir) < 0.0 {
                angle = -angle;
            }
            new_t.rotation = (Quat::from_axis_angle(axis_dir, angle) * start.rotation).normalize();
        }
        GizmoMode::Scale => {
            // Same signed on-screen amount as translate → grow when dragging the
            // handle the way it points, shrink the other way (per-axis correct).
            let factor = (1.0 + along_axis).max(0.05);
            let mut scale = start.scale;
            match axis {
                GizmoAxis::X => scale.x *= factor,
                GizmoAxis::Y => scale.y *= factor,
                GizmoAxis::Z => scale.z *= factor,
            }
            new_t.scale = scale;
        }
        GizmoMode::None => return None,
    }

    let mut t = transforms.get_mut(entity).ok()?;
    *t = new_t;
    Some(TransformOut::from_transform(&new_t))
}
