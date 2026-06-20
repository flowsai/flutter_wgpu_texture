//! Reflect-driven component spawn from the editor transport.

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::reflect::enums::{DynamicEnum, DynamicVariant};
use bevy::reflect::structs::DynamicStruct;
use bevy::reflect::tuple_struct::DynamicTupleStruct;
use bevy::reflect::{TypeInfo, TypeRegistry};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Stable per-entity id, persisted as a reflected component.
#[derive(Component, Reflect, Default, Debug, Clone)]
#[reflect(Component)]
pub struct SceneObjectId(pub String);

/// Marker for the procedural atmosphere entity.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct SkyAtmosphere;

/// One reflected component on an editor entity. `fields` is a JSON object
/// keyed by field name.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ComponentDef {
    pub type_path: String,
    #[serde(default)]
    pub fields: Map<String, Value>,
}

/// Insert every reflected component in `components` onto `entity`. Unknown
/// types and unsupported kinds are skipped with a warning.
pub fn spawn_components(
    entity: &mut EntityWorldMut,
    components: &[ComponentDef],
    type_registry: &TypeRegistry,
) {
    for comp in components {
        let Some(reg) = type_registry.get_with_type_path(&comp.type_path) else {
            warn!(
                "set_scene: component type '{}' not registered; skipped",
                comp.type_path
            );
            continue;
        };
        let Some(reflect_component) = reg.data::<ReflectComponent>() else {
            warn!(
                "set_scene: type '{}' is not a Component; skipped",
                comp.type_path
            );
            continue;
        };
        let fields_value = Value::Object(comp.fields.clone());
        let Some(dynamic) = json_to_reflect(&fields_value, reg.type_info()) else {
            warn!(
                "set_scene: could not build reflect value for '{}'; skipped",
                comp.type_path
            );
            continue;
        };
        reflect_component.insert(entity, &*dynamic, type_registry);
    }
}

/// Convert a serde_json `Value` into a dynamic reflect value guided by
/// `TypeInfo`. Handles structs, tuple structs and opaque primitives.
fn json_to_reflect(value: &Value, info: &'static TypeInfo) -> Option<Box<dyn PartialReflect>> {
    match info {
        TypeInfo::Struct(s) => {
            let obj = value.as_object()?;
            let mut dyn_struct = DynamicStruct::default();
            dyn_struct.set_represented_type(Some(info));
            for field in s.iter() {
                if let Some(field_value) = obj.get(field.name()) {
                    if let Some(built) = json_to_reflect(field_value, field.type_info()?) {
                        dyn_struct.insert_boxed(field.name(), built);
                    }
                }
            }
            Some(Box::new(dyn_struct))
        }
        TypeInfo::TupleStruct(s) => {
            let arr = value.as_array()?;
            let mut dyn_ts = DynamicTupleStruct::default();
            dyn_ts.set_represented_type(Some(info));
            for i in 0..s.field_len() {
                if let Some(field_value) = arr.get(i) {
                    if let Some(field_info) = s.field_at(i).and_then(|u| u.type_info()) {
                        if let Some(built) = json_to_reflect(field_value, field_info) {
                            dyn_ts.insert_boxed(built);
                        }
                    }
                }
            }
            Some(Box::new(dyn_ts))
        }
        TypeInfo::Enum(e) => {
            // Unit variants are authored as the variant name string.
            let name = value.as_str()?;
            if !e.iter().any(|v| v.name() == name) {
                return None;
            }
            let mut dyn_enum = DynamicEnum::default();
            dyn_enum.set_represented_type(Some(info));
            dyn_enum.set_variant(name, DynamicVariant::Unit);
            Some(Box::new(dyn_enum))
        }
        TypeInfo::Opaque(_) => json_to_opaque(value, info.type_path()),
        _ => None,
    }
}

fn json_to_opaque(value: &Value, type_path: &str) -> Option<Box<dyn PartialReflect>> {
    match type_path {
        "f32" => value.as_f64().map(|v| Box::new(v as f32) as Box<dyn PartialReflect>),
        "f64" => value.as_f64().map(|v| Box::new(v) as Box<dyn PartialReflect>),
        "i32" => value.as_i64().map(|v| Box::new(v as i32) as Box<dyn PartialReflect>),
        "u32" => value
            .as_i64()
            .map(|v| Box::new(v as u32) as Box<dyn PartialReflect>),
        "bool" => value.as_bool().map(|v| Box::new(v) as Box<dyn PartialReflect>),
        "alloc::string::String" | "String" => value
            .as_str()
            .map(|v| Box::new(v.to_string()) as Box<dyn PartialReflect>),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::reflect::AppTypeRegistry;

    #[derive(Component, Reflect, Default, PartialEq, Debug)]
    #[reflect(Component)]
    struct Foo {
        x: f32,
        name: String,
    }

    #[test]
    fn spawns_component_with_unit_enum_field() {
        use crate::level::physics::{BodyType, ColliderShape, RigidBodyDef};

        let mut world = World::new();
        let atr = AppTypeRegistry::default();
        atr.write().register::<RigidBodyDef>();
        world.insert_resource(atr);
        let type_registry = world.resource::<AppTypeRegistry>().clone();
        let guard = type_registry.read();

        let mut fields = Map::new();
        fields.insert("body".to_string(), Value::from("Static"));
        fields.insert("collider".to_string(), Value::from("HalfSpace"));
        let comp = ComponentDef {
            type_path: std::any::type_name::<RigidBodyDef>().to_string(),
            fields,
        };

        let mut entity = world.spawn(());
        spawn_components(&mut entity, &[comp], &guard);
        let got = entity.get::<RigidBodyDef>().expect("RigidBodyDef not inserted");
        assert_eq!(got.body, BodyType::Static);
        assert_eq!(got.collider, ColliderShape::HalfSpace);
    }

    #[test]
    fn spawns_reflect_component_from_json() {
        let mut world = World::new();
        let atr = AppTypeRegistry::default();
        atr.write().register::<Foo>();
        world.insert_resource(atr);
        let type_registry = world.resource::<AppTypeRegistry>().clone();
        let guard = type_registry.read();

        let mut fields = Map::new();
        fields.insert("x".to_string(), Value::from(7.5));
        fields.insert("name".to_string(), Value::from("hi"));
        let comp = ComponentDef {
            type_path: std::any::type_name::<Foo>().to_string(),
            fields,
        };

        let mut entity = world.spawn(());
        spawn_components(&mut entity, &[comp], &guard);
        let got = entity.get::<Foo>().expect("Foo not inserted");
        assert_eq!(got.x, 7.5);
        assert_eq!(got.name, "hi");
    }

    #[test]
    fn skips_unknown_type() {
        let mut world = World::new();
        let atr = AppTypeRegistry::default();
        world.insert_resource(atr);
        let type_registry = world.resource::<AppTypeRegistry>().clone();
        let guard = type_registry.read();

        let mut entity = world.spawn(());
        spawn_components(
            &mut entity,
            &[ComponentDef {
                type_path: "nope::Missing".to_string(),
                fields: Map::new(),
            }],
            &guard,
        );
        assert!(entity.get::<SceneObjectId>().is_none());
    }
}