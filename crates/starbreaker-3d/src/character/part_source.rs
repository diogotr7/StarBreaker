//! A character part as the entity pipeline loads it, and its reshape to the
//! character: the one part-loading path behind both the character GLB
//! ([`super::shared::CharacterRig::to_glb`]) and the decomposed package
//! ([`super::package`]).
//!
//! - [`PartSource::load`]: a part's LOD0 `.skin` through
//!   `pipeline::entity_export::load_single_mesh` — every vertex stream the
//!   file has (colours, both UV sets), per-submesh material ids, the part's
//!   `.mtl` and NMC. Loaded once (with the rig assets / head items) and
//!   shared by every render.
//! - [`PartSource::reshaped`]: the pipeline mesh with the positions, normals
//!   and skin of the character's shaped copy of the same part (a
//!   [`HeadMesh`] from the rig). The two share the `.skinm` vertex stream, so
//!   the other streams carry over as they are.
//! - [`PartSource::zones`]: each submesh's clothing zone (see
//!   [`super::clothing`]), read from the `.skinm`'s descriptors and
//!   extra-word table ([`submesh_zones`]); [`PartSource::visible_submeshes`]
//!   drops the zones other worn items cover.
//! - [`geometry_path`]: `Data\Objects\…` → `Objects/…`.

use starbreaker_p4k::MappedP4k;

use super::CharacterError;
use super::head_skin::HeadMesh;
use super::shared;
use crate::pipeline::entity_export::load_single_mesh;
use crate::pipeline::{ExportOptions, MaterialMode, PngCache};
use std::collections::HashSet;

use starbreaker_chunks::ChunkFile;

use crate::ivo::skin::{SkinMesh, SubMeshDescriptor};
use crate::types::{Mesh, SubMesh};

/// `Data\Objects\…` (P4k) → `Objects/…` (DataCore-style geometry path).
pub(crate) fn geometry_path(p4k_path: &str) -> String {
    let p = p4k_path.replace('\\', "/");
    p.strip_prefix("Data/").map(str::to_owned).unwrap_or(p)
}

/// One part's pipeline mesh (LOD0, at the source file's shape) with its
/// materials (see the module docs).
pub(crate) struct PartSource {
    /// P4k path of the part's `.skin`.
    pub skin_path: String,
    pub mesh: Mesh,
    pub materials: Option<crate::mtl::MtlFile>,
    pub nmc: Option<crate::nmc::NodeMeshCombo>,
    /// Per `mesh.submeshes` entry: its clothing zone hash, when the file
    /// names one.
    pub zones: Vec<Option<u32>>,
}

impl PartSource {
    /// Load the part at `skin_path` (P4k path) through the entity pipeline
    /// with `material_path` (empty: the `.skin`'s own material). Textures
    /// are not loaded; the writers resolve them from `materials`.
    pub(crate) fn load(p4k: &MappedP4k, skin_path: &str, material_path: &str) -> Result<Self, CharacterError> {
        // The character shapes are LOD0 meshes.
        let opts = ExportOptions { lod_level: 0, material_mode: MaterialMode::None, ..ExportOptions::default() };
        let geom = geometry_path(skin_path);
        let (mesh, materials, _, nmc) = load_single_mesh(p4k, &geom, material_path, &opts, &mut PngCache::new(), false)
            .map_err(|e| CharacterError::Customization(format!("{geom}: {e}")))?;
        let zones = match p4k.read_file(&format!("{skin_path}m")).ok().as_deref().and_then(read_skin_mesh) {
            Some(skin) => submesh_zones(&mesh.submeshes, &skin.submeshes, &skin.extra_words),
            None => vec![None; mesh.submeshes.len()],
        };
        Ok(Self { skin_path: skin_path.to_owned(), mesh, materials, nmc, zones })
    }

    /// Indices of the submeshes whose zone is not in `hidden`.
    pub(crate) fn visible_submeshes(&self, hidden: &HashSet<u32>) -> Vec<usize> {
        (0..self.mesh.submeshes.len())
            .filter(|&i| !self.zones.get(i).copied().flatten().is_some_and(|z| hidden.contains(&z)))
            .collect()
    }

    /// The pipeline mesh with `shaped`'s positions and normals, and the skin
    /// of `shaped` named by `joint_names` (the part's own skeleton): the
    /// pipeline loader splits fully rigid parts (eyes) onto bones with no
    /// skin, while the rig binds every vertex. Tangents are dropped (stale
    /// for the new shape). The vertex streams must line up (same `.skinm`
    /// stream); a count mismatch is an error.
    pub(crate) fn reshaped(&self, shaped: &HeadMesh, joint_names: &[String]) -> Result<Mesh, CharacterError> {
        if self.mesh.positions.len() != shaped.positions.len() {
            return Err(CharacterError::Customization(format!(
                "{}: pipeline mesh has {} vertices, shaped part {}",
                self.skin_path,
                self.mesh.positions.len(),
                shaped.positions.len()
            )));
        }
        let mut mesh = self.mesh.clone();
        mesh.positions = shaped.positions.clone();
        mesh.skin = shared::skin_from_head(shaped, joint_names);
        if shaped.normals.len() == shaped.positions.len() {
            mesh.normals = Some(shaped.normals.clone());
            mesh.tangents = None; // stale for the new shape; the importers derive their own
        }
        Ok(mesh)
    }

