use crate::database::Database;
use crate::query::compile::CompiledPath;
use crate::query::value::Value;
use crate::types::{Record, StructId};
use std::collections::{HashMap, HashSet};

/// A node in the resolved loadout tree.
#[derive(Debug)]
pub struct LoadoutNode {
    /// The entity's DataCore record.
    pub record: Record,
    /// Entity name (e.g., "AEGS_Gladius_Thruster_Main").
    pub entity_name: String,
    /// Item port name on the parent (e.g., "hardpoint_thruster_main"). Empty for root.
    pub item_port_name: String,
    /// Bone name from the parent's SItemPortContainerComponentParams.
    /// Used to attach at the correct skeleton bone in the parent.
    pub helper_bone_name: Option<String>,
    /// If true, the attached entity should not inherit its parent's rotation.
    /// From SAttachmentConstraintParams.noRotation.
    pub no_rotation: bool,
    /// Item port helper offset position (CryEngine Z-up coordinates).
    pub offset_position: [f32; 3],
    /// Item port helper offset rotation (Euler angles in degrees, CryEngine convention).
    pub offset_rotation: [f32; 3],
    /// Port tags from SItemPortContainerComponentParams (e.g., for SubGeometry variant selection).
    pub port_tags: String,
    /// Geometry file path from SGeometryResourceParams, if present.
    pub geometry_path: Option<String>,
    /// Material file path from SGeometryResourceParams, if present.
    pub material_path: Option<String>,
    /// Child loadout entries (one level for now).
    pub children: Vec<LoadoutNode>,
}

/// A fully resolved loadout tree rooted at a ship/entity.
#[derive(Debug)]
pub struct LoadoutTree {
    pub root: LoadoutNode,
}

/// A geometry variant from SubGeometry[], selected by tag matching.
#[derive(Debug, Clone)]
pub struct SubGeometryVariant {
    pub tag: String,
    pub geometry_path: String,
    pub material_path: String,
}

/// Pre-built index for fast entity lookups and cached compiled query paths.
pub struct EntityIndex<'a> {
    /// Lowercase short-name → record index in `db.records()`.
    name_map: HashMap<String, usize>,
    pub db: &'a Database<'a>,
    /// Cached compiled paths keyed by struct_index → (geom_path, mtl_path).
    geom_paths: std::cell::RefCell<HashMap<i32, Option<(CompiledPath, CompiledPath)>>>,
}

impl<'a> EntityIndex<'a> {
    /// Build the index from a Database. Scans EntityClassDefinition records (O(n)).
    pub fn new(db: &'a Database<'a>) -> Self {
        let mut name_map = HashMap::with_capacity(16384);
        if let Some(si) = db.struct_id("EntityClassDefinition") {
            for r in db.records_of_type(si) {
                // Compute record's index in the global records slice for O(1) retrieval.
                let i = ((r as *const Record as usize) - (db.records().as_ptr() as usize))
                    / std::mem::size_of::<Record>();
                let record_name = db.resolve_string2(r.name_offset);
                let entity_part = record_name.rsplit('.').next().unwrap_or(record_name);
                name_map.insert(entity_part.to_lowercase(), i);
            }
        }
        EntityIndex {
            name_map,
            db,
            geom_paths: std::cell::RefCell::new(HashMap::new()),
        }
    }

