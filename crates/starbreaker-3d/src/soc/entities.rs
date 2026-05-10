//! Entity placements parsed from a SOC's CryXmlB chunk (0x0004).
//!
//! Each `.soc` file embeds a CryXmlB tree under chunk type 0x0004. The tree
//! lists `<Entity>` records that the engine spawns when the container is
//! activated: lights, doors, loot points, audio triggers, etc. This module
//! decodes a useful subset, focusing on what a renderer needs to draw the
//! scene: light placements, doors, and any entity that references a brush
//! mesh through the StatObj table.
//!
//! Decoding leans on the existing [`starbreaker_cryxml`] zero-copy reader
//! rather than re-implementing a binary XML decoder.
//!
//! # Lights
//!
//! Light entities use one of these `EntityClass` strings: `Light`, `LightBox`,
//! `LightGroup`, `LightGroupPoweredItem`. The viewer needs at minimum the
//! world-space placement (position, rotation) and a stable name. Light
//! intensity / colour / radius live deeper inside `PropertiesDataCore`; that
//! is where the existing socpak loader looks. For the SOC parser port we
//! emit the basic placement plus the entity class so the consumer can decide
//! whether to drive a punctual light or a baked light group.

use super::brushes::{ParentTransform, SocError};
use super::common::{
    CHUNK_TYPE_CRYXMLB, CHUNK_TYPE_STATOBJ, chunk_slice, parse_chunk_table, parse_crch_header,
    read_u32,
};
use glam::{DQuat, DVec3};

const STATOBJ_PATH_LEN: usize = 256;

// ── Public types ────────────────────────────────────────────────────────────

/// Coarse classification driven off the `EntityClass` attribute. The renderer
/// uses this to decide which payload to honour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    /// `Light`, `LightBox`, `LightGroup`, `LightGroupPoweredItem`.
    Light,
    /// Anything matching `Door*` or `Door_*`.
    Door,
    /// Loot points (`LootPoint`, `LootSpawner`, `LootContainer*`).
    Loot,
    /// Anything else worth surfacing (entities that reference a CGF mesh).
    Other,
}

/// One decoded entity placement from chunk 0x0004.
#[derive(Debug, Clone)]
pub struct EntityPlacement {
    /// `Name` attribute on the entity node (may be empty).
    pub name: String,
    /// Raw `EntityClass` string.
    pub entity_class: String,
    /// Coarse classification.
    pub kind: EntityKind,
    /// World-space translation (parent QuatTS composed with the entity's
    /// own pos vector).
    pub translation: [f32; 3],
    /// World-space rotation as a quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
    /// Resolved mesh path, populated when the entity references a brush
    /// mesh via `GeomLink/BrushID` or a child `Geometry` node with `path`.
    pub mesh_path: Option<String>,
    /// Index into the SOC's StatObj table when the entity used a `BrushID`
    /// to reference a brush mesh. `None` when the mesh path came from a
    /// direct `Geometry` reference or could not be resolved.
    pub mesh_index: Option<u16>,
}

/// Lights, doors, loot points, mesh-referencing entities — all parsed from
/// the CryXmlB chunk of a single SOC.
#[derive(Debug, Clone, Default)]
pub struct SocEntities {
    /// Lower-cased StatObj geometry paths (same set as the brush parser
    /// produces, hoisted here for `BrushID` resolution). Empty when the SOC
    /// has no StatObj chunk.
    pub mesh_paths: Vec<String>,
    /// All decoded entities, in source order.
    pub entities: Vec<EntityPlacement>,
}

impl SocEntities {
    /// View only the light placements.
    pub fn lights(&self) -> impl Iterator<Item = &EntityPlacement> {
        self.entities.iter().filter(|e| e.kind == EntityKind::Light)
    }

