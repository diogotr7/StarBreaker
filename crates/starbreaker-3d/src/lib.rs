pub mod dequant;
pub(crate) mod decomposed;
pub mod error;
pub(crate) mod gltf;
pub(crate) mod included_objects;
pub mod ivo;
pub mod clip;
pub mod mesh_rig;
pub mod parity;
pub mod mtl;
pub mod nmc;
pub(crate) mod pipeline;
pub mod skeleton;
pub(crate) mod socpak;
pub mod types;
pub mod animation;
pub mod character;
pub mod chrparams;
pub mod riglogic;
pub mod validation;
pub mod ui_pipeline;
#[cfg(test)]
pub(crate) mod test_p4k;

pub use error::Error;
pub use included_objects::{IncludedObject, IncludedObjects};
pub use pipeline::{
    DecomposedExport, ExportFormat, ExportKind, ExportOptions, ExportResult, ExportedFile,
    ExportedFileKind, MaterialMode,
    assemble_glb_with_loadout, assemble_glb_with_loadout_with_progress,
    dump_hierarchy, inspect_socpak_hierarchy, load_invisible_ports, query_animation_controller_source,
    resolve_loadout_meshes, socpaks_to_decomposed_blend,
    socpaks_to_decomposed_blend_with_progress, socpaks_to_glb, SocpakExportProgress,
    SocpakHierarchyNode,
};
pub use types::Mesh;
pub use validation::{ValidationReport, validate_decomposed_export};

use starbreaker_chunks::ChunkFile;

/// Decode a multi-part `.cga`/`.cgf` and BAKE its NMC scene-graph node
/// transforms into a single world-space [`Mesh`], the same assembly the glTF
/// builder applies (so the result matches what the app's `skin_to_glb` viewer
/// shows: wings, engine pods and sub-objects positioned, not detached at the
/// origin). `mesh_data` is the vertex companion (`.cgam`/`.cgfm`/`.skinm`);
/// `nmc_data` is the primary file (`.cga`/…) carrying the NMC chunk. Falls back
/// to the raw [`parse_skin`] mesh when the file has no usable NMC hierarchy.
/// Result is in CryEngine (Z-up) model space.
pub fn parse_skin_positioned(mesh_data: &[u8], nmc_data: &[u8]) -> Result<Mesh, Error> {
    let mesh = parse_skin(mesh_data)?;
    match nmc::parse_nmc_full(nmc_data) {
        Some((nodes, _mat)) if nodes.len() > 1 => Ok(flatten_nmc_to_world(&mesh, &nodes)),
        _ => Ok(mesh),
    }
}

