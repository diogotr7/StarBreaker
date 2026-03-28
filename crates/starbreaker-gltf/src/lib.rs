pub mod dequant;
pub mod error;
pub(crate) mod glb_builder;
pub(crate) mod gltf;
pub(crate) mod included_objects;
pub mod ivo;
pub mod mtl;
pub(crate) mod nmc;
pub(crate) mod pipeline;
pub mod skeleton;
pub(crate) mod socpak;
pub mod types;

pub use error::Error;
pub use pipeline::{
    ExportOptions, ExportResult, assemble_glb_with_loadout,
    dump_hierarchy, load_invisible_ports, resolve_loadout_meshes, socpaks_to_glb,
};
pub use types::Mesh;

use starbreaker_chunks::ChunkFile;

/// Parse a `.skin`/`.cgf` IVO file into a Mesh domain type.
/// Returns an error if the file uses CrCh format (not supported).
pub fn parse_skin(data: &[u8]) -> Result<Mesh, Error> {
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

    Ok(types::build_mesh(&skin_mesh, &materials))
}

/// Like `parse_skin` but dequantizes positions using the model bounding box
/// instead of the scaling bbox. Interior CGFs need this because IncludedObjects
/// placements are authored for model-bbox space.
pub(crate) fn parse_skin_model_bbox(data: &[u8]) -> Result<Mesh, Error> {
    let chunk_file = ChunkFile::from_bytes(data)?;
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => return Err(Error::UnsupportedFormat),
    };
    let skin_entry = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == starbreaker_chunks::known_types::ivo::IVO_SKIN2)
        .ok_or(Error::MissingChunk {
            chunk_type: starbreaker_chunks::known_types::ivo::IVO_SKIN2,
        })?;
    let skin_mesh = ivo::skin::SkinMesh::read(ivo.chunk_data(skin_entry))?;
    let materials: Vec<ivo::material::MaterialName> = ivo
        .chunks()
        .iter()
        .filter(|c| c.chunk_type == starbreaker_chunks::known_types::ivo::MTL_NAME_IVO320)
        .map(|entry| ivo::material::MaterialName::read(ivo.chunk_data(entry)))
        .collect::<Result<_, _>>()?;
    Ok(types::build_mesh_with_bbox(&skin_mesh, &materials, true))
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
        },
        &mut gltf::GlbLoaders {
            load_textures: &mut |_| None,
            load_interior_mesh: &mut |_| None,
        },
        &gltf::GlbOptions {
            include_tangents: true,
            experimental_textures: false,
            metadata: gltf::GlbMetadata {
                entity_name: None,
                geometry_path: None,
                material_path: None,
                export_options: gltf::ExportOptionsMetadata {
                    texture_mip: 0,
                    lod_level: 0,
                    include_interior: false,
                    include_tangents: true,
                    include_lights: false,
                    include_textures: false,
                    include_materials: false,
                },
            },
            fallback_palette: None,
        },
    )
}