    /// A source made from a [`HeadMesh`] itself (one material-less submesh,
    /// no colours or second UV set), for rigs built in unit tests.
    #[cfg(test)]
    pub(crate) fn from_head_mesh(name: &str, mesh: &HeadMesh) -> Self {
        let mesh = shared::mesh_from_head(mesh, &[]);
        let zones = vec![None; mesh.submeshes.len()];
        Self { skin_path: name.to_owned(), mesh, materials: None, nmc: None, zones }
    }
}

/// The IVO skin mesh chunk of a `.skinm` file.
fn read_skin_mesh(bytes: &[u8]) -> Option<SkinMesh> {
    let ChunkFile::Ivo(ivo) = ChunkFile::from_bytes(bytes).ok()? else { return None };
    ivo.chunks()
        .iter()
        .find(|e| e.chunk_type == starbreaker_chunks::known_types::ivo::IVO_SKIN2)
        .and_then(|e| SkinMesh::read(ivo.chunk_data(e)).ok())
}

/// The zone of each of `submeshes` (a loaded mesh's): the file's descriptor
/// with the same index range names it through `node_parent_index` into the
/// extra-word table `words`. A submesh without such a descriptor (split or
/// merged by the loader) has none.
pub(crate) fn submesh_zones(submeshes: &[SubMesh], descriptors: &[SubMeshDescriptor], words: &[u32]) -> Vec<Option<u32>> {
    submeshes
        .iter()
        .map(|s| {
            let d = descriptors.iter().find(|d| d.first_index == s.first_index && d.num_indices == s.num_indices)?;
            words.get(d.node_parent_index as usize).copied()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SubMesh;

    fn shaped() -> HeadMesh {
        HeadMesh {
            num_vertices: 2,
            num_indices: 0,
            num_submeshes: 0,
            bbox_min: [0.0; 3],
            bbox_max: [0.0; 3],
            positions: vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 2],
            uvs: Vec::new(),
            indices: Vec::new(),
            joints: vec![[1, 0, 0, 0]; 2],
            weights: vec![[1.0, 0.0, 0.0, 0.0]; 2],
            joints_hi: Vec::new(),
            weights_hi: Vec::new(),
        }
    }

    fn submesh(material_id: u32) -> SubMesh {
        SubMesh {
            material_name: None,
            material_id,
            source_material_id: None,
            first_index: 0,
            num_indices: 0,
            first_vertex: 0,
            num_vertices: 2,
            node_parent_index: 0,
        }
    }

    /// A pipeline-loaded part: two vertices with colours, two UV sets,
    /// tangents and two submeshes.
    fn source() -> PartSource {
        let mut mesh = Mesh::empty();
        mesh.positions = vec![[0.0; 3]; 2];
        mesh.normals = Some(vec![[1.0, 0.0, 0.0]; 2]);
        mesh.tangents = Some(vec![[1.0, 0.0, 0.0, 1.0]; 2]);
        mesh.uvs = Some(vec![[0.25, 0.5]; 2]);
        mesh.secondary_uvs = Some(vec![[0.75, 0.5]; 2]);
        mesh.colors = Some(vec![[255, 0, 0, 255]; 2]);
        mesh.submeshes = vec![submesh(0), submesh(1)];
        PartSource { skin_path: "eyes.skin".into(), mesh, materials: None, nmc: None, zones: vec![None, None] }
    }

    fn descriptor(node: u16, first_index: u32, num_indices: u32) -> SubMeshDescriptor {
        SubMeshDescriptor {
            mat_id: 0,
            node_parent_index: node,
            first_index,
            num_indices,
            first_vertex: 0,
            page_base: 0,
            num_vertices: 0,
            radius: 0.0,
            center: [0.0; 3],
            unknown0: 0,
            unknown1: 0,
        }
    }

    /// Submeshes find their descriptor by index range (in any order); its
    /// node indexes the zone words. A range the file does not have, or a
    /// node past the table, names no zone.
    #[test]
    fn submeshes_take_the_zone_of_their_descriptor() {
        let ranged = |first_index, num_indices| SubMesh { first_index, num_indices, ..submesh(0) };
        let submeshes = [ranged(30, 6), ranged(0, 30), ranged(36, 3), ranged(39, 3)];
        let descriptors = [descriptor(1, 0, 30), descriptor(0, 30, 6), descriptor(5, 39, 3)];
        let zones = submesh_zones(&submeshes, &descriptors, &[111, 222]);
        assert_eq!(zones, [Some(111), Some(222), None, None]);
    }

    #[test]
    fn hidden_zones_drop_their_submeshes() {
        let mut part = source();
        part.zones = vec![Some(7), None];
        assert_eq!(part.visible_submeshes(&HashSet::new()), [0, 1]);
        assert_eq!(part.visible_submeshes(&[7].into()), [1], "a zone-less submesh always shows");
    }

    /// A reshaped part takes the rig's shape and skin: fully rigid parts
    /// (eyes) come out of the pipeline loader split onto bones with no skin,
    /// but the rig binds every vertex.
    #[test]
    fn reshaped_takes_the_rig_shape_and_skin() {
        let shaped = shaped();
        let mesh = source().reshaped(&shaped, &["root".into(), "eye_l".into()]).unwrap();
        assert_eq!(mesh.positions, shaped.positions);
        assert_eq!(mesh.normals.as_deref(), Some(&shaped.normals[..]));
        assert!(mesh.tangents.is_none(), "tangents are stale for the new shape");
        let skin = mesh.skin.expect("rig skin");
        assert_eq!(skin.joint_names, vec!["root".to_string(), "eye_l".to_string()]);
        assert_eq!(skin.joints[0][0], 1);
        assert_eq!(skin.weights[1][0], 1.0);
    }

    /// Everything the shape does not change stays the pipeline's: colours,
    /// both UV sets and the per-material submeshes.
    #[test]
    fn reshaped_keeps_the_pipeline_streams_and_submeshes() {
        let mesh = source().reshaped(&shaped(), &["root".into(), "eye_l".into()]).unwrap();
        assert_eq!(mesh.colors, Some(vec![[255, 0, 0, 255]; 2]));
        assert_eq!(mesh.uvs, Some(vec![[0.25, 0.5]; 2]));
        assert_eq!(mesh.secondary_uvs, Some(vec![[0.75, 0.5]; 2]));
        assert_eq!(mesh.submeshes.iter().map(|s| s.material_id).collect::<Vec<_>>(), vec![0, 1]);
    }

    /// Without shaped normals the pipeline's normals (and tangents) stay.
    #[test]
    fn reshaped_without_normals_keeps_the_pipeline_normals() {
        let mut shaped = shaped();
        shaped.normals.clear();
        let mesh = source().reshaped(&shaped, &[]).unwrap();
        assert_eq!(mesh.normals, Some(vec![[1.0, 0.0, 0.0]; 2]));
        assert!(mesh.tangents.is_some());
        assert!(mesh.skin.is_none(), "unnamed joints cannot bind");
    }

    /// Streams that do not line up are an error naming the part, not a
    /// corrupt mesh.
    #[test]
    fn reshaped_rejects_a_vertex_count_mismatch() {
        let mut shaped = shaped();
        shaped.positions.push([0.0; 3]);
        let err = source().reshaped(&shaped, &[]).err().expect("mismatch");
        assert!(err.to_string().contains("eyes.skin"), "{err}");
    }

    #[test]
    fn geometry_path_strips_data_prefix() {
        assert_eq!(geometry_path(r"Data\Objects\A\b.skin"), "Objects/A/b.skin");
        assert_eq!(geometry_path("Objects/A/b.skin"), "Objects/A/b.skin");
    }

    /// The entity pipeline and the head parser read the same `.skinm`
    /// vertex stream in the same order — the premise of
    /// [`PartSource::reshaped`]. Skipped unless Data.p4k is accessible.
    #[test]
    fn pipeline_mesh_matches_head_parser_vertex_order() {
        let Some(p4k) = crate::test_p4k::p4k() else { return };
        let base = r"Data\Objects\Characters\Human\heads\male\pu\protos_human_male_face_t1_pu\protos_human_male_face_t1_pu";
        for part in ["head", "eyes", "teeth"] {
            let skin = format!("{base}_{part}.skin");
            let (skin_bytes, skinm_bytes) = (p4k.read_file(&skin).unwrap(), p4k.read_file(&format!("{skin}m")).unwrap());
            let ours = if part == "head" {
                super::super::head_skin::parse_head_skin(&skin_bytes, &skinm_bytes).unwrap().mesh
            } else {
                super::super::head_skin::parse_skin_mesh(&skin_bytes, &skinm_bytes).unwrap().0
            };
            let source = PartSource::load(&p4k, &skin, "").unwrap();
            assert_eq!(source.mesh.positions.len(), ours.positions.len(), "{part}");
            let worst = source
                .mesh
                .positions
                .iter()
                .zip(&ours.positions)
                .flat_map(|(a, b)| (0..3).map(move |k| (a[k] - b[k]).abs()))
                .fold(0.0f32, f32::max);
            assert!(worst < 1e-5, "{part}: vertex streams differ by {worst}");
            assert!(source.materials.is_some(), "{part}: the .skin's own material");
        }
    }
}