/// Append 4 shield-face panes (front/rear/port/starboard) around `ship`,
/// returning the merged mesh **and the index-array offset at which the shield
/// triangles begin** (so the renderer can draw them at a reduced alpha).
///
/// Mirrors the engine's SELF-STATUS shield render (`UIHoloVehicle_Config`:
/// `shieldProxyModel` = the generic `shield_pane.cgf`, `shieldDistance` = how
/// much of the shield box the hull fills). The decoded `shield_pane` is FLAT in
/// its local X-Y plane (width X, height Y) with a shallow dome bulging along +Z
/// (its normal). Each of the 4 faces stands the pane up (width → horizontal
/// tangent, height → world up, dome → outward), then rotates it about the
/// vertical axis by 0/90/180/270° so the faces are identical and the rear is
/// the front rotated 180°.
///
/// Layout is derived ENTIRELY from the hull bounding box + `shield_distance`
/// (no invented per-screen constants); X = hull length, Y = width, Z = height:
/// * Each wall is sized to the hull's SILHOUETTE on its face — front/rear walls
///   span the hull width (Y) × height (Z); port/starboard span the hull length
///   (X) × height (Z). The box is therefore PROPORTIONED to the hull, not a
///   forced square.
/// * Each wall sits at `hull_half × (1 + shield_distance)` along its outward
///   normal — i.e. `shield_distance` (e.g. 0.85) is the gap beyond the hull as
///   a fraction of that half-extent, so the box is markedly larger than the
///   hull. The hull-silhouette-sized wall is narrower than the (larger) box
///   face, so the margin appears as a GAP at every corner.
/// The engine derives exact faces from the ship's shield component; this is the
/// generic UI-proxy approximation the MFD hologram uses.
pub fn with_shield_panes(mut ship: Mesh, pane: &Mesh, shield_distance: f32) -> (Mesh, usize) {
    let ship_index_count = ship.indices.len();
    if pane.positions.is_empty() || ship.positions.is_empty() {
        return (ship, ship_index_count);
    }
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in &ship.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    // Centre the box on the ship's ORIGIN (0,0,0), not the bbox centre: a hull is
    // typically asymmetric about its origin (it reaches further one way than the
    // other), so a bbox-centred box sits off to one side of the origin. The
    // origin also DEFINES the box size — each half-extent is the hull's max reach
    // FROM the origin, so the box is symmetric about the origin and contains it.
    let c = [0.0, 0.0, 0.0];
    let half = [
        lo[0].abs().max(hi[0].abs()),
        lo[1].abs().max(hi[1].abs()),
        lo[2].abs().max(hi[2].abs()),
    ];

    // Pane local extents: X = width, Y = height (both centred on 0), Z = the
    // shallow dome depth (its normal; NOT centred — the rim sits at z_min).
    let (mut plo, mut phi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in &pane.positions {
        for k in 0..3 {
            plo[k] = plo[k].min(p[k]);
            phi[k] = phi[k].max(p[k]);
        }
    }
    let pane_w = (phi[0] - plo[0]).max(1e-4); // local x extent (width)
    let pane_h = (phi[1] - plo[1]).max(1e-4); // local y extent (height)

    // `shieldDistance` is the fraction of the shield box the hull fills along
    // each axis (per `UIHoloVehicle_Config`), so the box face sits at
    // hull_half / sd — a snug box (sd 0.85 → ~18% gap) that hugs the hull with
    // natural corner gaps, matching the in-game SELF-STATUS reference. (The
    // earlier `hull_half × (1 + sd)` reading put the panes ~85% out — far too
    // loose.)
    let sd = shield_distance.clamp(0.05, 1.0);
    // The box is PROPORTIONED to the hull (not square): each wall is sized to the
    // hull silhouette on its own face — front/rear span the hull WIDTH, port/
    // starboard span the hull LENGTH — and placed per-axis at `hull_half / sd`.
    // A face therefore spans exactly `sd` of its box side, leaving a clean
    // ~15% (1 − sd) corner gap, and the box's aspect matches the hull's so the
    // longest hull axis fills the longest screen axis when fit to the frame.
    let wall_height = 2.0 * half[2];
    let pcx = 0.5 * (plo[0] + phi[0]);
    let pcy = 0.5 * (plo[1] + phi[1]);

    // Combined rotation R(θ) = Rz(θ)·R0, where R0 stands the pane up (local
    // width→world +Y, height→world +Z, dome/normal→world +X) and Rz(θ) spins it
    // about the vertical axis. Outward normal n(θ) = (cosθ, sinθ, 0): quarters
    // 0/2 face ±X (normal axis X, tangent Y), 1/3 face ±Y (normal Y, tangent X).
    for quarter in 0..4 {
        let theta = std::f32::consts::FRAC_PI_2 * quarter as f32;
        let (s, co) = theta.sin_cos();
        let on_x = quarter % 2 == 0;
        let normal_half = if on_x { half[0] } else { half[1] };
        let tangent_half = if on_x { half[1] } else { half[0] };
        let offset = normal_half / sd; // box face on this axis: hull fills `sd`
        let wall_width = 2.0 * tangent_half; // hull silhouette span on this face
        // R rows (see doc): [[-s,0,co],[co,0,s],[0,1,0]].
        let r = [[-s, 0.0, co], [co, 0.0, s], [0.0, 1.0, 0.0]];
        let centre = [c[0] + offset * co, c[1] + offset * s, c[2]];
        let base = ship.positions.len() as u32;
        for p in &pane.positions {
            let lx = p[0] - pcx; // centred width
            let ly = p[1] - pcy; // centred height
            let lz = p[2] - plo[2]; // rim at 0, dome outward
            // Rotate the pane 90° in its own plane BEFORE stretching: the pane's
            // HEIGHT axis (local Y) fills the wall's long tangent, its WIDTH
            // (local X) fills the short world-up edge, and the dome scales with
            // the tangent fill. (Stretching before this rotation warped the
            // curve onto the wrong axis — the panes read as flat trays instead
            // of outward-bowing walls.) Outputs feed R as (sx→tangent,
            // sy→world-up, sz→normal/outward).
            let sx = ly * (wall_width / pane_h);
            let sy = lx * (wall_height / pane_w);
            let sz = lz * (wall_width / pane_h);
            ship.positions.push([
                r[0][0] * sx + r[0][1] * sy + r[0][2] * sz + centre[0],
                r[1][0] * sx + r[1][1] * sy + r[1][2] * sz + centre[1],
                r[2][0] * sx + r[2][1] * sy + r[2][2] * sz + centre[2],
            ]);
        }
        for &i in &pane.indices {
            ship.indices.push(base + i);
        }
    }
    ship.submeshes = Vec::new();
    (ship, ship_index_count)
}

/// Multiply two 3×4 row-major affine matrices (each implicitly `[0,0,0,1]`
/// bottom row): `a * b`.
fn mat3x4_mul(a: &[[f32; 4]; 3], b: &[[f32; 4]; 3]) -> [[f32; 4]; 3] {
    let mut r = [[0.0f32; 4]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
        r[i][3] = a[i][0] * b[0][3] + a[i][1] * b[1][3] + a[i][2] * b[2][3] + a[i][3];
    }
    r
}

const NMC_IDENTITY: [[f32; 4]; 3] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

fn nmc_matrix_is_identity_or_zero(m: &[[f32; 4]; 3]) -> bool {
    const EPS: f32 = 1e-5;
    if m.iter().all(|row| row.iter().all(|&v| v.abs() < EPS)) {
        return true;
    }
    m.iter()
        .zip(NMC_IDENTITY.iter())
        .all(|(r, ir)| r.iter().zip(ir.iter()).all(|(&a, &b)| (a - b).abs() < EPS))
}

/// Bake NMC node transforms into world-space vertices. Mirrors the glTF
/// builder: each node's `bone_to_world` is its LOCAL matrix composed down the
/// parent hierarchy; a submesh's triangles (its INDEX range) reference shared
/// per-node-local positions, transformed by that node's composed world matrix.
fn flatten_nmc_to_world(mesh: &Mesh, nodes: &[nmc::NmcNode]) -> Mesh {
    // Composed world matrix per node (parents resolved first via a fixpoint
    // pass — NMC parent indices are not guaranteed to precede children).
    let local: Vec<[[f32; 4]; 3]> = nodes
        .iter()
        .map(|n| {
            if nmc_matrix_is_identity_or_zero(&n.bone_to_world) {
                NMC_IDENTITY
            } else {
                n.bone_to_world
            }
        })
        .collect();
    let mut world: Vec<Option<[[f32; 4]; 3]>> = vec![None; nodes.len()];
    for _ in 0..nodes.len() {
        let mut progressed = false;
        for (i, node) in nodes.iter().enumerate() {
            if world[i].is_some() {
                continue;
            }
            match node.parent_index {
                Some(p) if (p as usize) < nodes.len() => {
                    if let Some(pw) = world[p as usize] {
                        world[i] = Some(mat3x4_mul(&pw, &local[i]));
                        progressed = true;
                    }
                }
                _ => {
                    world[i] = Some(local[i]);
                    progressed = true;
                }
            }
        }
        if !progressed {
            break;
        }
    }

    let apply = |m: &[[f32; 4]; 3], p: [f32; 3]| -> [f32; 3] {
        [
            m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
            m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
            m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
        ]
    };

    let mut out_pos: Vec<[f32; 3]> = Vec::with_capacity(mesh.indices.len());
    let mut out_idx: Vec<u32> = Vec::with_capacity(mesh.indices.len());
    for sub in &mesh.submeshes {
        let w = world
            .get(sub.node_parent_index as usize)
            .and_then(|w| *w)
            .unwrap_or(NMC_IDENTITY);
        let start = sub.first_index as usize;
        let end = (start + sub.num_indices as usize).min(mesh.indices.len());
        for &vi in &mesh.indices[start..end] {
            let Some(&p) = mesh.positions.get(vi as usize) else {
                continue;
            };
            out_idx.push(out_pos.len() as u32);
            out_pos.push(apply(&w, p));
        }
    }
    if out_pos.is_empty() {
        return mesh.clone();
    }
    let mut flat = mesh.clone();
    flat.positions = out_pos;
    flat.indices = out_idx;
    flat.uvs = None;
    flat.normals = None;
    flat.tangents = None;
    flat.colors = None;
    flat.secondary_uvs = None;
    flat.submeshes = Vec::new();
    flat
}

/// Parse a `.skin`/`.cgf` IVO file into a Mesh domain type.
/// Returns an error if the file uses CrCh format (not supported).
pub fn parse_skin(data: &[u8]) -> Result<Mesh, Error> {
    parse_skin_with_options(data, false)
}

/// Parse a `.skin`/`.cgf` IVO file, optionally dequantizing with model bbox.
/// Interior CGFs use `use_model_bbox = true` because IncludedObjects placements
/// are authored for model-bbox space.
pub(crate) fn parse_skin_with_options(data: &[u8], use_model_bbox: bool) -> Result<Mesh, Error> {
    parse_skin_pair_with_options(data, Some(data), use_model_bbox)
}

/// Parse an IVO mesh from its stream file (`.skinm` / `.cgfm`, or a
/// single-file `.skin`) and its metadata file (`.skin` / `.cgf`), which holds
/// the bone table that names the soft skin's joints (`Mesh::skin`).
pub fn parse_skin_pair(streams: &[u8], metadata: Option<&[u8]>) -> Result<Mesh, Error> {
    parse_skin_pair_with_options(streams, metadata, false)
}

pub(crate) fn parse_skin_pair_with_options(
    data: &[u8],
    metadata: Option<&[u8]>,
    use_model_bbox: bool,
) -> Result<Mesh, Error> {
    let chunk_file = ChunkFile::from_bytes(data)?;
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => return Err(Error::UnsupportedFormat),
    };

    // Find and parse IvoSkin2 chunk (0xB8757777)
    let skin_entry = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == starbreaker_chunks::known_types::ivo::IVO_SKIN2)
        .ok_or(Error::MissingChunk {
            chunk_type: starbreaker_chunks::known_types::ivo::IVO_SKIN2,
        })?;
    let skin_mesh = ivo::skin::SkinMesh::read(ivo.chunk_data(skin_entry))?;

    // Find and parse MtlName chunks (0x83353333)
    let materials: Vec<ivo::material::MaterialName> = ivo
        .chunks()
        .iter()
        .filter(|c| c.chunk_type == starbreaker_chunks::known_types::ivo::MTL_NAME_IVO320)
        .map(|entry| ivo::material::MaterialName::read(ivo.chunk_data(entry)))
        .collect::<Result<_, _>>()?;

    let joint_names: Option<Vec<String>> = [metadata, Some(data)]
        .into_iter()
        .flatten()
        .find_map(skeleton::parse_skeleton)
        .filter(|bones| !bones.is_empty())
        .map(|bones| bones.into_iter().map(|b| b.name).collect());
    Ok(types::build_mesh_with_rig(&skin_mesh, &materials, use_model_bbox, joint_names.as_deref()))
}

