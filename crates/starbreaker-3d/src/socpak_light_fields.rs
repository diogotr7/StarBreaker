//! Resolve light component enum semantics without copying game enum ordinals.
use crate::types::LightInfo;
use starbreaker_datacore::{Database, enums::DataType};

pub(crate) fn affect_gi(mode: Option<&str>, affects_objects: bool) -> bool {
    match mode {
        Some("InheritAffectsObjects") => affects_objects,
        Some("On" | "1" | "true" | "True" | "TRUE") => true,
        Some("Off" | "0" | "false" | "False" | "FALSE") | None => false,
        Some(unknown) => {
            log::warn!("Unknown light affectGI mode {unknown:?}; preserving source mode");
            false
        }
    }
}

pub(crate) fn importance_options<'a>(db: &'a Database<'_>) -> Vec<&'a str> {
    let Some(index) = db.struct_index_by_name("LightComponentParams") else {
        return Vec::new();
    };
    let Some(property) = db.all_properties(index).into_iter().find(|p| {
        db.resolve_string2(p.name_offset) == "importance"
            && p.data_type == DataType::EnumChoice as u16
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
            if light.importance.is_none() {
                log::warn!(
                    "Light importance {name:?} absent from DataCore options; preserving source name"
                );
            }
        }
    }
}
