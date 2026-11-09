//! Actor motion-base inference and authored speed-query enumeration.
use super::ports::{subset, text};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) type Schemas = BTreeMap<String, BTreeMap<String, Vec<String>>>;
fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

/// A weapon association in one motion fragment establishes the actor's base
/// for every participating fragment. Stow selection remains independent.
pub(super) fn bases(
    fragments: &[Value],
    schemas: &Schemas,
    tags: &[String],
    weapon_tags: &[String],
) -> BTreeMap<String, Option<String>> {
    let base = fragments.iter().find_map(|f| {
        let schema = schemas.get(text(&f["fragment"]))?;
        if schema.contains_key("selectTag")
            || !subset(&strings(&f["tags"]), tags)
            || !strings(&f["tags"]).iter().any(|t| weapon_tags.contains(t))
        {
            return None;
        }
        let allowed = schema.get("MotionBase")?;
        strings(&f["frag_tags"]).into_iter().find(|t| allowed.contains(t))
    });
    schemas
        .iter()
        .map(|(name, schema)| {
            let applicable = base
                .as_ref()
                .filter(|b| schema.get("MotionBase").is_some_and(|values| values.contains(b)))
                .cloned();
            (
                name.clone(),
                super::motion::query_base(schema.contains_key("selectTag"), applicable),
            )
        })
        .collect()
}

pub(super) fn speeds(groups: &BTreeMap<String, Vec<String>>) -> Vec<Option<&String>> {
    std::iter::once(None)
        .chain(groups.get("moveSpeed").into_iter().flatten().map(Some))
        .collect()
}

/// Base identity survives stance changes. Only the association lookup broadens;
/// actual fragment resolution still uses the exact requested stance/tags.
pub(super) fn base_context(tags: &[String], stances: &[String]) -> Vec<String> {
    let mut context = tags.to_vec();
    for stance in stances {
        if !context.contains(stance) {
            context.push(stance.clone());
        }
    }
    context
}