/// Parse a `.skin`/`.cgf` IVO file and write it as a native `.blend` (one
/// object `name`), through the same `.blend` mesh writer as decomposed
/// exports: NMC hierarchy from `metadata`, and the skin as vertex groups.
/// The counterpart of [`skin_to_glb`].
pub fn skin_to_blend(name: &str, data: &[u8], metadata: Option<&[u8]>) -> Result<Vec<u8>, Error> {
    let mesh = parse_skin_pair(data, metadata)?;
    let nmc = metadata
        .and_then(nmc::parse_nmc_full)
        .map(|(nodes, material_indices)| nmc::NodeMeshCombo { nodes, material_indices });
    // A standalone .skin's own bone table is its skeleton.
    let bones = metadata.and_then(skeleton::parse_skeleton).unwrap_or_default();
    pipeline::mesh_to_blend(name, &mesh, &None, nmc.as_ref(), None, &bones, &[])
}

/// Parse a `.skin`/`.cgf` IVO file and convert to GLB in one step.
/// If `metadata` is provided (the primary `.cgf`/`.skin` bytes), its NMC
/// transforms build the scene graph and its bone table is the skeleton the
/// mesh's skin binds to.
pub fn skin_to_glb(data: &[u8], metadata: Option<&[u8]>) -> Result<Vec<u8>, Error> {
    let mesh = parse_skin_pair(data, metadata)?;
    // A standalone .skin's own bone table is its skeleton.
    let skeleton_bones = metadata.and_then(skeleton::parse_skeleton).unwrap_or_default();
    let root_nmc = metadata
        .and_then(nmc::parse_nmc_full)
        .map(|(nodes, mat_indices)| nmc::NodeMeshCombo {
            nodes,
            material_indices: mat_indices,
        });
    gltf::write_glb(
        gltf::GlbInput {
            root_mesh: Some(mesh),
            root_materials: None,
            root_textures: None,
            root_nmc,
            root_palette: None,
            skeleton_bones,
            children: Vec::new(),
            interiors: pipeline::LoadedInteriors::default(),
            animations: Vec::new(),
        },
        &mut gltf::GlbLoaders {
            load_textures: &mut |_, _| None,
            load_interior_mesh: &mut |_| None,
        },
        &gltf::GlbOptions {
            material_mode: pipeline::MaterialMode::None,
            preserve_textureless_decal_primitives: false,
            metadata: gltf::GlbMetadata {
                entity_name: None,
                geometry_path: None,
                material_path: None,
                export_options: gltf::ExportOptionsMetadata {
                    kind: "Bundled".to_string(),
                    material_mode: "None".to_string(),
                    format: "Glb".to_string(),
                    lod_level: 0,
                    texture_mip: 0,
                    include_attachments: false,
                    include_interior: false,
                },
            },
            fallback_palette: None,
        },
    )
}

