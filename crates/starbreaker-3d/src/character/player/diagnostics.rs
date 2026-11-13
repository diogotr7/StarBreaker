//! Explicit missing-clip metadata without changing authored fragment selection.
use super::ports::text;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Missing assets never change the authored Mannequin selection.
pub(super) fn mark_missing(
    states: &mut [Value],
    spaces: &mut BTreeMap<String, Value>,
    missing: &BTreeSet<String>,
    gender: &str,
) -> Vec<Value> {
    for space in spaces.values_mut() {
        for example in space["examples"].as_array_mut().into_iter().flatten() {
            if missing.contains(text(&example["clip"])) {
                example["missing"] = json!(true);
            }
        }
    }
    let mut diagnostics = Vec::new();
    for state in states {
        for f in state["fragments"].as_array_mut().into_iter().flatten() {
            let mut absent = BTreeSet::new();
            for layer in f["layers"].as_array_mut().into_iter().flatten() {
                if missing.contains(text(&layer["clip"])) {
                    absent.insert(text(&layer["clip"]).to_owned());
                    layer["missing"] = json!(true);
                }
                if let Some(space) = spaces.get(text(&layer["blend_space"])) {
                    for example in space["examples"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|e| e["missing"] == true)
                    {
                        absent.insert(text(&example["clip"]).to_owned());
                        layer["missing"] = json!(true);
                    }
                }
            }
            for clip in absent {
                let diagnostic = json!({"kind":"missing_clip","clip":clip,"fragment":f["fragment"],"tags":f["tags"],"frag_tags":f["frag_tags"],"gender":gender});
                if !diagnostics.contains(&diagnostic) {
                    diagnostics.push(diagnostic);
                }
            }
        }
    }
    diagnostics
}