    /// O(1) entity record lookup by short name (case-insensitive).
    pub fn find_record(&self, name: &str) -> Option<&'a Record> {
        self.name_map
            .get(&name.to_lowercase())
            .map(|&i| &self.db.records()[i])
    }

    /// Query geometry and material paths, caching the compiled paths per struct_index.
    pub fn query_geometry_paths(&self, record: &Record) -> (Option<String>, Option<String>) {
        let si = record.struct_index;
        let sid = StructId(si);
        let mut cache = self.geom_paths.borrow_mut();
        let compiled = cache.entry(si).or_insert_with(|| {
            let geom = self
                .db
                .compile_path::<String>(
                    sid,
                    "Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path",
                )
                .ok();
            let mtl = self
                .db
                .compile_path::<String>(
                    sid,
                    "Components[SGeometryResourceParams].Geometry.Geometry.Material.path",
                )
                .ok();
            match (geom, mtl) {
                (Some(g), Some(m)) => Some((g, m)),
                _ => None,
            }
        });
        let (geom, mtl) = match compiled {
            Some((g, m)) => (
                self.db
                    .query_single::<String>(g, record)
                    .ok()
                    .flatten()
                    .filter(|p| !p.is_empty()),
                self.db
                    .query_single::<String>(m, record)
                    .ok()
                    .flatten()
                    .filter(|p| !p.is_empty()),
            ),
            None => (None, None),
        };
        (geom, mtl)
    }

    /// Query SubGeometry variants from an entity's SGeometryResourceParams.
    /// Returns tagged geometry variants (e.g., "right" -> Right.cga, "left" -> Left.cga).
    pub fn query_sub_geometry(&self, record: &Record) -> Vec<SubGeometryVariant> {
        // Query the full SGeometryResourceParams component, then walk to SubGeometry.
        // Querying just ".Geometry" may truncate the SubGeometry array.
        let compiled = match self.db.compile_path::<Value>(
            record.struct_id(),
            "Components[SGeometryResourceParams]",
        ) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let components = match self.db.query::<Value>(&compiled, record) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let mut variants = Vec::new();
        for component in &components {
            let geom_node = match get_object_field(component, "Geometry") {
                Some(g) => g,
                None => {
                    log::debug!("    no Geometry field in component");
                    continue;
                }
            };
            let sub_arr = get_array_field(geom_node, "SubGeometry");
            if let Some(sub_geom_arr) = sub_arr {
                for sub in sub_geom_arr {
                    collect_sub_geometry_variants(sub, &[], &mut variants);
                }
            }
        }
        variants
    }
}

/// Extract a string field from a Value::Object.
fn get_string_field<'v, 'a>(val: &'v Value<'a>, name: &str) -> Option<&'a str> {
    if let Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name
                && let Value::String(s) = v
            {
                return Some(s);
            }
        }
    }
    None
}

/// Extract an array field from a Value::Object.
fn get_array_field<'v, 'a>(val: &'v Value<'a>, name: &str) -> Option<&'v Vec<Value<'a>>> {
    if let Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name
                && let Value::Array(arr) = v
            {
                return Some(arr);
            }
        }
    }
    None
}

/// Extract an object field from a Value::Object.
fn get_object_field<'v, 'a>(val: &'v Value<'a>, name: &str) -> Option<&'v Value<'a>> {
    if let Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name
                && let Value::Object { .. } = v
            {
                return Some(v);
            }
        }
    }
    None
}

fn get_f32_field(val: &Value, name: &str) -> Option<f32> {
    if let Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name {
                match v {
                    Value::Float(f) => return Some(*f),
                    Value::Double(f) => return Some(*f as f32),
                    Value::Int32(i) => return Some(*i as f32),
                    _ => {}
                }
            }
        }
    }
    None
}

fn get_bool_field(val: &Value, name: &str) -> Option<bool> {
    if let Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name {
                if let Value::Bool(b) = v {
                    return Some(*b);
                }
            }
        }
    }
    None
}

/// Extract a Guid field from a Value::Object (used for truncated References).
fn get_guid_field(val: &Value, name: &str) -> Option<crate::types::CigGuid> {
    if let Value::Object { fields, .. } = val {
        for (k, v) in fields {
            if *k == name
                && let Value::Guid(g) = v
            {
                return Some(*g);
            }
        }
    }
    None
}

/// Extract the source record_id from a Value::Object (set when materialized from a Reference).
fn get_record_id(val: &Value) -> Option<crate::types::CigGuid> {
    if let Value::Object { record_id, .. } = val {
        *record_id
    } else {
        None
    }
}

/// Per-port metadata extracted from SItemPortContainerComponentParams.
struct PortInfo {
    bone_name: Option<String>,
    no_rotation: bool,
    offset_position: [f32; 3],
    offset_rotation: [f32; 3],
    port_tags: String,
}