#[cfg(test)]
mod nmc_flatten_tests {
    use super::*;
    use crate::nmc::NmcNode;
    use crate::types::{Mesh, SubMesh};

    fn node(name: &str, parent: Option<u16>, btw: [[f32; 4]; 3]) -> NmcNode {
        NmcNode {
            name: name.into(),
            parent_index: parent,
            world_to_bone: NMC_IDENTITY,
            bone_to_world: btw,
            scale: [1.0, 1.0, 1.0],
            geometry_type: 0,
            properties: std::collections::HashMap::new(),
        }
    }

    fn submesh(node_idx: u16, first_index: u32, num_indices: u32) -> SubMesh {
        SubMesh {
            material_name: None,
            material_id: 0,
            source_material_id: None,
            first_index,
            num_indices,
            first_vertex: 0,
            num_vertices: 0,
            node_parent_index: node_idx,
        }
    }

    /// A submesh attached to a node with a translation is baked into world
    /// space; a child node composes its parent's transform. Mirrors the wing /
    /// engine-pod placement that raw `parse_skin` leaves at the origin.
    #[test]
    fn flatten_bakes_node_and_parent_transforms() {
        // Node 0: root identity. Node 1: root, +10 in x. Node 2: child of 1, +1 in y.
        let nodes = vec![
            node("root", None, NMC_IDENTITY),
            node("wing", None, [[1.0, 0.0, 0.0, 10.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]]),
            node("tip", Some(1), [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]]),
        ];
        // One vertex at origin per node, indexed once each.
        let mut mesh = Mesh {
            positions: vec![[0.0, 0.0, 0.0]],
            indices: vec![0, 0, 0],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![submesh(0, 0, 1), submesh(1, 1, 1), submesh(2, 2, 1)],
            model_min: [0.0; 3],
            model_max: [0.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [0.0; 3],
            skin: None,
            morph_targets: Vec::new(),
        };
        // Make each index reference the single shared origin vertex.
        mesh.indices = vec![0, 0, 0];
        let flat = flatten_nmc_to_world(&mesh, &nodes);
        assert_eq!(flat.positions.len(), 3, "one emitted vertex per index");
        // sub0 (root): origin unchanged.
        assert_eq!(flat.positions[0], [0.0, 0.0, 0.0]);
        // sub1 (wing): +10 x.
        assert_eq!(flat.positions[1], [10.0, 0.0, 0.0]);
        // sub2 (tip, child of wing): parent +10x composed with local +1y.
        assert_eq!(flat.positions[2], [10.0, 1.0, 0.0]);
    }

