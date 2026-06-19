//! Reflect type-registry introspection over FFI: the list of registered
//! component types and the field-level description of a single component.

use bevy::ecs::reflect::{ReflectComponent, ReflectResource};
use bevy::prelude::*;
use bevy::reflect::{Reflect, ReflectKind, ReflectRef, TypeInfo, TypeRegistry};

use serde_json::{json, Map, Value};

/// JSON: `[ { "type_path": "...", "short_name": "..." }, ... ]`.
pub fn list_component_types(type_registry: &TypeRegistry) -> String {
    let mut out: Vec<Value> = Vec::new();
    for reg in type_registry.iter() {
        if reg.data::<ReflectComponent>().is_none() {
            continue;
        }
        // Skip resources (even if they also carry ReflectComponent) and generic
        // types like `Time<()>`, `Handle<Mesh>`, `Vec<...>`: not addable to an
        // entity from the editor.
        if reg.data::<ReflectResource>().is_some() {
            continue;
        }
        let type_path = reg.type_info().type_path();
        if type_path.contains('<') || type_path.contains('>') {
            continue;
        }
        out.push(json!({
            "type_path": type_path,
            "short_name": short_name(type_path),
        }));
    }
    Value::Array(out).to_string()
}

/// JSON for one component type, or `None` if `type_path` is not registered.
///
/// ```json
/// {
///   "type_path": "...", "short_name": "...", "kind": "struct",
///   "fields": [ { "name": "translation", "type_path": "...", "kind": "...", "default": null } ]
/// }
/// ```
pub fn describe_component(type_registry: &TypeRegistry, type_path: &str) -> Option<String> {
    let reg = type_registry.get_with_type_path(type_path)?;
    let info = reg.type_info();
    let default_instance = reg
        .data::<ReflectDefault>()
        .map(|d| d.default() as Box<dyn Reflect>);

    let fields = fields(info, default_instance.as_deref());
    Some(
        json!({
            "type_path": info.type_path(),
            "short_name": short_name(info.type_path()),
            "kind": kind_str(info_kind(info)),
            "fields": fields,
        })
        .to_string(),
    )
}

