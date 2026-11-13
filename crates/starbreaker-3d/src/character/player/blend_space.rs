//! ParaGroup dimensions, examples and alias-indexed topology. Authored parameters
//! remain distinguishable from root-motion-derived parameters.
use super::xml::{Node, Result, error};
use serde_json::{Value, json};

pub(super) fn parse(root: &Node) -> Result<Value> {
    if root.tag != "ParaGroup" {
        return Err(error(format!("expected ParaGroup, got {}", root.tag)));
    }
    let dimensions=root.child("Dimensions").ok_or_else(||error("blend space has no dimensions"))?.children.iter().map(|p|Ok(json!({
        "name":p.attr("Name"),"min":p.number("Min",0.0)?,"max":p.number("Max",0.0)?,"cells":p.number("Cells",0.0)?
    }))).collect::<Result<Vec<_>>>()?;
    if dimensions.is_empty() {
        return Err(error("blend space has zero dimensions"));
    }
    let mut examples = Vec::new();
    for e in &root
        .child("ExampleList")
        .ok_or_else(|| error("blend space has no examples"))?
        .children
    {
        let mut params = Vec::new();
        let mut direct = Vec::new();
        for i in 0..dimensions.len() {
            let key = format!("SetPara{i}");
            params.push(if e.attrs.contains_key(&key) {
                json!(e.number(&key, 0.0)?)
            } else {
                Value::Null
            });
            direct.push(e.number(&format!("UseDirectlyForDeltaMotion{i}"), 0.0)? != 0.0);
        }
        if e.attr("AName").is_empty() {
            return Err(error("blend example missing AName"));
        }
        examples.push(
            json!({"clip":e.attr("AName").to_ascii_lowercase(),"alias":e.attr("Alias"),"params":params,
            "authored_params":params,"playback_scale":e.number("PlaybackScale",1.0)?,"direct_delta_motion":direct}),
        );
    }
    let mut faces = Vec::new();
    for f in root.child("Blendable").into_iter().flat_map(|n| &n.children) {
        let mut face = Vec::new();
        for i in 0..8 {
            let key = format!("p{i}");
            if !f.attrs.contains_key(&key) {
                break;
            }
            face.push(
                examples
                    .iter()
                    .position(|e| !text(&e["alias"]).is_empty() && text(&e["alias"]) == f.attr(&key))
                    .or_else(|| f.attr(&key).parse::<usize>().ok().filter(|i| *i < examples.len()))
                    .ok_or_else(|| error(format!("unknown blend-space alias {}", f.attr(&key))))?,
            );
        }
        if !face.is_empty() {
            faces.push(face);
        }
    }
    Ok(json!({"dimensions":dimensions,"examples":examples,"faces":faces}))
}
fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
pub(super) fn combine(root: &Node, components: &[Value]) -> Result<Value> {
    let mut dimensions = components.first().ok_or_else(|| error("empty combined space"))?["dimensions"]
        .as_array()
        .ok_or_else(|| error("component dimensions missing"))?
        .clone();
    let mut examples = Vec::new();
    let mut faces = Vec::new();
    for c in components {
        let dims = c["dimensions"]
            .as_array()
            .ok_or_else(|| error("component dimensions missing"))?;
        if dims.len() != dimensions.len() || dims.iter().zip(&dimensions).any(|(a, b)| a["name"] != b["name"]) {
            return Err(error("combined-space dimension mismatch"));
        }
        for (a, b) in dimensions.iter_mut().zip(dims) {
            a["min"] = json!(a["min"].as_f64().unwrap().min(b["min"].as_f64().unwrap()));
            a["max"] = json!(a["max"].as_f64().unwrap().max(b["max"].as_f64().unwrap()));
        }
        let offset = examples.len();
        for e in c["examples"]
            .as_array()
            .ok_or_else(|| error("component examples missing"))?
        {
            let mut e = e.clone();
            e["component"] = c["id"].clone();
            examples.push(e);
        }
        for f in c["faces"].as_array().ok_or_else(|| error("component faces missing"))? {
            faces.push(
                f.as_array()
                    .ok_or_else(|| error("invalid face"))?
                    .iter()
                    .map(|i| i.as_u64().unwrap() as usize + offset)
                    .collect::<Vec<_>>(),
            );
        }
    }
    let selectors=root.child("Dimensions").into_iter().flat_map(|n|&n.children).map(|n|json!({"name":n.attr("name"),"choose_blend_space":n.attr("ChooseBlendSpace")=="1","locked":n.attr("locked")=="1"})).collect::<Vec<_>>();
    Ok(
        json!({"dimensions":dimensions,"examples":examples,"faces":faces,"components":components.iter().map(|c|c["id"].clone()).collect::<Vec<_>>(),"selectors":selectors}),
    )
}
