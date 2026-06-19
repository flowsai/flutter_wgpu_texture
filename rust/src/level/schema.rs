//! Scene description deserialized from the editor (Dart `EditorState.toSceneJson`).
//!
//! The editor owns the scene tree; it pushes the whole thing as JSON via
//! `set_scene(handle, json)`. The render thread diffs it against the live ECS
//! world (see `level::rebuild_scene`).

use bevy::prelude::{Quat, Transform};
use serde::Deserialize;

use super::components::ComponentDef;

#[derive(Debug, Deserialize)]
pub struct SceneDoc {
    pub entities: Vec<SceneEntityDef>,
}

#[derive(Debug, Deserialize)]
pub struct SceneEntityDef {
    /// Stable editor id — the join key with the `SceneObjectId` component.
    pub id: String,
    #[allow(dead_code)]
    pub name: String,
    /// `mesh:cube` | `mesh:plane` |
    /// `light:directional` | `light:point` | `light:spot` | `light:rect` |
    /// `light:ambient` (cameras are viewport-owned; `light:ambient` maps to the
    /// global ambient resource, not a world entity).
    pub kind: String,
    /// Parent entity id within this scene. None = root. Wired to `ChildOf` so
    /// the child transform is relative to its parent.
    #[serde(default)]
    pub parent_id: Option<String>,
    pub transform: TransformDef,
    #[serde(default)]
    pub material: Option<MaterialDef>,
    #[serde(default)]
    pub light: Option<LightDef>,
    /// Reflected add-on components, inserted via the `AppTypeRegistry`.
    #[serde(default)]
    pub components: Vec<ComponentDef>,
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

/// Flattened union of all Bevy light fields: a single `brightness` multiplier
/// + `color` per light, plus per-type shape fields. The entity's `kind`
/// discriminates which component to spawn; fields not relevant to a kind are
/// ignored. `brightness` maps to lux (directional) or lumens (point/spot/rect)
/// in `light::spawn_light`, and to `GlobalAmbientLight.brightness` for ambient.
#[derive(Debug, Deserialize, Default)]
pub struct LightDef {
    /// Linear RGBA in 0..1. Defaults to white when `None`.
    #[serde(default)]
    pub color: Option<[f32; 4]>,

    /// Brightness multiplier (default 3.14). Mapped to Bevy units by
    /// `light::spawn_light` / `apply_ambient_light`.
    #[serde(default)]
    pub brightness: Option<f32>,

    // point / spot / rect shape
    #[serde(default)]
    pub range: Option<f32>,
    #[serde(default)]
    pub radius: Option<f32>,

    // spot (radians)
    #[serde(default)]
    pub inner_angle: Option<f32>,
    #[serde(default)]
    pub outer_angle: Option<f32>,

    // rect
    #[serde(default)]
    pub width: Option<f32>,
    #[serde(default)]
    pub height: Option<f32>,

    // shared
    #[serde(default)]
    pub shadow_maps_enabled: Option<bool>,
}