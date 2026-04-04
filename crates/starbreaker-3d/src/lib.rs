pub mod animation;
pub mod dequant;
pub mod error;
pub(crate) mod gltf;
pub(crate) mod included_objects;
pub mod ivo;
pub mod mtl;
pub mod nmc;
pub(crate) mod pipeline;
pub mod skeleton;
pub(crate) mod socpak;
pub mod types;

pub use error::Error;
pub use pipeline::{
    ExportFormat, ExportOptions, ExportResult, MaterialMode,
    assemble_glb_with_loadout,
    dump_hierarchy, load_invisible_ports, resolve_loadout_meshes, socpaks_to_glb,
};
pub use types::Mesh;

use starbreaker_chunks::ChunkFile;

/// Parse a `.skin`/`.cgf` IVO file into a Mesh domain type.
/// Returns an error if the file uses CrCh format (not supported).
pub fn parse_skin(data: &[u8]) -> Result<Mesh, Error> {
    parse_skin_with_options(data, false)
}

/// Parse a `.skin`/`.cgf` IVO file, optionally dequantizing with model bbox.
/// Interior CGFs use `use_model_bbox = true` because IncludedObjects placements
/// are authored for model-bbox space.
pub(crate) fn parse_skin_with_options(data: &[u8], use_model_bbox: bool) -> Result<Mesh, Error> {
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

    Ok(types::build_mesh_with_bbox(&skin_mesh, &materials, use_model_bbox))
}

/// Parse a `.skin`/`.cgf` IVO file and convert to GLB in one step.
pub fn skin_to_glb(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mesh = parse_skin(data)?;
    gltf::write_glb(
        gltf::GlbInput {
            root_mesh: Some(mesh),
            root_materials: None,
            root_textures: None,
            root_nmc: None,
            root_palette: None,
            skeleton_bones: Vec::new(),
            children: Vec::new(),
            interiors: pipeline::LoadedInteriors::default(),
            animations: Vec::new(),
        },
        &mut gltf::GlbLoaders {
            load_textures: &mut |_| None,
            load_interior_mesh: &mut |_| None,
        },
        &gltf::GlbOptions {
            material_mode: pipeline::MaterialMode::None,
            metadata: gltf::GlbMetadata {
                entity_name: None,
                geometry_path: None,
                material_path: None,
                export_options: gltf::ExportOptionsMetadata {
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
