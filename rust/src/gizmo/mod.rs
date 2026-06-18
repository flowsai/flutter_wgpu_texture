//! Transform gizmo (≈ Flax `Source/Editor/Gizmo/`). The drag math lives here on
//! the engine side because it operates directly on the ECS world; the editor
//! only routes input + holds the active mode.

pub mod draw;
pub mod drag;
pub mod hit;

use bevy::ecs::resource::Resource;
use bevy::prelude::*;

/// Transform gizmo interaction mode (mirrors the Dart `GizmoMode`).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GizmoMode {
    #[default]
    None,
    Translate,
    Rotate,
    Scale,
}

impl GizmoMode {
    pub(crate) fn from_str(s: &str) -> Self {
        match s {
            "translate" => GizmoMode::Translate,
            "rotate" => GizmoMode::Rotate,
            "scale" => GizmoMode::Scale,
            _ => GizmoMode::None,
        }
    }
}

/// Which gizmo axis is grabbed during a drag.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GizmoAxis {
    X,
    Y,
    Z,
}

impl GizmoAxis {
    pub(crate) fn dir(self) -> Vec3 {
        match self {
            GizmoAxis::X => Vec3::X,
            GizmoAxis::Y => Vec3::Y,
            GizmoAxis::Z => Vec3::Z,
        }
    }
}

/// Which gizmo handle the cursor is hovering (for Unity-style highlight).
#[derive(Resource, Default)]
pub(crate) struct GizmoHover {
    pub(crate) axis: Option<GizmoAxis>,
}

/// In-progress gizmo drag state (set on drag_begin, cleared on drag_end).
#[derive(Resource, Default)]
pub(crate) struct DragState {
    pub(crate) active: bool,
    pub(crate) axis: Option<GizmoAxis>,
    pub(crate) start_cursor: Vec2,
    pub(crate) start_transform: Transform,
}