    /// View only the door placements.
    pub fn doors(&self) -> impl Iterator<Item = &EntityPlacement> {
        self.entities.iter().filter(|e| e.kind == EntityKind::Door)
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Parse entity placements from a SOC, composing their `Pos`/`Rotate`
/// attributes with `parent` to produce world-space placements. SOCs that lack
/// a CryXmlB chunk (some meta-only containers) return an empty result rather
/// than an error.
pub fn parse(data: &[u8], parent: &ParentTransform) -> Result<SocEntities, SocError> {
    let header = parse_crch_header(data).ok_or(SocError::BadMagic)?;
    let chunks = parse_chunk_table(data, &header).ok_or(SocError::BadChunkTable)?;

    // StatObj table is optional for entity parsing, but when present it lets
    // us resolve `BrushID` references to mesh paths.
    let mesh_paths = chunks
        .iter()
        .find(|c| c.chunk_type == CHUNK_TYPE_STATOBJ)
        .map(|c| read_statobj_paths(data, c))
        .unwrap_or_default();

    // Find the CryXmlB chunk; absence is benign — emit empty entities.
    let Some(xml_chunk) = chunks.iter().find(|c| c.chunk_type == CHUNK_TYPE_CRYXMLB) else {
        return Ok(SocEntities {
            mesh_paths,
            entities: Vec::new(),
        });
    };

    let xml_bytes = chunk_slice(data, xml_chunk);
    let entities = parse_cryxml_entities(xml_bytes, parent, &mesh_paths);

    Ok(SocEntities {
        mesh_paths,
        entities,
    })
}

// ── StatObj path table read (mirrors brushes.rs) ────────────────────────────

fn read_statobj_paths(data: &[u8], chunk: &super::common::ChunkEntry) -> Vec<String> {
    let start = chunk.offset as usize;
    let end = start
        .saturating_add(chunk.size as usize)
        .min(data.len());
    if end < start + 8 {
        return Vec::new();
    }
    let count = read_u32(data, start + 4) as usize;
    let mut off = start + 8;
    let table_end = off.saturating_add(count.saturating_mul(STATOBJ_PATH_LEN));
    if table_end > end {
        return Vec::new();
    }
    let mut paths = Vec::with_capacity(count);
    for _ in 0..count {
        let raw = &data[off..off + STATOBJ_PATH_LEN];
        let nul = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let s = String::from_utf8_lossy(&raw[..nul]).to_lowercase();
        paths.push(s);
        off += STATOBJ_PATH_LEN;
    }
    paths
}

// ── CryXmlB walk ────────────────────────────────────────────────────────────

fn parse_cryxml_entities(
    xml_bytes: &[u8],
    parent: &ParentTransform,
    mesh_paths: &[String],
) -> Vec<EntityPlacement> {
    let xml = match starbreaker_cryxml::from_bytes(xml_bytes) {
        Ok(x) => x,
        Err(_) => return Vec::new(),
    };

    let root = xml.root();
    let root_tag = xml.node_tag(root);

    // Same root-tag set as the existing socpak loader.
    let entity_root = if root_tag == "Entities" || root_tag == "SCOC_Entities" {
        Some(root)
    } else {
        xml.node_children(root).find(|child| {
            let tag = xml.node_tag(child);
            tag == "Entities" || tag == "SCOC_Entities"
        })
    };

    let Some(entity_root) = entity_root else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entity in xml.node_children(entity_root) {
        if xml.node_tag(entity) != "Entity" {
            continue;
        }

        let attrs: std::collections::HashMap<&str, &str> =
            xml.node_attributes(entity).collect();
        let entity_class = attrs.get("EntityClass").copied().unwrap_or("").to_string();
        let kind = classify(&entity_class);

        // Filter rule (per brief): keep lights, doors, loot, and any entity
        // that references a brush mesh. Skip the rest to keep the output
        // small. Entities can still reference a mesh through
        // `GeomLink`/`Geometry` even when the class is not light/door/loot.
        let (mesh_path, mesh_index) = resolve_entity_mesh(&xml, entity, mesh_paths);
        if matches!(kind, EntityKind::Other) && mesh_path.is_none() {
            continue;
        }

        let name = attrs.get("Name").copied().unwrap_or("").to_string();
        let pos = parse_csv_pos(attrs.get("Pos").copied());
        let rot_quat = parse_csv_rotate(attrs.get("Rotate").copied());

        let world_translation = parent.translation + parent.rotation * pos;
        let world_rotation = (parent.rotation * rot_quat).normalize();

        out.push(EntityPlacement {
            name,
            entity_class,
            kind,
            translation: [
                world_translation.x as f32,
                world_translation.y as f32,
                world_translation.z as f32,
            ],
            rotation: [
                world_rotation.x as f32,
                world_rotation.y as f32,
                world_rotation.z as f32,
                world_rotation.w as f32,
            ],
            mesh_path,
            mesh_index,
        });
    }

    out
}

fn classify(entity_class: &str) -> EntityKind {
    match entity_class {
        "Light" | "LightBox" | "LightGroup" | "LightGroupPoweredItem" => EntityKind::Light,
        s if s.starts_with("Door") => EntityKind::Door,
        s if s.starts_with("Loot") => EntityKind::Loot,
        _ => EntityKind::Other,
    }
}

/// Resolve an `<Entity>` to a mesh path via two strategies:
/// `GeomLink/BrushID` -> StatObj index, then any descendant
/// `<Geometry path="..."/>`.
fn resolve_entity_mesh(
    xml: &starbreaker_cryxml::CryXml,
    entity: &starbreaker_cryxml::CryXmlNode,
    mesh_paths: &[String],
) -> (Option<String>, Option<u16>) {
    // Strategy 1: GeomLink with BrushID.
    if let Some(brush_id) = find_descendant_attr(xml, entity, "GeomLink", "BrushID", 8)
        && let Ok(id) = brush_id.parse::<i32>()
        && id >= 0
        && (id as usize) < mesh_paths.len()
    {
        return (Some(mesh_paths[id as usize].clone()), Some(id as u16));
    }

    // Strategy 2: any descendant <Geometry path="...">.
    if let Some(path) = find_descendant_attr(xml, entity, "Geometry", "path", 8)
        && !path.is_empty()
    {
        return (Some(path.to_string()), None);
    }

    (None, None)
}

/// Walk descendants up to `max_depth`, returning the first node with tag
/// `tag` that has the requested attribute.
fn find_descendant_attr<'a>(
    xml: &'a starbreaker_cryxml::CryXml,
    parent: &'a starbreaker_cryxml::CryXmlNode,
    tag: &str,
    attr: &str,
    max_depth: u32,
) -> Option<&'a str> {
    if max_depth == 0 {
        return None;
    }
    for child in xml.node_children(parent) {
        if xml.node_tag(child) == tag {
            for (k, v) in xml.node_attributes(child) {
                if k == attr {
                    return Some(v);
                }
            }
        }
        if let Some(found) = find_descendant_attr(xml, child, tag, attr, max_depth - 1) {
            return Some(found);
        }
    }
    None
}