/// Build a mapping from item port name → port metadata.
/// Walks: Components[SItemPortContainerComponentParams].Ports[]
fn build_port_info_map(db: &Database, record: &Record) -> HashMap<String, PortInfo> {
    let mut map = HashMap::new();

    let compiled = match db.compile_path::<Value>(
        record.struct_id(),
        "Components[SItemPortContainerComponentParams]",
    ) {
        Ok(c) => c,
        Err(_) => return map,
    };

    let components = match db.query::<Value>(&compiled, record) {
        Ok(c) => c,
        Err(_) => return map,
    };

    for component in &components {
        if let Some(ports) = get_array_field(component, "Ports") {
            for port in ports {
                let port_name = match get_string_field(port, "Name") {
                    Some(n) if !n.is_empty() => n,
                    _ => continue,
                };

                let attach_impl = get_object_field(port, "AttachmentImplementation");

                // Walk: AttachmentImplementation.Helper.Helper.Name
                let bone_name = attach_impl
                    .and_then(|ai| get_object_field(ai, "Helper"))
                    .and_then(|hn| get_object_field(hn, "Helper"))
                    .and_then(|h| get_string_field(h, "Name"))
                    .filter(|n| !n.is_empty())
                    .map(|s| s.to_string());

                // Walk: AttachmentImplementation.constraintParams.noRotation
                let no_rotation = attach_impl
                    .and_then(|ai| get_object_field(ai, "constraintParams"))
                    .and_then(|cp| get_bool_field(cp, "noRotation"))
                    .unwrap_or(false);

                // Walk: AttachmentImplementation.Helper.Helper.Offset.Position / Rotation
                let offset_obj = attach_impl
                    .and_then(|ai| get_object_field(ai, "Helper"))
                    .and_then(|hn| get_object_field(hn, "Helper"))
                    .and_then(|h| get_object_field(h, "Offset"));

                let offset_position = offset_obj
                    .and_then(|o| get_object_field(o, "Position"))
                    .map(|p| [
                        get_f32_field(p, "x").unwrap_or(0.0),
                        get_f32_field(p, "y").unwrap_or(0.0),
                        get_f32_field(p, "z").unwrap_or(0.0),
                    ])
                    .unwrap_or([0.0; 3]);

                let offset_rotation = offset_obj
                    .and_then(|o| get_object_field(o, "Rotation"))
                    .map(|r| [
                        get_f32_field(r, "x").unwrap_or(0.0),
                        get_f32_field(r, "y").unwrap_or(0.0),
                        get_f32_field(r, "z").unwrap_or(0.0),
                    ])
                    .unwrap_or([0.0; 3]);

                let port_tags = get_string_field(port, "PortTags")
                    .unwrap_or("")
                    .to_string();

                map.insert(port_name.to_string(), PortInfo {
                    bone_name,
                    no_rotation,
                    offset_position,
                    offset_rotation,
                    port_tags,
                });
            }
        }
    }

    map
}

/// Post-pass: set helper_bone_name and no_rotation on loadout children from parent's item port data.
fn apply_port_info(db: &Database, parent_record: &Record, children: &mut [LoadoutNode]) {
    let port_map = build_port_info_map(db, parent_record);
    for child in children {
        if let Some(info) = port_map.get(&child.item_port_name) {
            child.helper_bone_name = info.bone_name.clone();
            child.no_rotation = info.no_rotation;
            child.offset_position = info.offset_position;
            child.offset_rotation = info.offset_rotation;
            child.port_tags = info.port_tags.clone();
            if info.offset_position != [0.0; 3] || info.offset_rotation != [0.0; 3] {
                log::debug!(
                    "  port '{}': offset pos=[{:.2},{:.2},{:.2}] rot=[{:.1},{:.1},{:.1}]",
                    child.item_port_name,
                    info.offset_position[0], info.offset_position[1], info.offset_position[2],
                    info.offset_rotation[0], info.offset_rotation[1], info.offset_rotation[2],
                );
            }
        }
        // Recurse for grandchildren
        apply_port_info(db, &child.record, &mut child.children);
    }
}