fn fields(info: &TypeInfo, default: Option<&dyn Reflect>) -> Vec<Value> {
    let default_ref = default.map(|d| {
        let p: &dyn PartialReflect = d;
        p.reflect_ref()
    });

    match info {
        TypeInfo::Struct(s) => s
            .iter()
            .map(|f| {
                let def = match &default_ref {
                    Some(ReflectRef::Struct(ds)) => ds.field(f.name()),
                    _ => None,
                };
                field_entry(f.name(), f.type_info(), def)
            })
            .collect(),
        TypeInfo::TupleStruct(s) => (0..s.field_len())
            .map(|i| {
                let def = match &default_ref {
                    Some(ReflectRef::TupleStruct(ds)) => ds.field(i),
                    _ => None,
                };
                field_entry(
                    &i.to_string(),
                    s.field_at(i).and_then(|u| u.type_info()),
                    def,
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn field_entry(name: &str, field_info: Option<&'static TypeInfo>, default: Option<&dyn PartialReflect>) -> Value {
    let (type_path, kind) = match field_info {
        Some(info) => (info.type_path(), kind_str(info_kind(info))),
        None => ("", "unknown"),
    };
    json!({
        "name": name,
        "type_path": type_path,
        "kind": kind,
        "default": default.map(reflect_value_to_json).unwrap_or(Value::Null),
    })
}

fn reflect_value_to_json(v: &dyn PartialReflect) -> Value {
    match v.reflect_ref() {
        ReflectRef::Opaque(op) => {
            let any = match op.try_as_reflect() {
                Some(r) => r.as_any(),
                None => return Value::Null,
            };
            if let Some(x) = any.downcast_ref::<f32>() {
                json!(*x as f64)
            } else if let Some(x) = any.downcast_ref::<f64>() {
                json!(*x)
            } else if let Some(x) = any.downcast_ref::<i32>() {
                json!(*x)
            } else if let Some(x) = any.downcast_ref::<u32>() {
                json!(*x)
            } else if let Some(x) = any.downcast_ref::<bool>() {
                json!(*x)
            } else if let Some(x) = any.downcast_ref::<String>() {
                json!(x)
            } else {
                Value::Null
            }
        }
        ReflectRef::TupleStruct(ts) => {
            let mut arr = Vec::new();
            for i in 0..ts.field_len() {
                arr.push(match ts.field(i) {
                    Some(f) => reflect_value_to_json(f),
                    None => Value::Null,
                });
            }
            Value::Array(arr)
        }
        ReflectRef::List(l) => {
            let mut arr = Vec::new();
            for i in 0..l.len() {
                arr.push(match l.get(i) {
                    Some(f) => reflect_value_to_json(f),
                    None => Value::Null,
                });
            }
            Value::Array(arr)
        }
        ReflectRef::Struct(s) => {
            let mut m = Map::new();
            for i in 0..s.field_len() {
                if let Some(f) = s.field_at(i) {
                    m.insert(format!("field_{i}"), reflect_value_to_json(f));
                }
            }
            Value::Object(m)
        }
        _ => Value::Null,
    }
}

fn short_name(type_path: &str) -> &str {
    type_path.rsplit("::").next().unwrap_or(type_path)
}

fn info_kind(info: &TypeInfo) -> ReflectKind {
    match info {
        TypeInfo::Struct(_) => ReflectKind::Struct,
        TypeInfo::TupleStruct(_) => ReflectKind::TupleStruct,
        TypeInfo::Tuple(_) => ReflectKind::Tuple,
        TypeInfo::List(_) => ReflectKind::List,
        TypeInfo::Array(_) => ReflectKind::Array,
        TypeInfo::Map(_) => ReflectKind::Map,
        TypeInfo::Set(_) => ReflectKind::Set,
        TypeInfo::Enum(_) => ReflectKind::Enum,
        TypeInfo::Opaque(_) => ReflectKind::Opaque,
    }
}

fn kind_str(k: ReflectKind) -> &'static str {
    match k {
        ReflectKind::Struct => "struct",
        ReflectKind::TupleStruct => "tuple_struct",
        ReflectKind::Tuple => "tuple",
        ReflectKind::List => "list",
        ReflectKind::Array => "array",
        ReflectKind::Map => "map",
        ReflectKind::Set => "set",
        ReflectKind::Enum => "enum",
        ReflectKind::Opaque => "opaque",
        #[allow(unreachable_patterns)]
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::reflect::TypeRegistry;

    #[derive(Component, Reflect, Default)]
    #[reflect(Component, Default)]
    struct TestComp {
        x: f32,
        name: String,
    }

    fn registry() -> TypeRegistry {
        let mut r = TypeRegistry::new();
        r.register::<TestComp>();
        r
    }

    #[test]
    fn lists_registered_component() {
        let json = list_component_types(&registry());
        let v: Value = serde_json::from_str(&json).unwrap();
        let arr = v.as_array().unwrap();
        let entry = arr
            .iter()
            .find(|e| e["short_name"] == "TestComp")
            .expect("TestComp listed");
        assert!(entry["type_path"].as_str().unwrap().ends_with("TestComp"));
    }

    #[test]
    fn describes_struct_fields_and_defaults() {
        let path = std::any::type_name::<TestComp>();
        let json = describe_component(&registry(), path).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["kind"], "struct");
        assert_eq!(v["short_name"], "TestComp");
        let fields = v["fields"].as_array().unwrap();
        let x = fields.iter().find(|f| f["name"] == "x").unwrap();
        assert_eq!(x["default"], 0.0);
        let name = fields.iter().find(|f| f["name"] == "name").unwrap();
        assert!(name["default"].is_string());
    }

    #[test]
    fn describe_unknown_returns_none() {
        assert!(describe_component(&registry(), "nope::Missing").is_none());
    }
}