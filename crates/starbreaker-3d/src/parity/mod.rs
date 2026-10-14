//! Writer parity (plan `docs/superpowers/plans/2026-10-14-glb-blend-parity.md`,
//! phase F): reduce a GLB and the `.blend` export of the same input to one
//! [`Summary`] each ([`from_glb`], [`from_blend`]), [`diff`] them into keys,
//! and check the keys against an [`Allowlist`] of known gaps.
//!
//! A summary holds, per mesh (by name, `_LOD<n>` / `.NNN` suffixes dropped):
//! vertex and triangle counts, an order-independent hash of the positions
//! (0.1 mm grid), normals, UV set count, colours, material slot count,
//! per-joint weight mass and morph target names; the skin joints' bind heads
//! (CryEngine space); and per clip, each channel's key times (and morph
//! weight values). Difference keys read `fixture/area/name/field`.

mod blend;
mod glb;

use std::collections::BTreeMap;

pub use blend::from_blend;
pub use glb::from_glb;

/// One mesh's comparable facts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshSummary {
    pub vertices: usize,
    pub triangles: usize,
    pub positions: u64,
    pub normals: bool,
    pub uv_sets: usize,
    pub colors: bool,
    pub material_slots: usize,
    /// Joint name → total weight over the mesh's vertices.
    pub joints: BTreeMap<String, f32>,
    pub morphs: Vec<String>,
}

/// One animated channel: key times (seconds), and values for morph weights.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Channel {
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}

/// One clip: `bone:<name>:rotation|translation`, `morph:<target>` and
/// `material:<param>` channels.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClipSummary {
    pub channels: BTreeMap<String, Channel>,
}

/// What a writer put in its file, reduced to comparable facts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub meshes: BTreeMap<String, MeshSummary>,
    /// Skin joint → bind head (CryEngine space).
    pub bones: BTreeMap<String, [f32; 3]>,
    pub clips: BTreeMap<String, ClipSummary>,
}

/// A mesh name without `_LOD<n>` and Blender `.NNN` suffixes.
pub(crate) fn normalize_name(name: &str) -> String {
    let mut name = name;
    if let Some((stem, digits)) = name.rsplit_once('.') {
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            name = stem;
        }
    }
    if let Some(i) = name.rfind("_LOD") {
        if name[i + 4..].chars().all(|c| c.is_ascii_digit()) && i + 4 < name.len() {
            name = &name[..i];
        }
    }
    name.to_string()
}

/// Order-independent hash of positions on a 0.1 mm grid.
pub(crate) fn position_hash(positions: impl Iterator<Item = [f32; 3]>) -> u64 {
    use std::hash::{Hash, Hasher};
    positions
        .map(|p| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            p.map(|x| (x as f64 * 1e4).round() as i64).hash(&mut h);
            h.finish()
        })
        .fold(0u64, u64::wrapping_add)
}

/// Insert under a unique key (`name`, then `name#2`, …).
pub(crate) fn insert_unique<V>(map: &mut BTreeMap<String, V>, name: String, value: V) {
    let mut key = name.clone();
    let mut n = 1;
    while map.contains_key(&key) {
        n += 1;
        key = format!("{name}#{n}");
    }
    map.insert(key, value);
}

fn close(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol * (1.0 + a.abs().max(b.abs()))
}