/// Post-pass: override geometry_path on children that have SubGeometry variants.
/// Matches port_tags or item_port_name against SubGeometry tags.
fn resolve_sub_geometry(idx: &EntityIndex, children: &mut [LoadoutNode]) {
    // First pass: direct tag matching for each child.
    let mut unmatched: Vec<usize> = Vec::new();
    for (i, child) in children.iter_mut().enumerate() {
        if child.geometry_path.is_none() {
            continue;
        }
        let variants = idx.query_sub_geometry(&child.record);
        if variants.is_empty() {
            continue;
        }
        if let Some(m) = match_sub_geometry_variant(&variants, &child.port_tags, child.helper_bone_name.as_deref(), &child.item_port_name) {
            log::debug!("  SubGeometry: {} tag='{}' -> {}", child.entity_name, m.tag, m.geometry_path);
            child.geometry_path = Some(m.geometry_path.clone());
            if !m.material_path.is_empty() { child.material_path = Some(m.material_path.clone()); }
        } else if variants.len() > 1 {
            if has_multiple_geometry_variants(&variants) {
                log::warn!(
                    "  SubGeometry: {} has {} variants but none matched port='{}' bone='{}' tags='{}'. Variants: [{}]",
                    child.entity_name, variants.len(), child.item_port_name,
                    child.helper_bone_name.as_deref().unwrap_or(""),
                    child.port_tags,
                    variants.iter().map(|v| format!("{}:{}", v.tag, v.geometry_path)).collect::<Vec<_>>().join(", ")
                );
                unmatched.push(i);
            } else {
                log::debug!(
                    "  SubGeometry: {} has {} material-only variants with no selector match; keeping default geometry {}",
                    child.entity_name,
                    variants.len(),
                    child.geometry_path.as_deref().unwrap_or("")
                );
            }
        }
    }

    // Second pass: for unmatched children, assign the variant not claimed by siblings.
    for &ui in &unmatched {
        let variants = idx.query_sub_geometry(&children[ui].record);
        let entity = children[ui].entity_name.clone();
        // Collect geometry paths already assigned to siblings with the same entity
        let claimed: Vec<String> = children.iter()
            .enumerate()
            .filter(|(j, c)| *j != ui && c.entity_name == entity)
            .filter_map(|(_, c)| c.geometry_path.clone())
            .collect();
        if let Some(v) = variants.iter().find(|v| !claimed.contains(&v.geometry_path)) {
            log::info!("  SubGeometry: {} tag='{}' (fallback, unclaimed) -> {}", entity, v.tag, v.geometry_path);
            children[ui].geometry_path = Some(v.geometry_path.clone());
            if !v.material_path.is_empty() { children[ui].material_path = Some(v.material_path.clone()); }
        }
    }

    // Recurse for grandchildren
    for child in children.iter_mut() {
        resolve_sub_geometry(idx, &mut child.children);
    }
}

fn has_multiple_geometry_variants(variants: &[SubGeometryVariant]) -> bool {
    let mut geometry_paths = HashSet::new();
    for variant in variants {
        geometry_paths.insert(variant.geometry_path.to_ascii_lowercase());
        if geometry_paths.len() > 1 {
            return true;
        }
    }
    false
}

/// Match SubGeometry variants against port tags, helper bone name, or item port name.
/// Priority: 1) port_tags, 2) helper_bone_name, 3) item_port_name.
/// This mirrors scdatatools' behavior where the helper (bone) name is the primary
/// matching field for SubGeometry variant selection.
fn match_sub_geometry_variant<'a>(
    variants: &'a [SubGeometryVariant],
    port_tags: &str,
    helper_bone_name: Option<&str>,
    item_port_name: &str,
) -> Option<&'a SubGeometryVariant> {
    // 1. Try match against port_tags
    if !port_tags.is_empty()
        && let Some(variant) = best_sub_geometry_variant_match(variants, port_tags)
    {
        return Some(variant);
    }

    // 2. Try SubGeometry tag as substring of helper bone name
    //    (scdatatools' primary matching strategy — the bone name often contains
    //     variant-specific identifiers like "left", "right", "center")
    if let Some(bone) = helper_bone_name
        && let Some(variant) = best_sub_geometry_variant_match(variants, bone)
    {
        return Some(variant);
    }

    // 3. Try SubGeometry tag as substring of item_port_name
    best_sub_geometry_variant_match(variants, item_port_name)
}

fn collect_sub_geometry_variants(
    node: &Value,
    parent_tags: &[String],
    variants: &mut Vec<SubGeometryVariant>,
) {
    let mut combined_tags = parent_tags.to_vec();
    if let Some(tag) = get_string_field(node, "Tags") {
        append_selector_tokens(&mut combined_tags, tag);
    }

    let (geom, mtl) = extract_sub_geometry_paths(node);
    if let Some(geometry_path) = geom {
        let tag = combined_tags.join(" ");
        if !tag.is_empty()
            && !variants.iter().any(|variant| {
                variant.tag.eq_ignore_ascii_case(&tag)
                    && variant.geometry_path.eq_ignore_ascii_case(&geometry_path)
            })
        {
            variants.push(SubGeometryVariant {
                tag,
                geometry_path,
                material_path: mtl.unwrap_or_default(),
            });
        }
    }

    if let Some(children) = get_array_field(node, "SubGeometry") {
        for child in children {
            collect_sub_geometry_variants(child, &combined_tags, variants);
        }
    }
}

fn append_selector_tokens(tokens: &mut Vec<String>, raw: &str) {
    for token in selector_tokens(raw) {
        if !tokens.iter().any(|existing| existing == &token) {
            tokens.push(token);
        }
    }
}

