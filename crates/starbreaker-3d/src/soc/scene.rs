//! Multi-zone scene composition — gluing many SOCs into one world.
//!
//! A single SOC file holds the brushes / entities / visareas for **one**
//! object container. A real level (an Executive Hangar, a hangar interior,
//! a station module) is built up by nesting containers: a top-level
//! "assembly" socpak references zero or more child socpaks with a
//! per-child `pos` / `rot` attribute, and each of those may in turn
//! reference its own children. The engine resolves the world transform of
//! each leaf brush by composing the local-space placement against the chain
//! of parent transforms — that is the `CZoneSystem::GetTransform` pattern
//! the Star Citizen rendering pipeline uses.
//!
//! The per-child `<Child name="..." pos="..." rot="..."/>` entries inside
//! a socpak's main XML give a `(socpak_name, parent_pos, parent_rot)`
//! tuple, and chaining those tuples is exactly the parent QuatTS the
//! brush parser expects. The XML in a `.socpak` is a CryXmlB blob, so we
//! pull it out via the in-tree zero-copy reader.
//!
//! # What is intentionally not in scope
//!
//! - Octree node walks. Brushes are still produced by the byte-level scan
//!   in [`super::brushes::parse`]. The supervisor task to RE the parent
//!   `QuatTS` per octree node is still open, and until that lands we apply
//!   the zone's parent QuatTS uniformly to every brush in the SOC. For
//!   SOCs whose every brush sits under the same node — which empirically
//!   covers the assembly/interior containers used by Executive Hangar —
//!   this matches the engine's own world placement.
//!
//! - Mesh decimation, CGF lookup, p4k extraction. Those happen in the
//!   viewer; this module's job is to produce typed `(world_pos, world_rot,
//!   mesh_path)` tuples from the SOC data.

use super::brushes::{self, BrushInstance, ParentTransform, SocError};
use super::entities::{self, EntityKind, EntityPlacement};
use glam::{DQuat, DVec3};
use starbreaker_p4k::{MappedP4k, P4kArchive};

// ── Public types ────────────────────────────────────────────────────────────

/// A single child reference parsed out of a socpak's main XML.
#[derive(Debug, Clone)]
pub struct ChildSocpakRef {
    /// `name` attribute exactly as written in the XML (case-preserving).
    pub name: String,
    /// `pos` attribute parsed as world-space position relative to the
    /// containing socpak.
    pub pos: [f64; 3],
    /// `rot` attribute parsed as a unit quaternion.
    pub rot: [f64; 4], // (x, y, z, w) glam ordering
}

/// One zone's contribution to a composed scene.
#[derive(Debug, Clone, Default)]
pub struct SceneZone {
    /// Logical zone name, derived from the socpak path (no extension).
    pub name: String,
    /// Per-zone parent transform that was applied to every brush /
    /// entity below.
    pub parent_translation: [f64; 3],
    pub parent_rotation: [f64; 4],
    /// World-space brush placements.
    pub brushes: Vec<BrushInstance>,
    /// Mesh paths referenced by `brush.mesh_index`.
    pub mesh_paths: Vec<String>,
    /// World-space entity placements (lights, doors, loot, mesh-bearing
    /// entities).
    pub entities: Vec<EntityPlacement>,
}

/// A fully-composed multi-zone scene.
#[derive(Debug, Clone, Default)]
pub struct ComposedScene {
    pub zones: Vec<SceneZone>,
}

impl ComposedScene {
    /// Total brush count across every zone.
    pub fn brush_count(&self) -> usize {
        self.zones.iter().map(|z| z.brushes.len()).sum()
    }

    /// Total entity count across every zone.
    pub fn entity_count(&self) -> usize {
        self.zones.iter().map(|z| z.entities.len()).sum()
    }

    /// Total light entity count across every zone.
    pub fn light_count(&self) -> usize {
        self.zones
            .iter()
            .flat_map(|z| z.entities.iter())
            .filter(|e| e.kind == EntityKind::Light)
            .count()
    }

