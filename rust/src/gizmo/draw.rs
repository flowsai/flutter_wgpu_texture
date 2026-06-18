//! Immediate-mode gizmo drawing (selection outline + transform handles) into the
//! offscreen camera. Runs in `Update` every frame.

use bevy::camera::primitives::Aabb;
use bevy::prelude::*;

use super::{DragState, GizmoAxis, GizmoHover, GizmoMode};
use crate::picking::EditorSelection;

/// Draw the selection outline + transform gizmo for the selected entity.
///
/// Reads the entity's local `Transform` (which is already up-to-date this frame,
/// e.g. during a gizmo drag) rather than `GlobalTransform` (propagated only in
/// `PostUpdate`, i.e. one frame late) so the outline tracks the mesh with no lag.
/// For root entities (our scene) local == world.
pub(crate) fn draw_editor_gizmos(
    selection: Res<EditorSelection>,
    hover: Option<Res<GizmoHover>>,
    drag: Option<Res<DragState>>,
    transforms: Query<&Transform>,
    aabbs: Query<&Aabb>,
    mut gizmos: Gizmos,
) {
    let Some(entity) = selection.selected else {
        return;
    };
    let Ok(xf) = transforms.get(entity) else {
        return;
    };

    // Selection outline: an orange box around the entity's AABB (or unit cube).
    let outline_color = Color::srgb(1.0, 0.6, 0.0);
    if let Ok(aabb) = aabbs.get(entity) {
        let center = xf.transform_point(Vec3::from(aabb.center));
        let half = Vec3::from(aabb.half_extents) * xf.scale;
        gizmos.cube(
            Transform::from_translation(center)
                .with_rotation(xf.rotation)
                .with_scale(half * 2.0),
            outline_color,
        );
    } else {
        gizmos.cube(*xf, outline_color);
    }

    // Highlight the hovered handle (or the grabbed one while dragging).
    let active_drag = drag.as_ref().filter(|d| d.active).and_then(|d| d.axis);
    let highlight = active_drag.or_else(|| hover.as_ref().and_then(|h| h.axis));

    draw_mode_gizmos(&mut gizmos, xf, selection.mode, highlight);
}

/// Draw translate/rotate/scale handles at the selected entity. `highlight` is
/// the hovered/grabbed axis, drawn in yellow (Unity-style).
fn draw_mode_gizmos(
    gizmos: &mut Gizmos,
    xf: &Transform,
    mode: GizmoMode,
    highlight: Option<GizmoAxis>,
) {
    let p = xf.translation;
    let len = 1.0_f32;
    let yellow = Color::srgb(1.0, 0.95, 0.2);
    let base = |axis: GizmoAxis| -> Color {
        if Some(axis) == highlight {
            yellow
        } else {
            match axis {
                GizmoAxis::X => Color::srgb(1.0, 0.25, 0.25),
                GizmoAxis::Y => Color::srgb(0.25, 1.0, 0.25),
                GizmoAxis::Z => Color::srgb(0.3, 0.5, 1.0),
            }
        }
    };

    match mode {
        GizmoMode::None => {}
        GizmoMode::Translate => {
            for axis in [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
                gizmos.arrow(p, p + axis.dir() * len, base(axis));
            }
        }
        GizmoMode::Rotate => {
            for axis in [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
                let rot = Quat::from_rotation_arc(Vec3::Z, axis.dir());
                gizmos.circle(Isometry3d::new(p, rot), len, base(axis));
            }
        }
        GizmoMode::Scale => {
            for axis in [GizmoAxis::X, GizmoAxis::Y, GizmoAxis::Z] {
                let dir = axis.dir();
                gizmos.line(p, p + dir * len, base(axis));
                gizmos.cube(
                    Transform::from_translation(p + dir * len).with_scale(Vec3::splat(0.12)),
                    base(axis),
                );
            }
        }
    }
}
