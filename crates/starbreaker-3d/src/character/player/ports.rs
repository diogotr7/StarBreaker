//! DataCore item ports, attachment helpers and authored item metadata.
use super::xml::{Result, error};
use serde_json::{Value, json};
use starbreaker_datacore::{Database, types::Record};

pub(super) fn words(s: &str) -> Vec<String> {
    s.split(|c: char| c == '+' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
pub(super) fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
pub(super) fn subset(need: &[String], have: &[String]) -> bool {
    need.iter().all(|n| have.iter().any(|h| n.eq_ignore_ascii_case(h)))
}

/// Preserve source field names/types while following DataCore pointers/references.
pub(super) fn query(db: &Database, record: &Record, path: &str) -> Result<Value> {
    use starbreaker_datacore::query::value::Value as D;
    fn owned(v: &D) -> Value {
        match v {
            D::Null => Value::Null,
            D::Bool(v) => json!(v),
            D::String(s) | D::Enum(s) | D::Locale(s) => json!(s),
            D::Int8(v) => json!(v),
            D::Int16(v) => json!(v),
            D::Int32(v) => json!(v),
            D::Int64(v) => json!(v),
            D::UInt8(v) => json!(v),
            D::UInt16(v) => json!(v),
            D::UInt32(v) => json!(v),
            D::UInt64(v) => json!(v),
            D::Float(v) => json!(v),
            D::Double(v) => json!(v),
            D::Guid(v) => json!(v.to_string()),
            D::Array(a) => Value::Array(a.iter().map(owned).collect()),
            D::Object { type_name, fields, .. } => {
                let mut m = serde_json::Map::new();
                m.insert("_Type_".into(), json!(type_name));
                for (k, v) in fields {
                    m.insert((*k).into(), owned(v));
                }
                Value::Object(m)
            }
        }
    }
    let path = db
        .compile_path::<D>(record.struct_id(), path)
        .map_err(|e| error(e.to_string()))?;
    let values = db
        .query_no_references(&path, record)
        .map_err(|e| error(e.to_string()))?;
    Ok(if values.len() == 1 {
        owned(&values[0])
    } else {
        Value::Array(values.iter().map(owned).collect())
    })
}

fn helper<'a>(node: &'a Value, tags: &[String]) -> &'a Value {
    if let Some(children) = node["SubHelpers"].as_array() {
        for child in children {
            if subset(&words(text(&child["Tags"])), tags) {
                return helper(child, tags);
            }
        }
    }
    &node["Helper"]
}
fn vector(v: &Value) -> Value {
    json!([
        v["x"].as_f64().unwrap_or(0.0),
        v["y"].as_f64().unwrap_or(0.0),
        v["z"].as_f64().unwrap_or(0.0)
    ])
}

pub(super) fn parse_port(port: &Value, owner: &str, tags: &[String]) -> Option<Value> {
    let implementation = &port["AttachmentImplementation"];
    let kind = match text(&implementation["_Type_"]) {
        "SItemPortDefAttachmentImplementationBone" => "bone",
        "SItemPortDefAttachmentImplementationSkin" => "skin",
        _ => return None,
    };
    let h = helper(&implementation["Helper"], tags);
    Some(
        json!({"name":port["Name"],"owner_entity":owner,"flags":words(text(&port["Flags"])),
        "types":port["Types"],"attachment":kind,"attach_bone":h["Name"],
        "offset":{"position":vector(&h["Offset"]["Position"]),"rotation_degrees":vector(&h["Offset"]["Rotation"])},
        "item_offset_helper":h["ItemOffsetHelperName"],"select_tag":port["Extension"]["SelectTag"]}),
    )
}
pub(super) fn ports(db: &Database, record: &Record, owner: &str, tags: &[String]) -> Result<Vec<Value>> {
    let value = query(db, record, "Components[SItemPortContainerComponentParams].Ports")?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| parse_port(p, owner, tags))
        .collect())
}
pub(super) fn attach(db: &Database, record: &Record) -> Result<Value> {
    query(db, record, "Components[SAttachableComponentParams].AttachDef")
}

/// Variant labels come from TagDefs/helper names. Match the longest supported
/// gender suffix (Female must not also match Male).
pub(super) fn gender_variant(label: &str, gender: &str) -> bool {
    use super::super::Gender;
    [Gender::Female, Gender::Male]
        .into_iter()
        .find(|g| label.to_ascii_lowercase().ends_with(&g.tag().to_ascii_lowercase()))
        .is_some_and(|g| g.tag().eq_ignore_ascii_case(gender))
}