fn selector_tokens(raw: &str) -> Vec<String> {
    raw.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn best_sub_geometry_variant_match<'a>(
    variants: &'a [SubGeometryVariant],
    selector: &str,
) -> Option<&'a SubGeometryVariant> {
    let selector_token_set: HashSet<String> = selector_tokens(selector).into_iter().collect();
    if selector_token_set.is_empty() {
        return None;
    }

    variants
        .iter()
        .filter_map(|variant| {
            let variant_tokens = selector_tokens(&variant.tag);
            if variant_tokens.is_empty()
                || !variant_tokens
                    .iter()
                    .all(|token| selector_token_set.contains(token))
            {
                return None;
            }

            Some((variant_tokens.len(), variant.tag.len(), variant))
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)))
        .map(|(_, _, variant)| variant)
}

/// Resolve the full loadout tree for an entity, recursively walking
/// `SEntityComponentDefaultLoadoutParams.loadout.entries[]` on each child.
///
/// Never fails — missing children or missing geometry are logged and skipped.
pub fn resolve_loadout(db: &Database, record: &Record) -> LoadoutTree {
    let idx = EntityIndex::new(db);
    resolve_loadout_indexed(&idx, record)
}

/// Resolve loadout using a pre-built EntityIndex (avoids rebuilding it per call).
pub fn resolve_loadout_indexed(idx: &EntityIndex, record: &Record) -> LoadoutTree {
    let db = idx.db;
    let entity_name = db.resolve_string2(record.name_offset).to_string();
    let (geometry_path, material_path) = idx.query_geometry_paths(record);

    // Query the full loadout component once — the Value includes nested loadout
    // entries, so we walk the Value tree instead of making recursive queries.
    let children = if let Ok(compiled) = db.compile_path::<Value>(
        record.struct_id(),
        "Components[SEntityComponentDefaultLoadoutParams]",
    ) {
        if let Ok(components) = db.query_no_references(&compiled, record) {
            let mut result = Vec::new();
            for component in &components {
                if let Some(loadout) = get_object_field(component, "loadout") {
                    collect_entries_recursive(idx, loadout, &mut result, 0);
                }
            }
            result
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // Resolve helper bone names from parent item port definitions
    let mut children = children;
    apply_port_info(idx.db, record, &mut children);
    resolve_sub_geometry(idx, &mut children);

    fn count_all(nodes: &[LoadoutNode]) -> (usize, usize) {
        let mut total = 0;
        let mut with_geom = 0;
        for n in nodes {
            total += 1;
            if n.geometry_path.is_some() {
                with_geom += 1;
            }
            let (t, g) = count_all(&n.children);
            total += t;
            with_geom += g;
        }
        (total, with_geom)
    }
    let (total, with_geom) = count_all(&children);
    log::info!("Loadout: {total} total entries ({with_geom} with geometry)");

    LoadoutTree {
        root: LoadoutNode {
            record: *record,
            entity_name,
            item_port_name: String::new(),
            helper_bone_name: None,
            no_rotation: false,
            offset_position: [0.0; 3],
            offset_rotation: [0.0; 3],
            port_tags: String::new(),
            geometry_path,
            material_path,
            children,
        },
    }
}

const MAX_LOADOUT_DEPTH: usize = 4;

/// Extract a geometry file path from an inline EntityClassDefinition Value.
/// Walks: Components[] → SGeometryResourceParams → Geometry.Geometry.Geometry.path
fn extract_geometry_from_value(entity_val: &Value) -> Option<String> {
    let components = get_array_field(entity_val, "Components")?;
    for comp in components {
        if let Value::Object { type_name, .. } = comp
            && *type_name == "SGeometryResourceParams"
        {
            let geom = get_object_field(comp, "Geometry")?;
            let geom2 = get_object_field(geom, "Geometry")?;
            let geom3 = get_object_field(geom2, "Geometry")?;
            return get_string_field(geom3, "path")
                .filter(|p| !p.is_empty())
                .map(|s| s.to_owned());
        }
    }
    None
}

/// Extract a material file path from an inline EntityClassDefinition Value.
fn extract_material_from_value(entity_val: &Value) -> Option<String> {
    let components = get_array_field(entity_val, "Components")?;
    for comp in components {
        if let Value::Object { type_name, .. } = comp
            && *type_name == "SGeometryResourceParams"
        {
            let geom = get_object_field(comp, "Geometry")?;
            let geom2 = get_object_field(geom, "Geometry")?;
            let mat = get_object_field(geom2, "Material")?;
            return get_string_field(mat, "path")
                .filter(|p| !p.is_empty())
                .map(|s| s.to_owned());
        }
    }
    None
}

/// Extract geometry and material paths from a SubGeometry SGeometryNodeParams Value.
/// Walks: Geometry.Geometry.Geometry.path and Geometry.Geometry.Material.path
fn extract_sub_geometry_paths(node_params: &Value) -> (Option<String>, Option<String>) {
    // SGeometryNodeParams.Geometry = SGeometryDataParams
    // SGeometryDataParams.Geometry = GlobalResourceGeometry { path }
    // SGeometryDataParams.Material = GlobalResourceMaterial { path }
    let geom_data = get_object_field(node_params, "Geometry"); // SGeometryDataParams
    let geom = geom_data
        .and_then(|gd| get_object_field(gd, "Geometry"))      // GlobalResourceGeometry
        .and_then(|g| get_string_field(g, "path"))
        .filter(|p| !p.is_empty())
        .map(|s| s.to_owned());
    let mtl = geom_data
        .and_then(|gd| get_object_field(gd, "Material"))      // GlobalResourceMaterial
        .and_then(|m| get_string_field(m, "path"))
        .filter(|p| !p.is_empty())
        .map(|s| s.to_owned());
    (geom, mtl)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(value: &'static str) -> Value<'static> {
        Value::String(value)
    }

    fn object(type_name: &'static str, fields: Vec<(&'static str, Value<'static>)>) -> Value<'static> {
        Value::Object {
            type_name,
            fields,
            record_id: None,
        }
    }

    fn geometry_node(
        tag: &'static str,
        geometry_path: &'static str,
        children: Vec<Value<'static>>,
    ) -> Value<'static> {
        object(
            "SGeometryNodeParams",
            vec![
                (
                    "Geometry",
                    object(
                        "SGeometryDataParams",
                        vec![
                            (
                                "Geometry",
                                object(
                                    "GlobalResourceGeometry",
                                    vec![("path", string(geometry_path))],
                                ),
                            ),
                            (
                                "Material",
                                object("GlobalResourceMaterial", vec![("path", string(""))]),
                            ),
                        ],
                    ),
                ),
                ("SubGeometry", Value::Array(children)),
                ("Tags", string(tag)),
            ],
        )
    }

    #[test]
    fn collect_sub_geometry_variants_flattens_nested_tags() {
        let front = geometry_node(
            "front",
            "Objects/Ships/Vulture/Front_Left.cga",
            vec![
                geometry_node("left", "Objects/Ships/Vulture/Front_Left.cga", Vec::new()),
                geometry_node("right", "Objects/Ships/Vulture/Front_Right.cga", Vec::new()),
            ],
        );

        let mut variants = Vec::new();
        collect_sub_geometry_variants(&front, &[], &mut variants);

        assert!(variants.iter().any(|variant| {
            variant.tag == "front left"
                && variant.geometry_path == "Objects/Ships/Vulture/Front_Left.cga"
        }));
        assert!(variants.iter().any(|variant| {
            variant.tag == "front right"
                && variant.geometry_path == "Objects/Ships/Vulture/Front_Right.cga"
        }));
    }

    #[test]
    fn match_sub_geometry_variant_prefers_most_specific_selector() {
        let variants = vec![
            SubGeometryVariant {
                tag: "front".to_string(),
                geometry_path: "Objects/Ships/Vulture/Front_Left.cga".to_string(),
                material_path: String::new(),
            },
            SubGeometryVariant {
                tag: "front left".to_string(),
                geometry_path: "Objects/Ships/Vulture/Front_Left.cga".to_string(),
                material_path: String::new(),
            },
            SubGeometryVariant {
                tag: "front right".to_string(),
                geometry_path: "Objects/Ships/Vulture/Front_Right.cga".to_string(),
                material_path: String::new(),
            },
            SubGeometryVariant {
                tag: "rear".to_string(),
                geometry_path: "Objects/Ships/Vulture/Rear.cga".to_string(),
                material_path: String::new(),
            },
        ];

        let matched = match_sub_geometry_variant(
            &variants,
            "",
            None,
            "hardpoint_thruster_vtol_front_right",
        )
        .expect("front-right hardpoint should match nested right variant");

        assert_eq!(matched.tag, "front right");
        assert_eq!(
            matched.geometry_path,
            "Objects/Ships/Vulture/Front_Right.cga"
        );
    }

    #[test]
    fn has_multiple_geometry_variants_ignores_material_only_differences() {
        let variants = vec![
            SubGeometryVariant {
                tag: "brown01".to_string(),
                geometry_path: "Objects/fps_weapons/gadgets/grin/multitool/Cutter/gdgt_fps_grin_multitool_cutter.cgf".to_string(),
                material_path: "Objects/fps_weapons/gadgets/grin/multitool/Cutter/gdgt_fps_grin_multitool_cutter_mirage_mat.mtl".to_string(),
            },
            SubGeometryVariant {
                tag: "red01".to_string(),
                geometry_path: "Objects/fps_weapons/gadgets/grin/multitool/Cutter/gdgt_fps_grin_multitool_cutter.cgf".to_string(),
                material_path: "Objects/fps_weapons/gadgets/grin/multitool/Cutter/gdgt_fps_grin_multitool_cutter_bloodline_mat.mtl".to_string(),
            },
        ];

        assert!(!has_multiple_geometry_variants(&variants));
    }
}

/// Extract the loadout from an inline EntityClassDefinition Value.
/// Walks: Components[] → SEntityComponentDefaultLoadoutParams → loadout
fn extract_loadout_from_value<'v, 'a>(entity_val: &'v Value<'a>) -> Option<&'v Value<'a>> {
    let components = get_array_field(entity_val, "Components")?;
    for comp in components {
        if let Value::Object { type_name, .. } = comp
            && *type_name == "SEntityComponentDefaultLoadoutParams"
        {
            return get_object_field(comp, "loadout");
        }
    }
    None
}

/// Walk a loadout Value tree recursively, collecting LoadoutNodes.
/// The `loadout_value` is a SItemPortLoadoutManualParams with an `entries` array.
/// Each entry may have its own nested `loadout` field, and may reference child
/// entities via either `entityClassName` (string) or `entityClassReference` (inline record).
fn collect_entries_recursive(
    idx: &EntityIndex,
    loadout_value: &Value,
    out: &mut Vec<LoadoutNode>,
    depth: usize,
) {
    if depth >= MAX_LOADOUT_DEPTH {
        return;
    }

    let entries = match get_array_field(loadout_value, "entries") {
        Some(e) => e,
        None => return,
    };

    let indent = "  ".repeat(depth + 1);

    for entry in entries {
        let port_name = get_string_field(entry, "itemPortName").unwrap_or("");
        let class_name = get_string_field(entry, "entityClassName").unwrap_or("");

        // Recurse into the entry's own nested loadout (from the parent's Value tree)
        let mut grandchildren = Vec::new();
        if let Some(nested_loadout) = get_object_field(entry, "loadout") {
            collect_entries_recursive(idx, nested_loadout, &mut grandchildren, depth + 1);
        }

        if !class_name.is_empty() {
            // Path A: entityClassName — O(1) lookup via EntityIndex
            let child = if let Some(child_record) = idx.find_record(class_name) {
                let (child_geom, child_mtl) = idx.query_geometry_paths(child_record);
                let has_geom = child_geom.is_some();
                log::debug!(
                    "{indent}loadout: {port_name} -> {class_name} ({})",
                    if has_geom {
                        "has geometry"
                    } else {
                        "no geometry"
                    }
                );
                LoadoutNode {
                    record: *child_record,
                    entity_name: class_name.to_owned(),
                    item_port_name: port_name.to_owned(),
                    helper_bone_name: None,
            no_rotation: false,
            offset_position: [0.0; 3],
            offset_rotation: [0.0; 3],
            port_tags: String::new(),
                    geometry_path: child_geom,
                    material_path: child_mtl,
                    children: grandchildren,
                }
            } else {
                log::warn!("{indent}loadout: {port_name} -> {class_name} (entity not found in EntityIndex)");
                continue;
            };
            out.push(child);
        } else if let Some(ref_val) = get_object_field(entry, "entityClassReference") {
            // Path B: entityClassReference materialized as Object (may be shallow).
            // Try extracting geometry from Value; fall back to targeted record query.
            let mut ref_geom = extract_geometry_from_value(ref_val);
            let mut ref_mtl = extract_material_from_value(ref_val);

            // Also walk the referenced entity's own loadout for further children
            if let Some(ref_loadout) = extract_loadout_from_value(ref_val) {
                collect_entries_recursive(idx, ref_loadout, &mut grandchildren, depth + 1);
            }

            // Resolve the entity record. Prefer the record_id stamped by the
            // materializer (always correct), fall back to className name lookup.
            let record_from_id = get_record_id(ref_val)
                .and_then(|guid| idx.db.record_by_id(&guid));

            let ref_name = if let Some(rec) = record_from_id {
                // Use the actual record name from the DataCore
                let full_name = idx.db.resolve_string2(rec.name_offset);
                full_name.rsplit('.').next().unwrap_or(full_name).to_owned()
            } else {
                get_string_field(ref_val, "className")
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_owned())
                    .or_else(|| {
                        ref_geom.as_ref().map(|p| {
                            p.rsplit(['/', '\\'])
                                .next()
                                .unwrap_or(p)
                                .split('.')
                                .next()
                                .unwrap_or(p)
                                .to_owned()
                        })
                    })
                    .unwrap_or_else(|| format!("<ref@{port_name}>"))
            };

            // Use record_id record first, then try name lookup as fallback
            let record = record_from_id.or_else(|| idx.find_record(&ref_name));

            // If geometry or material wasn't in the Value (truncated by depth
            // limit), try a targeted record query via EntityIndex.
            if ref_geom.is_none() || ref_mtl.is_none() {
                if let Some(rec) = record {
                    let (g, m) = idx.query_geometry_paths(rec);
                    if ref_geom.is_none() {
                        ref_geom = g;
                    }
                    if ref_mtl.is_none() {
                        ref_mtl = m;
                    }
                }
            }

            let has_geom = ref_geom.is_some();
            log::debug!(
                "{indent}loadout: {port_name} -> {ref_name} (ref, {})",
                if has_geom {
                    "has geometry"
                } else {
                    "no geometry"
                }
            );

            if let Some(rec) = record {
                out.push(LoadoutNode {
                    record: *rec,
                    entity_name: ref_name,
                    item_port_name: port_name.to_owned(),
                    helper_bone_name: None,
                    no_rotation: false,
                    offset_position: [0.0; 3],
                    offset_rotation: [0.0; 3],
                    port_tags: String::new(),
                    geometry_path: ref_geom,
                    material_path: ref_mtl,
                    children: grandchildren,
                });
            } else if has_geom {
                // No record found but we have geometry from the reference —
                // use a dummy record (geometry path is what matters for export)
                log::debug!("{indent}  (no record for '{ref_name}', using ref geometry)");
                out.push(LoadoutNode {
                    record: match idx.db.records().first() {
                        Some(r) => *r,
                        None => continue, // skip node if DB has no records
                    },
                    entity_name: ref_name,
                    item_port_name: port_name.to_owned(),
                    helper_bone_name: None,
                    no_rotation: false,
                    offset_position: [0.0; 3],
                    offset_rotation: [0.0; 3],
                    port_tags: String::new(),
                    geometry_path: ref_geom,
                    material_path: ref_mtl,
                    children: grandchildren,
                });
            }
        } else if let Some(guid) = get_guid_field(entry, "entityClassReference") {
            // Path C: entityClassReference truncated to Guid by depth limit.
            // Look up the referenced record, query geometry, and resolve its loadout.
            if let Some(ref_record) = idx.db.record_by_id(&guid) {
                let ref_name = idx.db.resolve_string2(ref_record.name_offset);
                let short_name = ref_name.rsplit('.').next().unwrap_or(ref_name).to_string();
                let (ref_geom, ref_mtl) = idx.query_geometry_paths(ref_record);

                // Also resolve the referenced entity's OWN loadout for nested children.
                // Each child entity's loadout is small, so this Value query is cheap.
                if let Ok(compiled) = idx.db.compile_path::<Value>(
                    ref_record.struct_id(),
                    "Components[SEntityComponentDefaultLoadoutParams]",
                ) && let Ok(components) = idx.db.query_no_references(&compiled, ref_record)
                {
                    for component in &components {
                        if let Some(loadout) = get_object_field(component, "loadout") {
                            collect_entries_recursive(idx, loadout, &mut grandchildren, depth + 1);
                        }
                    }
                }

                let has_geom = ref_geom.is_some();
                log::debug!(
                    "{indent}loadout: {port_name} -> {short_name} (ref/guid, {})",
                    if has_geom {
                        "has geometry"
                    } else {
                        "no geometry"
                    }
                );
                out.push(LoadoutNode {
                    record: *ref_record,
                    entity_name: short_name,
                    item_port_name: port_name.to_owned(),
                    helper_bone_name: None,
            no_rotation: false,
            offset_position: [0.0; 3],
            offset_rotation: [0.0; 3],
            port_tags: String::new(),
                    geometry_path: ref_geom,
                    material_path: ref_mtl,
                    children: grandchildren,
                });
            }
        }
        // else: both entityClassName and entityClassReference are empty — skip
    }
}
