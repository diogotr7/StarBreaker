//! Resolve light component enum semantics without copying game enum ordinals.
use crate::types::LightInfo;
use starbreaker_datacore::{Database, enums::DataType};

pub(crate) fn affect_gi(mode: Option<&str>, affects_objects: Option<bool>) -> Option<bool> {
    match mode {
        Some("InheritAffectsObjects") => affects_objects,
        Some("On" | "1" | "true" | "True" | "TRUE") => Some(true),
        Some("Off" | "0" | "false" | "False" | "FALSE") => Some(false),
        _ => None,
    }
}

pub(crate) fn importance_options<'a>(db: &'a Database<'_>) -> Vec<&'a str> {
    let options = enum_options(db, "importance");
    if options.is_empty() {
        log::warn!(
            "Light importance enum unavailable in DataCore; preserving names with null ordinals"
        );
    }
    options
}

pub(crate) fn enum_options<'a>(db: &'a Database<'_>, name: &str) -> Vec<&'a str> {
    let Some(index) = db.struct_index_by_name("LightComponentParams") else {
        return Vec::new();
    };
    let Some(property) = db.all_properties(index).into_iter().find(|p| {
        db.resolve_string2(p.name_offset) == name && p.data_type == DataType::EnumChoice as u16
    }) else {
        return Vec::new();
    };
    db.enum_options(i32::from(property.struct_index))
        .iter()
        .map(|id| db.resolve_string2(*id))
        .collect()
}

pub(crate) fn resolve_importance(light: &mut LightInfo, options: &[&str]) {
    if light.importance.is_none() {
        if let Some(name) = &light.importance_name {
            light.importance = options
                .iter()
                .position(|option| option == name)
                .and_then(|index| i32::try_from(index).ok());
        }
    }
}