    fn bare_mesh(positions: Vec<[f32; 3]>, indices: Vec<u32>) -> Mesh {
        Mesh {
            positions,
            indices,
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![],
            model_min: [0.0; 3],
            model_max: [0.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [0.0; 3],
            skin: None,
            morph_targets: Vec::new(),
        }
    }

    /// The 4 shield panes form a box PROPORTIONED to the hull: each wall is sized
    /// to the hull silhouette on its own face (front/rear span the hull WIDTH,
    /// port/starboard span the hull LENGTH — so the faces are NOT all the same
    /// size) and PLACED per-axis at `hull_half / shieldDistance`. A face spans
    /// `shieldDistance` of its box side → clean ~15% corner gap, and the box's
    /// aspect matches the hull's. Guards the proportioned sizing/placement, the
    /// pane-axis fix (flat X-Y, dome +Z) and the data-grounded box gap.
    #[test]
    fn shield_panes_form_proportioned_box_clear_of_hull() {
        // Hull bbox ±(10, 5, 2): half = (10, 5, 2), centred at the origin.
        let ship = bare_mesh(vec![[-10.0, -5.0, -2.0], [10.0, 5.0, 2.0], [0.0, 0.0, 0.0]], vec![0, 1, 2]);
        // Flat unit pane in X-Y (width 1, height 1) with a +Z dome (rim at z=0).
        let pane = bare_mesh(
            vec![[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.5, 0.5, 0.1], [-0.5, 0.5, 0.1]],
            vec![0, 1, 2, 0, 2, 3],
        );
        let sd = 0.85_f32;
        let (merged, boundary) = with_shield_panes(ship, &pane, sd);

        // Shield triangles begin right after the hull indices; 4 faces appended.
        assert_eq!(boundary, 3, "boundary = original hull index count");
        assert_eq!(merged.positions.len(), 3 + 4 * 4, "4 faces × 4 pane verts");
        assert_eq!(merged.indices.len(), 3 + 4 * 6, "4 faces × 6 pane indices");

        // Front wall (+X, quarter 0 = verts[3..7]): face at half_x/sd, spans the
        // hull WIDTH (2 × half_y = 10) along Y.
        let front = &merged.positions[3..7];
        let front_rim = front.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
        assert!((front_rim - 10.0 / sd).abs() < 0.2, "front face at half_x/sd, got {front_rim}");
        assert!(front_rim > 10.0, "front wall clears the hull (x > 10)");
        let front_w = span(front, 1);
        assert!((front_w - 10.0).abs() < 0.1, "front wall width = hull width (2×5), got {front_w}");

        // Port wall (+Y, quarter 1 = verts[7..11]): face at half_y/sd, spans the
        // hull LENGTH (2 × half_x = 20) along X.
        let port = &merged.positions[7..11];
        let port_rim = port.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert!((port_rim - 5.0 / sd).abs() < 0.2, "port face at half_y/sd, got {port_rim}");
        assert!(port_rim > 5.0, "port wall clears the hull (y > 5)");
        let port_w = span(port, 0);
        assert!((port_w - 20.0).abs() < 0.1, "port wall width = hull length (2×10), got {port_w}");

        // PROPORTIONED, not square: the faces differ in size, but each still
        // spans `sd` of its own box side (~15% corner gap).
        assert!((front_w - port_w).abs() > 1.0, "box is proportioned to the hull, not square");
        // The front wall spans Y, whose box side is set by the port/aft walls
        // (and vice-versa); each face covers `sd` of that side → ~15% gap.
        assert!((front_w / (2.0 * port_rim) - sd).abs() < 0.02, "front face spans `sd` of the box Y side");
        assert!((port_w / (2.0 * front_rim) - sd).abs() < 0.02, "port face spans `sd` of the box X side");
        // Wall height = hull height (2 × half_z = 4) centred on z=0 → ±2.
        let max_z = merged.positions[3..].iter().map(|p| p[2].abs()).fold(0.0, f32::max);
        assert!((max_z - 2.0).abs() < 1e-2, "wall half-height = hull half-height ~2, got {max_z}");
    }

    /// The pane is rotated 90° in its own plane BEFORE the stretch, so its
    /// HEIGHT axis (local +Y) fills the wall's long tangent and its WIDTH axis
    /// (local +X) fills the short world-up edge. Marker verts at the pane's
    /// +width and +height edges must therefore land on opposite wall axes — the
    /// discriminator vs the old stretch-then-rotate order (which mapped width
    /// onto the tangent and read as a flat tray).
    #[test]
    fn shield_pane_rotated_90_before_stretch() {
        // Hull ±(10, 5, 2). Symmetric cross pane (centre at origin): v0 at the
        // +width edge, v2 at the +height edge, so the centring is clean.
        let ship = bare_mesh(vec![[-10.0, -5.0, -2.0], [10.0, 5.0, 2.0], [0.0, 0.0, 0.0]], vec![0, 1, 2]);
        let pane = bare_mesh(
            vec![[0.5, 0.0, 0.0], [-0.5, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, -0.5, 0.0]],
            vec![0, 2, 1, 1, 2, 3],
        );
        let (merged, _) = with_shield_panes(ship, &pane, 0.85);
        // Front wall (+X, quarter 0) = first appended pane (verts[3..7]); its
        // tangent is world Y, its world-up is world Z.
        let m_w = merged.positions[3]; // +width-edge vertex (v0)
        let m_h = merged.positions[5]; // +height-edge vertex (v2)
        // WIDTH edge → world-up (Z) extreme, tangent (Y) ≈ 0.
        assert!(m_w[1].abs() < 1e-3, "pane width edge does NOT bleed onto the tangent, got y={}", m_w[1]);
        assert!(m_w[2].abs() > 1.0, "pane width edge fills the world-up (Z) edge, got z={}", m_w[2]);
        // HEIGHT edge → tangent (Y) extreme, world-up (Z) ≈ 0.
        assert!(m_h[2].abs() < 1e-3, "pane height edge does NOT bleed onto world-up, got z={}", m_h[2]);
        assert!(m_h[1].abs() > 1.0, "pane height edge fills the tangent (Y) edge, got y={}", m_h[1]);
    }

    fn span(verts: &[[f32; 3]], axis: usize) -> f32 {
        let mn = verts.iter().map(|p| p[axis]).fold(f32::MAX, f32::min);
        let mx = verts.iter().map(|p| p[axis]).fold(f32::MIN, f32::max);
        mx - mn
    }
}