    /// Compute the AABB across every brush translation, in world space.
    /// Returns `(min, max)` or `None` if the scene has no brushes.
    pub fn brush_aabb(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut iter = self.zones.iter().flat_map(|z| z.brushes.iter());
        let first = iter.next()?;
        let mut mn = first.translation;
        let mut mx = first.translation;
        for b in iter {
            let t = b.translation;
            for i in 0..3 {
                if t[i] < mn[i] {
                    mn[i] = t[i];
                }
                if t[i] > mx[i] {
                    mx[i] = t[i];
                }
            }
        }
        Some((mn, mx))
    }
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Errors produced while composing a multi-socpak scene.
#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("socpak entry not found in p4k: {0}")]
    SocpakNotFound(String),

    #[error("failed to read socpak '{path}': {message}")]
    ReadSocpak { path: String, message: String },

    #[error("socpak '{0}' is not a valid zip archive")]
    InvalidSocpak(String),

    #[error("socpak '{0}' has no main XML")]
    MissingMainXml(String),

    #[error("socpak '{0}' has no .soc payload")]
    MissingSoc(String),

    #[error(transparent)]
    Soc(#[from] SocError),
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Compose a scene from one root socpak, recursively expanding child
/// socpak references. Each zone is the SOC inside one socpak; child
/// transforms accumulate down the tree.
///
/// `max_depth` bounds the recursion. Two is sufficient for the standard
/// "assembly -> interior -> module" hierarchy used by Executive Hangar
/// and similar dungeons; pass a larger value if a level is known to nest
/// deeper.
pub fn compose_from_root(
    p4k: &MappedP4k,
    root_socpak_path: &str,
    max_depth: u32,
) -> Result<ComposedScene, SceneError> {
    let mut scene = ComposedScene::default();
    walk_socpak(
        p4k,
        root_socpak_path,
        &ParentTransform::identity(),
        max_depth,
        &mut scene,
    )?;
    Ok(scene)
}

/// Compose a scene from an explicit list of socpak paths to load at the
/// world origin (parent = identity). Useful for "load these N modules with
/// no parent transform" — the tests use this when they don't have a
/// top-level XML to walk.
pub fn compose_from_flat_list(
    p4k: &MappedP4k,
    socpak_paths: &[&str],
) -> Result<ComposedScene, SceneError> {
    let mut scene = ComposedScene::default();
    for path in socpak_paths {
        load_one_socpak_into_scene(p4k, path, &ParentTransform::identity(), &mut scene)?;
    }
    Ok(scene)
}

/// Parse a socpak's child-socpak references from its main XML, without
/// recursing. Exposed so callers can drive their own walk strategy.
pub fn read_child_refs_from_socpak(
    p4k: &MappedP4k,
    socpak_path: &str,
) -> Result<Vec<ChildSocpakRef>, SceneError> {
    let socpak_data = read_socpak_bytes(p4k, socpak_path)?;
    let inner = P4kArchive::from_bytes(&socpak_data)
        .map_err(|_| SceneError::InvalidSocpak(socpak_path.to_string()))?;

    let xml_bytes = read_main_xml(&inner)
        .ok_or_else(|| SceneError::MissingMainXml(socpak_path.to_string()))?;

    Ok(parse_child_refs(&xml_bytes))
}

// ── Recursive walker ────────────────────────────────────────────────────────

fn walk_socpak(
    p4k: &MappedP4k,
    socpak_path: &str,
    parent: &ParentTransform,
    depth_remaining: u32,
    scene: &mut ComposedScene,
) -> Result<(), SceneError> {
    // Always parse this socpak's own SOC payload.
    load_one_socpak_into_scene(p4k, socpak_path, parent, scene)?;

    if depth_remaining == 0 {
        return Ok(());
    }

    // Read its main XML and recurse into children.
    let socpak_data = match read_socpak_bytes(p4k, socpak_path) {
        Ok(d) => d,
        Err(SceneError::SocpakNotFound(_)) => return Ok(()),
        Err(e) => return Err(e),
    };
    let inner = match P4kArchive::from_bytes(&socpak_data) {
        Ok(z) => z,
        Err(_) => return Ok(()),
    };
    let Some(xml_bytes) = read_main_xml(&inner) else {
        return Ok(());
    };

    let children = parse_child_refs(&xml_bytes);
    for child in &children {
        let child_parent = compose_parents(parent, &child.pos, &child.rot);
        // Resolve the child socpak path. The XML uses paths relative to
        // the p4k root (typically `Data/...`).
        let child_path = normalize_p4k_path(&child.name);
        match walk_socpak(p4k, &child_path, &child_parent, depth_remaining - 1, scene) {
            Ok(()) => {}
            Err(SceneError::SocpakNotFound(_)) => continue,
            Err(SceneError::MissingSoc(_)) => continue,
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

fn load_one_socpak_into_scene(
    p4k: &MappedP4k,
    socpak_path: &str,
    parent: &ParentTransform,
    scene: &mut ComposedScene,
) -> Result<(), SceneError> {
    let socpak_data = read_socpak_bytes(p4k, socpak_path)?;
    let inner = P4kArchive::from_bytes(&socpak_data)
        .map_err(|_| SceneError::InvalidSocpak(socpak_path.to_string()))?;

    let soc_bytes = match read_first_soc(&inner) {
        Some(b) => b,
        None => return Err(SceneError::MissingSoc(socpak_path.to_string())),
    };

    let zone_name = socpak_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(socpak_path)
        .strip_suffix(".socpak")
        .unwrap_or(socpak_path)
        .to_string();

    // Brushes — soft-fail on every SOC error. Some socpaks carry SOC
    // payloads that do not have a StatObj chunk at all (parent / assembly
    // containers), and earlier-build SOC files may use a non-v15 brush
    // record format. In both cases the right thing for a multi-zone walk
    // is to skip the brushes from that zone, not abort the walk.
    let brushes = match brushes::parse(&soc_bytes, parent) {
        Ok(b) => b,
        Err(_) => brushes::SocBrushes::default(),
    };

    // Entities — soft-fail on parse errors, lights are best-effort.
    let entity_payload = match entities::parse(&soc_bytes, parent) {
        Ok(e) => e,
        Err(_) => entities::SocEntities::default(),
    };

    scene.zones.push(SceneZone {
        name: zone_name,
        parent_translation: parent.translation.into(),
        parent_rotation: [
            parent.rotation.x,
            parent.rotation.y,
            parent.rotation.z,
            parent.rotation.w,
        ],
        brushes: brushes.brushes,
        mesh_paths: brushes.mesh_paths,
        entities: entity_payload.entities,
    });

    Ok(())
}

// ── socpak / xml helpers ────────────────────────────────────────────────────

fn read_socpak_bytes(p4k: &MappedP4k, socpak_path: &str) -> Result<Vec<u8>, SceneError> {
    let normalized = normalize_p4k_path(socpak_path);
    let entry = p4k
        .entry_case_insensitive(&normalized)
        .ok_or_else(|| SceneError::SocpakNotFound(normalized.clone()))?;
    p4k.read(entry).map_err(|e| SceneError::ReadSocpak {
        path: normalized,
        message: e.to_string(),
    })
}

fn normalize_p4k_path(path: &str) -> String {
    let normalized = path.replace('/', "\\");
    if normalized
        .to_ascii_lowercase()
        .starts_with("data\\")
    {
        normalized
    } else {
        format!("Data\\{normalized}")
    }
}

/// Find the main XML inside a socpak. The convention from the reference
/// parser: pick the first `.xml` at the socpak root that is not the
/// `*.entxml`, editor metadata, or entity-data sidecar.
fn read_main_xml(inner: &P4kArchive<'_>) -> Option<Vec<u8>> {
    let mut candidate = None;
    for entry in inner.entries() {
        let name_lc = entry.name.to_ascii_lowercase();
        if !name_lc.ends_with(".xml") {
            continue;
        }
        if name_lc.contains("editor")
            || name_lc.contains("metadata")
            || name_lc.contains("entdata")
            || name_lc.contains("entxml")
        {
            continue;
        }
        candidate = Some(entry);
        break;
    }
    let entry = candidate?;
    inner.read(entry).ok()
}

fn read_first_soc(inner: &P4kArchive<'_>) -> Option<Vec<u8>> {
    for entry in inner.entries() {
        if entry.name.to_ascii_lowercase().ends_with(".soc") {
            if let Ok(bytes) = inner.read(entry) {
                return Some(bytes);
            }
        }
    }
    None
}

/// Parse `<Child name="..." pos="x,y,z" rot="w,x,y,z"/>` references out
/// of a socpak XML. Supports both CryXmlB binary XML and the plain-text
/// XML that some socpaks carry (the reference build_exec_hangar.py treats
/// them with a regex; we use a 1-pass scanner that handles either).
fn parse_child_refs(xml_bytes: &[u8]) -> Vec<ChildSocpakRef> {
    if starbreaker_cryxml::is_cryxmlb(xml_bytes) {
        return parse_child_refs_cryxml(xml_bytes);
    }
    parse_child_refs_text(xml_bytes)
}

fn parse_child_refs_cryxml(xml_bytes: &[u8]) -> Vec<ChildSocpakRef> {
    let xml = match starbreaker_cryxml::from_bytes(xml_bytes) {
        Ok(x) => x,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    let root = xml.root();
    walk_cryxml_for_children(&xml, root, &mut out, 0, 16);
    out
}

fn walk_cryxml_for_children(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
    out: &mut Vec<ChildSocpakRef>,
    depth: u32,
    max_depth: u32,
) {
    if depth > max_depth {
        return;
    }
    let tag = xml.node_tag(node);
    if tag == "Child" {
        let attrs: std::collections::HashMap<&str, &str> =
            xml.node_attributes(node).collect();
        if let Some(name) = attrs.get("name").copied()
            && let Some(pos_s) = attrs.get("pos").copied()
            && let Some(rot_s) = attrs.get("rot").copied()
            && name.to_ascii_lowercase().ends_with(".socpak")
            && let Some(pos) = parse_csv3(pos_s)
            && let Some(rot) = parse_csv4_w_first(rot_s)
        {
            out.push(ChildSocpakRef {
                name: name.to_string(),
                pos,
                rot,
            });
        }
    }

    for child in xml.node_children(node) {
        walk_cryxml_for_children(xml, child, out, depth + 1, max_depth);
    }
}

fn parse_child_refs_text(xml_bytes: &[u8]) -> Vec<ChildSocpakRef> {
    let Ok(text) = std::str::from_utf8(xml_bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // Conservative scan: each `<Child ... name="...socpak" ... pos="..." ... rot="..." ... />`
    // We don't need a full XML parser — we only consume well-formed
    // attribute strings that we know the engine writes verbatim.
    for chunk in text.split("<Child").skip(1) {
        let Some(end) = chunk.find('>') else { continue };
        let block = &chunk[..end];

        let Some(name) = extract_attr(block, "name") else {
            continue;
        };
        if !name.to_ascii_lowercase().ends_with(".socpak") {
            continue;
        }
        let Some(pos_s) = extract_attr(block, "pos") else {
            continue;
        };
        let Some(rot_s) = extract_attr(block, "rot") else {
            continue;
        };
        let Some(pos) = parse_csv3(&pos_s) else { continue };
        let Some(rot) = parse_csv4_w_first(&rot_s) else {
            continue;
        };
        out.push(ChildSocpakRef {
            name,
            pos,
            rot,
        });
    }
    out
}

fn extract_attr(block: &str, key: &str) -> Option<String> {
    // Find ` key="..."` (with the leading space to avoid prefix matches like
    // `pos` matching the start of `position`).
    let needle = format!(" {key}=\"");
    let start = block.find(&needle)? + needle.len();
    let rest = &block[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn parse_csv3(s: &str) -> Option<[f64; 3]> {
    let mut iter = s.split(',').map(str::trim);
    let x = iter.next()?.parse::<f64>().ok()?;
    let y = iter.next()?.parse::<f64>().ok()?;
    let z = iter.next()?.parse::<f64>().ok()?;
    Some([x, y, z])
}

/// Parse a CryEngine 4-tuple stored as `w,x,y,z` (scalar first) and return
/// it in glam's `(x, y, z, w)` order.
fn parse_csv4_w_first(s: &str) -> Option<[f64; 4]> {
    let mut iter = s.split(',').map(str::trim);
    let w = iter.next()?.parse::<f64>().ok()?;
    let x = iter.next()?.parse::<f64>().ok()?;
    let y = iter.next()?.parse::<f64>().ok()?;
    let z = iter.next()?.parse::<f64>().ok()?;
    Some([x, y, z, w])
}

// ── Transform composition ───────────────────────────────────────────────────

/// Compose a child's local-space `(pos, rot)` with the parent's QuatTS to
/// produce the child's world-space QuatTS. Uses the standard rigid-body
/// composition: `world_t = parent_t + parent_q * child_t`, `world_q =
/// parent_q * child_q`.
fn compose_parents(parent: &ParentTransform, child_pos: &[f64; 3], child_rot: &[f64; 4]) -> ParentTransform {
    let child_q = DQuat::from_xyzw(child_rot[0], child_rot[1], child_rot[2], child_rot[3])
        .normalize();
    let child_t = DVec3::new(child_pos[0], child_pos[1], child_pos[2]);

    let world_t = parent.translation + parent.rotation * child_t;
    let world_q = (parent.rotation * child_q).normalize();

    ParentTransform {
        rotation: world_q,
        translation: world_t,
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_csv3_handles_simple_input() {
        assert_eq!(parse_csv3("1, 2, 3"), Some([1.0, 2.0, 3.0]));
        assert_eq!(parse_csv3("1,2"), None);
        assert_eq!(parse_csv3("a,b,c"), None);
    }

    #[test]
    fn parse_csv4_reorders_w_first_to_glam() {
        // CryEngine identity quaternion is "1,0,0,0" (w,x,y,z).
        let q = parse_csv4_w_first("1,0,0,0").unwrap();
        assert_eq!(q, [0.0, 0.0, 0.0, 1.0]); // glam (x,y,z,w)
    }

    #[test]
    fn extract_attr_finds_quoted_value() {
        let block = " name=\"foo.socpak\" pos=\"1,2,3\" rot=\"1,0,0,0\"";
        assert_eq!(extract_attr(block, "name"), Some("foo.socpak".to_string()));
        assert_eq!(extract_attr(block, "pos"), Some("1,2,3".to_string()));
        assert_eq!(extract_attr(block, "rot"), Some("1,0,0,0".to_string()));
        assert_eq!(extract_attr(block, "missing"), None);
    }

    #[test]
    fn parse_child_refs_text_reads_one_child() {
        let xml = br#"<root><Child name="a/b/foo.socpak" pos="1,2,3" rot="1,0,0,0"/></root>"#;
        let refs = parse_child_refs_text(xml);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "a/b/foo.socpak");
        assert_eq!(refs[0].pos, [1.0, 2.0, 3.0]);
        // Glam (x,y,z,w) order — w-first input "1,0,0,0" -> identity.
        assert_eq!(refs[0].rot, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn compose_parents_is_associative_for_translation() {
        let parent = ParentTransform {
            rotation: DQuat::IDENTITY,
            translation: DVec3::new(10.0, 0.0, 0.0),
        };
        let composed = compose_parents(&parent, &[5.0, 0.0, 0.0], &[0.0, 0.0, 0.0, 1.0]);
        assert!((composed.translation.x - 15.0).abs() < 1e-9);
        assert_eq!(composed.translation.y, 0.0);
        assert_eq!(composed.translation.z, 0.0);
    }

    #[test]
    fn brush_aabb_returns_none_when_empty() {
        let scene = ComposedScene::default();
        assert!(scene.brush_aabb().is_none());
    }
}