// ── String parsers ──────────────────────────────────────────────────────────

fn parse_csv_pos(s: Option<&str>) -> DVec3 {
    let Some(s) = s else { return DVec3::ZERO };
    let mut iter = s.split(',').filter_map(|p| p.trim().parse::<f64>().ok());
    let x = iter.next().unwrap_or(0.0);
    let y = iter.next().unwrap_or(0.0);
    let z = iter.next().unwrap_or(0.0);
    DVec3::new(x, y, z)
}

/// CryEngine `Rotate` attribute is `w,x,y,z` (scalar first). Glam's
/// `DQuat::from_xyzw` expects `[x, y, z, w]`, so we re-order here.
fn parse_csv_rotate(s: Option<&str>) -> DQuat {
    let Some(s) = s else { return DQuat::IDENTITY };
    let mut iter = s.split(',').filter_map(|p| p.trim().parse::<f64>().ok());
    let w = iter.next().unwrap_or(1.0);
    let x = iter.next().unwrap_or(0.0);
    let y = iter.next().unwrap_or(0.0);
    let z = iter.next().unwrap_or(0.0);
    DQuat::from_xyzw(x, y, z, w).normalize()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_recognises_light_classes() {
        assert_eq!(classify("Light"), EntityKind::Light);
        assert_eq!(classify("LightBox"), EntityKind::Light);
        assert_eq!(classify("LightGroup"), EntityKind::Light);
        assert_eq!(classify("LightGroupPoweredItem"), EntityKind::Light);
    }

    #[test]
    fn classify_recognises_doors_and_loot() {
        assert_eq!(classify("Door"), EntityKind::Door);
        assert_eq!(classify("Door_Sliding"), EntityKind::Door);
        assert_eq!(classify("LootPoint"), EntityKind::Loot);
        assert_eq!(classify("LootSpawner"), EntityKind::Loot);
        assert_eq!(classify("ActionArea"), EntityKind::Other);
    }

    #[test]
    fn parse_csv_rotate_handles_w_first_quaternion() {
        // CryEngine "1,0,0,0" is identity in (w, x, y, z) order.
        let q = parse_csv_rotate(Some("1,0,0,0"));
        assert!((q.w - 1.0).abs() < 1e-6);
        assert!(q.x.abs() < 1e-6);
        assert!(q.y.abs() < 1e-6);
        assert!(q.z.abs() < 1e-6);

        // 90 deg rotation about Z: w=cos(45)=0.7071, z=sin(45)=0.7071
        let q = parse_csv_rotate(Some("0.7071068,0,0,0.7071068"));
        assert!((q.w - 0.7071068).abs() < 1e-5);
        assert!((q.z - 0.7071068).abs() < 1e-5);
    }

    #[test]
    fn parse_csv_pos_handles_short_inputs() {
        assert_eq!(parse_csv_pos(Some("1,2,3")), DVec3::new(1.0, 2.0, 3.0));
        assert_eq!(parse_csv_pos(None), DVec3::ZERO);
        assert_eq!(parse_csv_pos(Some("")), DVec3::ZERO);
    }

    #[test]
    fn parse_returns_empty_when_no_cryxml_chunk() {
        // Build a minimal CrCh file with only a StatObj chunk. The entity
        // parser must not error — it should return empty placements.
        let data = build_crch_with_only_statobj();
        let parsed = parse(&data, &ParentTransform::identity()).expect("parse ok");
        assert!(parsed.entities.is_empty());
    }

    fn build_crch_with_only_statobj() -> Vec<u8> {
        // 16-byte header + 8-byte StatObj body + 16-byte chunk-table entry.
        let mut data = Vec::new();
        data.extend_from_slice(b"CrCh");
        data.extend_from_slice(&0x0746u32.to_le_bytes()); // version
        data.extend_from_slice(&1u32.to_le_bytes()); // chunk_count
        let table_offset_pos = data.len();
        data.extend_from_slice(&0u32.to_le_bytes()); // chunk_table_offset (patched)

        let statobj_offset = data.len() as u32;
        data.extend_from_slice(&0u32.to_le_bytes()); // unknown
        data.extend_from_slice(&0u32.to_le_bytes()); // count = 0
        let statobj_size = (data.len() as u32) - statobj_offset;

        let table_off = data.len() as u32;
        data[table_offset_pos..table_offset_pos + 4]
            .copy_from_slice(&table_off.to_le_bytes());
        // Entry: type, version, id, size, offset
        data.extend_from_slice(&0x0010u16.to_le_bytes());
        data.extend_from_slice(&15u16.to_le_bytes());
        data.extend_from_slice(&0i32.to_le_bytes());
        data.extend_from_slice(&statobj_size.to_le_bytes());
        data.extend_from_slice(&statobj_offset.to_le_bytes());
        data
    }
}
