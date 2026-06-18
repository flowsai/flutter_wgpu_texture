//! Scene description deserialized from the editor (Dart `EditorState.toSceneJson`).
//!
//! The editor owns the scene tree; it pushes the whole thing as JSON via
//! `set_scene(handle, json)`. The render thread diffs it against the live ECS
//! world (see `level::rebuild_scene`).

use bevy::prelude::{Quat, Transform};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SceneDoc {
    pub entities: Vec<SceneEntityDef>,
}

#[derive(Debug, Deserialize)]
pub struct SceneEntityDef {
    /// Stable editor id — the join key between Dart `SceneEntity.id` and Bevy `Entity`.
    pub id: String,
    #[allow(dead_code)]
    pub name: String,
    /// `mesh:cube` | `mesh:plane` | `light:directional` (cameras are viewport-owned).
    pub kind: String,
    pub transform: TransformDef,
    #[serde(default)]
    pub material: Option<MaterialDef>,
    #[serde(default)]
    pub light: Option<LightDef>,
}

#[derive(Debug, Deserialize)]
pub struct TransformDef {
    pub translation: [f32; 3],
    /// Quaternion, xyzw order.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl TransformDef {
    pub fn to_bevy(&self) -> Transform {
        Transform {
            translation: self.translation.into(),
            rotation: Quat::from_array(self.rotation),
            scale: self.scale.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct MaterialDef {
    /// Linear RGBA in 0..1.
    pub color: [f32; 4],
}

#[derive(Debug, Deserialize)]
pub struct LightDef {
    pub illuminance: f32,
}