/// The differences between a GLB summary and a `.blend` summary, as keys
/// `fixture/area/name/field`. Meshes pair by name; when exactly one mesh on
/// each side is left unpaired, those two pair.
pub fn diff(fixture: &str, glb: &Summary, blend: &Summary) -> Vec<String> {
    let mut out = Vec::new();
    let mut key = |parts: &[&str]| out.push(std::iter::once(fixture).chain(parts.iter().copied()).collect::<Vec<_>>().join("/"));

    let mut pairs: Vec<(String, &MeshSummary, &MeshSummary)> = Vec::new();
    let only_glb: Vec<&String> = glb.meshes.keys().filter(|k| !blend.meshes.contains_key(*k)).collect();
    let only_blend: Vec<&String> = blend.meshes.keys().filter(|k| !glb.meshes.contains_key(*k)).collect();
    for (name, g) in &glb.meshes {
        if let Some(b) = blend.meshes.get(name) {
            pairs.push((name.clone(), g, b));
        }
    }
    if only_glb.len() == 1 && only_blend.len() == 1 {
        pairs.push((only_glb[0].clone(), &glb.meshes[only_glb[0]], &blend.meshes[only_blend[0]]));
    } else {
        for name in only_glb {
            key(&["mesh", name, "missing-in-blend"]);
        }
        for name in only_blend {
            key(&["mesh", name, "missing-in-glb"]);
        }
    }
    for (name, g, b) in &pairs {
        let fields: [(&str, bool); 7] = [
            ("vertices", g.vertices == b.vertices),
            ("triangles", g.triangles == b.triangles),
            ("positions", g.positions == b.positions),
            ("normals", g.normals == b.normals),
            ("uv_sets", g.uv_sets == b.uv_sets),
            ("colors", g.colors == b.colors),
            ("material_slots", g.material_slots == b.material_slots),
        ];
        for (field, same) in fields {
            if !same {
                key(&["mesh", name, field]);
            }
        }
        for joint in g.joints.keys().chain(b.joints.keys()).collect::<std::collections::BTreeSet<_>>() {
            match (g.joints.get(joint), b.joints.get(joint)) {
                (Some(x), Some(y)) if !close(*x, *y, 1e-3) => key(&["mesh", name, "joint", joint, "weight"]),
                (Some(_), None) => key(&["mesh", name, "joint", joint, "missing-in-blend"]),
                (None, Some(_)) => key(&["mesh", name, "joint", joint, "missing-in-glb"]),
                _ => {}
            }
        }
        if g.morphs != b.morphs {
            key(&["mesh", name, "morphs"]);
        }
    }

    for bone in glb.bones.keys().chain(blend.bones.keys()).collect::<std::collections::BTreeSet<_>>() {
        match (glb.bones.get(bone), blend.bones.get(bone)) {
            (Some(x), Some(y)) if (0..3).any(|k| (x[k] - y[k]).abs() > 1e-4) => key(&["bone", bone, "bind"]),
            (Some(_), None) => key(&["bone", bone, "missing-in-blend"]),
            (None, Some(_)) => key(&["bone", bone, "missing-in-glb"]),
            _ => {}
        }
    }

    for clip in glb.clips.keys().chain(blend.clips.keys()).collect::<std::collections::BTreeSet<_>>() {
        let (Some(g), Some(b)) = (glb.clips.get(clip), blend.clips.get(clip)) else {
            key(&["clip", clip, if glb.clips.contains_key(clip) { "missing-in-blend" } else { "missing-in-glb" }]);
            continue;
        };
        for channel in g.channels.keys().chain(b.channels.keys()).collect::<std::collections::BTreeSet<_>>() {
            match (g.channels.get(channel), b.channels.get(channel)) {
                (Some(x), Some(y)) => {
                    let same_len = |a: &[f32], b: &[f32], tol| a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p - q).abs() <= tol);
                    if !same_len(&x.times, &y.times, 1e-3) {
                        key(&["clip", clip, channel, "times"]);
                    }
                    if !same_len(&x.values, &y.values, 1e-4) {
                        key(&["clip", clip, channel, "values"]);
                    }
                }
                (Some(_), None) => key(&["clip", clip, channel, "missing-in-blend"]),
                (None, Some(_)) => key(&["clip", clip, channel, "missing-in-glb"]),
                _ => {}
            }
        }
    }
    out
}

/// Known gaps: one glob pattern per line (`*` matches anything, `#`
/// comments), each matching difference keys.
pub struct Allowlist {
    patterns: Vec<String>,
}

fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            let Some(at) = rest.find(part) else { return false };
            rest = &rest[at + part.len()..];
        }
    }
    true
}

impl Allowlist {
    pub fn parse(text: &str) -> Self {
        let patterns = text
            .lines()
            .map(|l| l.split('#').next().unwrap_or("").trim())
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect();
        Self { patterns }
    }

    /// `Err` listing differences no pattern allows, and stale patterns
    /// (matching nothing: fixed gaps must leave the list). A pattern naming a
    /// fixture is stale-checked when that fixture is in `checked`; a pattern
    /// whose fixture segment is a wildcard only when `check_wildcards` (the
    /// run covers every fixture the pattern was written for).
    pub fn check(&self, checked: &[&str], diffs: &[String], check_wildcards: bool) -> Result<(), String> {
        let unexpected: Vec<&String> = diffs.iter().filter(|d| !self.patterns.iter().any(|p| glob(p, d))).collect();
        let stale: Vec<&String> = self
            .patterns
            .iter()
            .filter(|p| {
                let fixture = p.split('/').next().unwrap_or("");
                let checkable = if fixture.contains('*') { check_wildcards } else { checked.contains(&fixture) };
                checkable && !diffs.iter().any(|d| glob(p, d))
            })
            .collect();
        if unexpected.is_empty() && stale.is_empty() {
            return Ok(());
        }
        Err(format!(
            "GLB/.blend parity: {} unexpected difference(s): {unexpected:#?}\n{} stale allowlist pattern(s): {stale:#?}",
            unexpected.len(),
            stale.len()
        ))
    }
}

#[cfg(test)]
mod tests;
