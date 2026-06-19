//! Procedural mesh + material components that round-trip through the native
//! scene file. `Mesh3d`/`MeshMaterial3d` hold runtime asset handles that do not
//! survive serialization across processes, so scene entities carry these
//! data-only components instead, and [`materialize_meshes`] rebuilds the
//! handles each frame from them.

use bevy::prelude::*;

/// Name of a procedural mesh (`"cube"` | `"plane"`).
#[derive(Component, Reflect, Default, Debug, Clone)]
#[reflect(Component)]
pub struct PrimitiveMesh(pub String);

/// Linear RGBA base color of an entity's material.
#[derive(Component, Reflect, Default, Debug, Clone)]
#[reflect(Component)]
pub struct MaterialColor(pub [f32; 4]);

fn mesh_for(name: &str, meshes: &mut Assets<Mesh>) -> Option<Handle<Mesh>> {
    Some(match name {
        "cube" => meshes.add(Cuboid::default()),
        "plane" => meshes.add(Plane3d::default().mesh().size(10.0, 10.0)),
        other => {
            warn!("unknown primitive mesh '{other}'");
            return None;
        }
    })
}

fn color_rgba(rgba: [f32; 4]) -> Color {
    Color::srgba(rgba[0], rgba[1], rgba[2], rgba[3])
}

/// Create the `Mesh3d` + `MeshMaterial3d` handles for entities that declare a
/// primitive mesh but don't yet have a mesh handle.
pub fn materialize_meshes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    q: Query<(Entity, &PrimitiveMesh, Option<&MaterialColor>), Without<Mesh3d>>,
) {
    for (entity, prim, color) in &q {
        let Some(mesh) = mesh_for(&prim.0, &mut meshes) else { continue };
        let rgba = color.map(|c| c.0).unwrap_or([0.8, 0.8, 0.8, 1.0]);
        let material = materials.add(StandardMaterial {
            base_color: color_rgba(rgba),
            ..default()
        });
        commands
            .entity(entity)
            .insert((Mesh3d(mesh), MeshMaterial3d(material)));
    }
}

/// Keep the rendered material's base color in sync with [`MaterialColor`].
pub fn sync_material_colors(
    mut materials: ResMut<Assets<StandardMaterial>>,
    q: Query<(&MaterialColor, &MeshMaterial3d<StandardMaterial>)>,
) {
    for (color, handle) in &q {
        if let Some(mut mat) = materials.get_mut(handle.0.id()) {
            mat.base_color = color_rgba(color.0);
        }
    }
}