use bevy::pbr::wireframe::WireframeConfig;
use bevy::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum ViewMode {
    #[default]
    Lit,
    Unlit,
    Wireframe,
}

impl ViewMode {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "Lit" => Some(Self::Lit),
            "Unlit" => Some(Self::Unlit),
            "Wireframe" => Some(Self::Wireframe),
            _ => None,
        }
    }
}

/// Apply the requested view mode to the live world:
/// - Lit: PBR shading, no wireframe, all materials restored to authored unlit flag.
/// - Unlit: force every `StandardMaterial` to `unlit = true`.
/// - Wireframe: Lit shading + `WireframeConfig::global = true`.
pub(crate) fn set_view_mode(world: &mut World, mode: &str) {
    let Some(vm) = ViewMode::from_str(mode) else {
        warn!("set_view_mode: unknown mode '{mode}'");
        return;
    };

    // Toggle global wireframe overlay.
    if let Some(mut cfg) = world.get_resource_mut::<WireframeConfig>() {
        cfg.global = vm == ViewMode::Wireframe;
    }

    // Toggle unlit on every material.
    let unlit = vm == ViewMode::Unlit;
    if let Some(mut mats) = world.get_resource_mut::<Assets<StandardMaterial>>() {
        for (_, mat) in mats.iter_mut() {
            mat.unlit = unlit;
        }
    }
}
