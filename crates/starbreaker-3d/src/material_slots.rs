//! Which material each submesh draws with — the engine's slot rule.
//!
//! A submesh names a material slot (`SubMesh::material_id`) of its `.mtl`.
//! The engine resolves it like CryEngine's `CMatInfo::GetSafeSubMtl`:
//!
//! - a material that is not a multi-material (no `<SubMaterials>`: the
//!   engine sets `MTL_FLAG_MULTI_SUBMTL` from their count) serves every
//!   slot;
//! - a multi-material serves the slots it has; a slot past its
//!   submaterials (a mesh compiled against an older `.mtl`) draws with the
//!   engine's default material (`Materials/material_default.mtl`) — it is
//!   drawn, not dropped.
//!
//! [`resolve_material_slots`] rewrites a mesh's slots to that rule (the
//! default material appended to the set when a slot needs it), so every
//! writer sees only valid slots and draws the same submeshes.
//! [`DEFAULT_MATERIAL_PATH`] is the engine default's P4k path.

use crate::error::Error;
use crate::mtl::{MtlFile, SubMaterial};
use crate::types::Mesh;

/// P4k path of the engine's default material.
pub const DEFAULT_MATERIAL_PATH: &str = r"Data\Materials\material_default.mtl";

/// Point every submesh of `mesh` at the material the engine draws it with
/// (see the module docs). `default_material` loads the engine default; it
/// is only called when a slot is past a multi-material's submaterials.
pub fn resolve_material_slots(
    mesh: &mut Mesh,
    materials: &mut MtlFile,
    default_material: impl FnOnce() -> Result<SubMaterial, Error>,
) -> Result<(), Error> {
    if !materials.material_set.sub_materials {
        for submesh in &mut mesh.submeshes {
            submesh.material_id = 0;
        }
        return Ok(());
    }
    let count = materials.materials.len() as u32;
    if mesh.submeshes.iter().all(|s| s.material_id < count) {
        return Ok(());
    }
    materials.materials.push(default_material()?);
    for submesh in mesh.submeshes.iter_mut().filter(|s| s.material_id >= count) {
        log::debug!(
            "submesh slot {} past the {count} submaterials of {}: engine default material",
            submesh.material_id,
            materials.source_path.as_deref().unwrap_or("<unknown material>"),
        );
        submesh.material_id = count;
        submesh.material_name = None;
    }
    Ok(())
}

/// The engine default material from `p4k`, named after its file.
pub(crate) fn load_default_material(p4k: &starbreaker_p4k::MappedP4k) -> Result<SubMaterial, Error> {
    let bytes = p4k
        .read_file(DEFAULT_MATERIAL_PATH)
        .map_err(|_| Error::FileNotFoundInP4k { path: DEFAULT_MATERIAL_PATH.to_owned() })?;
    let mut file = crate::mtl::parse_mtl(&bytes)?;
    let mut material = file.materials.remove(0);
    if material.name.is_empty() {
        material.name = "material_default".to_owned();
    }
    Ok(material)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mtl::MaterialSetAuthoredData;
    use crate::types::SubMesh;

    fn material(name: &str) -> SubMaterial {
        SubMaterial {
            name: name.to_owned(),
            shader: "Illum".to_owned(),
            diffuse: [1.0; 3],
            opacity: 1.0,
            alpha_test: 0.0,
            string_gen_mask: String::new(),
            is_nodraw: false,
            specular: [0.04; 3],
            shininess: 128.0,
            emissive: [0.0; 3],
            glow: 0.0,
            surface_type: String::new(),
            diffuse_tex: None,
            normal_tex: None,
            layers: Vec::new(),
            palette_tint: 0,
            texture_slots: Vec::new(),
            public_params: Vec::new(),
            authored_attributes: Vec::new(),
            authored_textures: Vec::new(),
            authored_child_blocks: Vec::new(),
        }
    }

    fn mtl(names: &[&str], sub_materials: bool) -> MtlFile {
        MtlFile {
            materials: names.iter().map(|n| material(n)).collect(),
            source_path: None,
            paint_override: None,
            material_set: MaterialSetAuthoredData { sub_materials, ..Default::default() },
        }
    }

    fn mesh(slots: &[u32]) -> Mesh {
        let mut mesh = Mesh::empty();
        mesh.submeshes = slots
            .iter()
            .map(|&material_id| SubMesh {
                material_name: Some(format!("slot{material_id}")),
                material_id,
                source_material_id: None,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            })
            .collect();
        mesh
    }

    fn slots(mesh: &Mesh) -> Vec<u32> {
        mesh.submeshes.iter().map(|s| s.material_id).collect()
    }

    /// A slot past a multi-material's submaterials draws with the engine
    /// default, appended once; valid slots are untouched.
    #[test]
    fn slots_past_a_multi_material_draw_with_the_default() {
        let mut m = mesh(&[0, 1, 2, 5]);
        let mut materials = mtl(&["a", "b"], true);
        resolve_material_slots(&mut m, &mut materials, || Ok(material("material_default"))).unwrap();
        assert_eq!(slots(&m), [0, 1, 2, 2]);
        assert_eq!(materials.materials.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["a", "b", "material_default"]);
        assert_eq!(m.submeshes[0].material_name.as_deref(), Some("slot0"));
    }

    #[test]
    fn valid_slots_need_no_default() {
        let mut m = mesh(&[0, 1]);
        let mut materials = mtl(&["a", "b"], true);
        resolve_material_slots(&mut m, &mut materials, || panic!("not needed")).unwrap();
        assert_eq!(slots(&m), [0, 1]);
        assert_eq!(materials.materials.len(), 2);
    }

    /// A single material (no multi flag) serves every slot.
    #[test]
    fn a_single_material_serves_every_slot() {
        let mut m = mesh(&[0, 3]);
        let mut materials = mtl(&["only"], false);
        resolve_material_slots(&mut m, &mut materials, || panic!("not needed")).unwrap();
        assert_eq!(slots(&m), [0, 0]);
        assert_eq!(materials.materials.len(), 1);
    }
}
