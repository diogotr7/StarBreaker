mod glb_builder;

use gltf_json as json;
use json::validation::Checked;
use starbreaker_common::progress::{report as report_progress, Progress};

use crate::error::Error;
use crate::nmc::NodeMeshCombo;
use crate::types::{MaterialTextures, Mesh};

use glb_builder::GlbBuilder;

/// All input data for building a GLB file.
pub struct GlbInput {
    pub root_mesh: Option<Mesh>,
    pub root_materials: Option<crate::mtl::MtlFile>,
    pub root_textures: Option<MaterialTextures>,
    pub root_nmc: Option<NodeMeshCombo>,
    pub root_palette: Option<crate::mtl::TintPalette>,
    pub skeleton_bones: Vec<crate::skeleton::Bone>,
    pub children: Vec<crate::types::EntityPayload>,
    pub interiors: crate::pipeline::LoadedInteriors,
}

pub struct GlbLoaders<'a> {
    pub load_textures: &'a mut dyn FnMut(
        Option<&crate::mtl::MtlFile>,
        Option<&crate::mtl::TintPalette>,
    ) -> Option<MaterialTextures>,
    pub load_interior_mesh: &'a mut dyn FnMut(
        &crate::pipeline::InteriorCgfEntry,
    ) -> Option<(Mesh, Option<crate::mtl::MtlFile>, Option<NodeMeshCombo>)>,
}

pub struct GlbOptions {
    pub material_mode: crate::pipeline::MaterialMode,
    pub preserve_textureless_decal_primitives: bool,
    pub metadata: GlbMetadata,
    pub fallback_palette: Option<crate::mtl::TintPalette>,
}

pub struct GlbMetadata {
    pub entity_name: Option<String>,
    pub geometry_path: Option<String>,
    pub material_path: Option<String>,
    pub export_options: ExportOptionsMetadata,
}

pub struct ExportOptionsMetadata {
    pub kind: String,
    pub material_mode: String,
    pub format: String,
    pub lod_level: u32,
    pub texture_mip: u32,
    pub include_attachments: bool,
    pub include_interior: bool,
}


pub(crate) fn mat3x4_to_gltf(m: &[[f32; 4]; 3]) -> [f32; 16] {
    [
        m[0][0], m[1][0], m[2][0], 0.0, // column 0
        m[0][1], m[1][1], m[2][1], 0.0, // column 1
        m[0][2], m[1][2], m[2][2], 0.0, // column 2
        m[0][3], m[1][3], m[2][3], 1.0, // column 3 (translation)
    ]
}

/// Add a BufferView + Accessor pair for vertex attribute data.
/// Returns the accessor index, or None if `byte_length` is 0.
pub(crate) fn add_vertex_accessor(
    buffer_views: &mut Vec<json::buffer::View>,
    accessors: &mut Vec<json::Accessor>,
    byte_offset: usize,
    byte_length: usize,
    count: usize,
    accessor_type: json::accessor::Type,
    min_max: Option<(&[f32], &[f32])>,
) -> Option<u32> {
    if byte_length == 0 {
        return None;
    }
    let bv_idx = buffer_views.len() as u32;
    buffer_views.push(json::buffer::View {
        buffer: json::Index::new(0),
        byte_offset: Some(json::validation::USize64(byte_offset as u64)),
        byte_length: json::validation::USize64(byte_length as u64),
        byte_stride: None,
        target: Some(Checked::Valid(json::buffer::Target::ArrayBuffer)),
        name: None,
        extensions: None,
        extras: Default::default(),
    });
    let acc_idx = accessors.len() as u32;
    accessors.push(json::Accessor {
        buffer_view: Some(json::Index::new(bv_idx)),
        byte_offset: Some(json::validation::USize64(0)),
        count: json::validation::USize64(count as u64),
        component_type: Checked::Valid(json::accessor::GenericComponentType(
            json::accessor::ComponentType::F32,
        )),
        type_: Checked::Valid(accessor_type),
        min: min_max.map(|(min, _)| serde_json::Value::Array(min.iter().map(|&v| serde_json::Value::from(v)).collect())),
        max: min_max.map(|(_, max)| serde_json::Value::Array(max.iter().map(|&v| serde_json::Value::from(v)).collect())),
        name: None,
        normalized: false,
        sparse: None,
        extensions: None,
        extras: Default::default(),
    });
    Some(acc_idx)
}


pub(crate) fn is_identity_or_zero(m: &[[f32; 4]; 3]) -> bool {
    let all_zero = m.iter().all(|row| row.iter().all(|&v| v == 0.0));
    if all_zero {
        return true;
    }
    let identity: [[f32; 4]; 3] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    for r in 0..3 {
        for c in 0..4 {
            if (m[r][c] - identity[r][c]).abs() > 1e-6 {
                return false;
            }
        }
    }
    true
}

pub fn write_glb(
    input: GlbInput,
    loaders: &mut GlbLoaders<'_>,
    opts: &GlbOptions,
) -> Result<Vec<u8>, Error> {
    write_glb_with_progress(input, loaders, opts, None)
}

pub fn write_glb_with_progress(
    input: GlbInput,
    loaders: &mut GlbLoaders<'_>,
    opts: &GlbOptions,
    progress: Option<&Progress>,
) -> Result<Vec<u8>, Error> {
    let mut builder = GlbBuilder::new();

    log::info!("[mem-phase] start write_glb");
    report_progress(progress, 0.05, "Packing root mesh");
    // ---- Pack root entity (drop textures after packing) ----
    let mut scene_nodes = if let Some(root_mesh) = input.root_mesh {
        let root_packed = builder.pack_mesh(
            &root_mesh,
            input.root_materials.as_ref(),
            input.root_textures.as_ref(),
            input.root_palette.as_ref(),
            None,
            opts.material_mode,
            opts.preserve_textureless_decal_primitives,
        );
        drop(input.root_textures);

        // ---- Build root NMC scene graph ----
        if let Some(nmc) = input.root_nmc.as_ref().filter(|n| !n.nodes.is_empty()) {
            let root_nodes = builder.build_nmc_hierarchy(&root_packed, nmc, &root_mesh.submeshes, true);
            root_nodes
                .iter()
                .map(|&i| json::Index::new(i))
                .collect::<Vec<_>>()
        } else {
            // Flat path — single node with root mesh
            builder.nodes_json.push(json::Node {
                mesh: Some(json::Index::new(root_packed.mesh_idx)),
                ..Default::default()
            });
            vec![json::Index::new(0)]
        }
    } else {
        Vec::new()
    };

    log::info!("[mem-phase] root packed, bin={}MB", builder.bin.len() / 1_048_576);
    report_progress(progress, 0.20, "Packing child meshes");
    // ---- Skeleton bone nodes ----
    builder.attach_skeleton_bones(&input.skeleton_bones, &scene_nodes);

    // ---- Attach child entities ----
    let num_children = input.children.len();
    for (i, child) in input.children.into_iter().enumerate() {
        builder.attach_child_entity(child, &scene_nodes, opts.material_mode, opts.fallback_palette.as_ref(), loaders.load_textures);
        if num_children > 0 {
            let fraction = (i + 1) as f32 / num_children as f32;
            report_progress(progress, 0.20 + 0.40 * fraction, "Packing child meshes");
        }
        if (i + 1) % 20 == 0 || i + 1 == num_children {
            log::info!("[mem-phase] children {}/{}, bin={}MB", i + 1, num_children, builder.bin.len() / 1_048_576);
        }
    }

    log::info!("[mem-phase] children done, bin={}MB", builder.bin.len() / 1_048_576);
    if num_children == 0 {
        report_progress(progress, 0.60, "Packing interiors");
    }
    // ---- Interior mesh instancing ----
    let (interior_scene_nodes, all_lights) = builder.attach_interiors(
        &input.interiors, opts.material_mode, opts.fallback_palette.as_ref(), loaders.load_textures, loaders.load_interior_mesh,
    );
    scene_nodes.extend(interior_scene_nodes);
    log::info!("[mem-phase] interiors done, bin={}MB", builder.bin.len() / 1_048_576);
    report_progress(progress, 0.80, "Finalizing GLB");

    // ---- Entity + palette extras on root node ----
    {
        let mut map = serde_json::Map::new();
        let include_root_entity_extras = opts.metadata.export_options.kind != "Decomposed";
        if include_root_entity_extras {
            if let Some(ref entity_name) = opts.metadata.entity_name {
                map.insert("entity_name".into(), serde_json::json!(entity_name));
                if let Some(ref gp) = opts.metadata.geometry_path {
                    map.insert("geometry_path".into(), serde_json::json!(gp));
                }
                if let Some(ref mp) = opts.metadata.material_path {
                    map.insert("material_path".into(), serde_json::json!(mp));
                }
            }
        }
        if let Some(ref pal) = input.root_palette {
            map.insert("tint_palette".into(), serde_json::json!({
                "primary": pal.primary,
                "secondary": pal.secondary,
                "tertiary": pal.tertiary,
                "glass": pal.glass
            }));
        }
        if !map.is_empty() {
            if let Some(first) = scene_nodes.first() {
                let idx = first.value() as usize;
                if idx < builder.nodes_json.len() {
                    builder.nodes_json[idx].extras = Some(
                        serde_json::value::RawValue::from_string(
                            serde_json::to_string(&serde_json::Value::Object(map))?
                        )?.into()
                    );
                }
            }
        }
    }

    report_progress(progress, 0.95, "Finalizing GLB");
    builder.finalize(scene_nodes, all_lights, &opts.metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Multiply a 3x4 row-major matrix A by a 3x4 row-major matrix B,
    /// treating B as a 4x4 matrix with an implicit `[0,0,0,1]` bottom row.
    fn mul_3x4(a: &[[f32; 4]; 3], b: &[[f32; 4]; 3]) -> [[f32; 4]; 3] {
        let mut result = [[0.0f32; 4]; 3];
        for r in 0..3 {
            for c in 0..3 {
                result[r][c] = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
            }
            result[r][3] = a[r][0] * b[0][3] + a[r][1] * b[1][3] + a[r][2] * b[2][3] + a[r][3];
        }
        result
    }
    use crate::nmc::NmcNode;
    use crate::types::SubMesh;

    fn triangle_mesh() -> Mesh {
        Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            uvs: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes: vec![SubMesh {
                material_name: Some("test".into()),
                material_id: 0,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            }],
            model_min: [0.0; 3],
            model_max: [1.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [1.0; 3],
        }
    }

    fn colored_triangle_mesh() -> Mesh {
        let mut mesh = triangle_mesh();
        mesh.colors = Some(vec![[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]);
        mesh
    }

    fn multi_uv_triangle_mesh() -> Mesh {
        let mut mesh = triangle_mesh();
        mesh.secondary_uvs = Some(vec![[0.5, 0.0], [1.0, 0.5], [0.5, 1.0]]);
        mesh
    }

    fn default_opts() -> GlbOptions {
        GlbOptions {
            material_mode: crate::pipeline::MaterialMode::None,
            preserve_textureless_decal_primitives: false,
            metadata: GlbMetadata {
                entity_name: None,
                geometry_path: None,
                material_path: None,
                export_options: ExportOptionsMetadata {
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
        }
    }

    fn textured_opts() -> GlbOptions {
        let mut opts = default_opts();
        opts.material_mode = crate::pipeline::MaterialMode::Textures;
        opts.metadata.export_options.material_mode = "Textures".to_string();
        opts
    }

    fn shared_png() -> Vec<u8> {
        vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0,
            0, 1, 8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156,
            99, 248, 255, 255, 63, 0, 5, 254, 2, 254, 167, 53, 129, 132, 0, 0, 0, 0, 73, 69,
            78, 68, 174, 66, 96, 130,
        ]
    }

    fn shared_material_file() -> crate::mtl::MtlFile {
        crate::mtl::MtlFile {
            materials: vec![crate::mtl::SubMaterial {
                name: "shared".into(),
                shader: "Illum".into(),
                diffuse: [1.0, 1.0, 1.0],
                opacity: 1.0,
                alpha_test: 0.0,
                string_gen_mask: String::new(),
                is_nodraw: false,
                specular: [0.04, 0.04, 0.04],
                shininess: 128.0,
                emissive: [0.0, 0.0, 0.0],
                glow: 0.0,
                surface_type: String::new(),
                diffuse_tex: Some("shared_diffuse.dds".into()),
                normal_tex: None,
                layers: Vec::new(),
                palette_tint: 0,
                texture_slots: vec![crate::mtl::TextureSlotBinding {
                    slot: "TexSlot1".into(),
                    path: "shared_diffuse.dds".into(),
                    is_virtual: false,
                }],
                public_params: Vec::new(),
                authored_attributes: Vec::new(),
                authored_textures: Vec::new(),
                authored_child_blocks: Vec::new(),
            }],
            source_path: Some("Data/Objects/shared.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        }
    }

    fn layered_material_file() -> crate::mtl::MtlFile {
        crate::mtl::MtlFile {
            materials: vec![crate::mtl::SubMaterial {
                name: "layered".into(),
                shader: "LayerBlend_V2".into(),
                diffuse: [1.0, 1.0, 1.0],
                opacity: 1.0,
                alpha_test: 0.0,
                string_gen_mask: "%STENCIL_MAP".into(),
                is_nodraw: false,
                specular: [0.04, 0.04, 0.04],
                shininess: 128.0,
                emissive: [0.0, 0.0, 0.0],
                glow: 0.0,
                surface_type: String::new(),
                diffuse_tex: None,
                normal_tex: None,
                layers: vec![
                    crate::mtl::MatLayer {
                        name: "Primary".into(),
                        path: "libs/materials/base_steel.mtl".into(),
                        sub_material: String::new(),
                        authored_attributes: vec![crate::mtl::AuthoredAttribute {
                            name: "CustomBlendMode".into(),
                            value: "Additive".into(),
                        }],
                        authored_child_blocks: vec![crate::mtl::AuthoredBlock {
                            tag: "CustomAnimation".into(),
                            attributes: vec![crate::mtl::AuthoredAttribute {
                                name: "Duration".into(),
                                value: "2.0".into(),
                            }],
                            children: Vec::new(),
                        }],
                        tint_color: [1.0, 1.0, 1.0],
                        wear_tint: [1.0, 1.0, 1.0],
                        palette_tint: 1,
                        gloss_mult: 1.0,
                        wear_gloss: 1.0,
                        uv_tiling: 1.0,
                        height_bias: 0.0,
                        height_scale: 1.0,
                        snapshot: None,
                        resolved_material: Some(crate::mtl::ResolvedLayerMaterial {
                            name: "base_steel".into(),
                            shader: "Layer".into(),
                            shader_family: "Layer".into(),
                            authored_attributes: vec![crate::mtl::AuthoredAttribute {
                                name: "MatTemplate".into(),
                                value: "steel_template".into(),
                            }],
                            public_params: vec![crate::mtl::PublicParam {
                                name: "WearGlossiness".into(),
                                value: "0.75".into(),
                            }],
                            authored_child_blocks: vec![crate::mtl::AuthoredBlock {
                                tag: "VertexDeform".into(),
                                attributes: vec![crate::mtl::AuthoredAttribute {
                                    name: "DividerY".into(),
                                    value: "0.25".into(),
                                }],
                                children: Vec::new(),
                            }],
                        }),
                    },
                    crate::mtl::MatLayer {
                        name: "Wear".into(),
                        path: "libs/materials/wear_scratches.mtl".into(),
                        sub_material: String::new(),
                        authored_attributes: Vec::new(),
                        authored_child_blocks: Vec::new(),
                        tint_color: [0.8, 0.7, 0.6],
                        wear_tint: [1.0, 1.0, 1.0],
                        palette_tint: 2,
                        gloss_mult: 1.0,
                        wear_gloss: 1.0,
                        uv_tiling: 2.5,
                        height_bias: 0.0,
                        height_scale: 1.0,
                        snapshot: None,
                        resolved_material: None,
                    },
                ],
                palette_tint: 1,
                texture_slots: vec![crate::mtl::TextureSlotBinding {
                    slot: "TexSlot7".into(),
                    path: "textures/stencil_mask.dds".into(),
                    is_virtual: false,
                }],
                public_params: vec![crate::mtl::PublicParam {
                    name: "WearAmount".into(),
                    value: "0.35".into(),
                }],
                authored_attributes: vec![crate::mtl::AuthoredAttribute {
                    name: "MtlFlags".into(),
                    value: "1024".into(),
                }],
                authored_textures: vec![crate::mtl::AuthoredTexture {
                    slot: "TexSlot7".into(),
                    path: "textures/stencil_mask.dds".into(),
                    is_virtual: false,
                    attributes: vec![crate::mtl::AuthoredAttribute {
                        name: "Map".into(),
                        value: "TexSlot7".into(),
                    }],
                    child_blocks: vec![crate::mtl::AuthoredBlock {
                        tag: "TexMod".into(),
                        attributes: vec![crate::mtl::AuthoredAttribute {
                            name: "TileU".into(),
                            value: "4".into(),
                        }],
                        children: Vec::new(),
                    }],
                }],
                authored_child_blocks: vec![crate::mtl::AuthoredBlock {
                    tag: "VertexDeform".into(),
                    attributes: vec![crate::mtl::AuthoredAttribute {
                        name: "DividerX".into(),
                        value: "0.5".into(),
                    }],
                    children: Vec::new(),
                }],
            }],
            source_path: Some("Data/Objects/layered.mtl".into()),
            paint_override: Some(crate::mtl::PaintOverrideInfo {
                paint_item_name: "paint_variant_gold".into(),
                subgeometry_tag: "VariantGold".into(),
                subgeometry_index: 1,
                material_path: Some("Data/Objects/layered_variant.mtl".into()),
            }),
            material_set: crate::mtl::MaterialSetAuthoredData {
                attributes: vec![crate::mtl::AuthoredAttribute {
                    name: "DefaultPalette".into(),
                    value: "vehicle.palette.rsi_zeus_cl".into(),
                }],
                public_params: vec![crate::mtl::PublicParam {
                    name: "RootGlowScale".into(),
                    value: "2.0".into(),
                }],
                child_blocks: vec![crate::mtl::AuthoredBlock {
                    tag: "VertexDeform".into(),
                    attributes: vec![crate::mtl::AuthoredAttribute {
                        name: "DividerY".into(),
                        value: "0.25".into(),
                    }],
                    children: Vec::new(),
                }],
            },
        }
    }

    fn named_palette() -> crate::mtl::TintPalette {
        crate::mtl::TintPalette {
            source_name: Some("vehicle.palette.rsi_zeus_cl".into()),
            display_name: Some("RSI Zeus CL".into()),
            primary: [0.1, 0.2, 0.3],
            secondary: [0.4, 0.5, 0.6],
            tertiary: [0.7, 0.8, 0.9],
            glass: [0.2, 0.3, 0.4],
            decal_color_r: None,
            decal_color_g: None,
            decal_color_b: None,
            decal_texture: None,
            finish: crate::mtl::TintPaletteFinish {
                primary: crate::mtl::TintPaletteFinishEntry {
                    specular: Some([0.6, 0.5, 0.4]),
                    glossiness: Some(0.8),
                },
                glass: crate::mtl::TintPaletteFinishEntry {
                    specular: Some([0.12, 0.22, 0.32]),
                    glossiness: Some(0.45),
                },
                ..Default::default()
            },
        }
    }

    fn alternate_palette() -> crate::mtl::TintPalette {
        crate::mtl::TintPalette {
            source_name: Some("vehicle.palette.variant_b".into()),
            display_name: Some("Variant B".into()),
            primary: [0.8, 0.3, 0.2],
            secondary: [0.2, 0.7, 0.6],
            tertiary: [0.4, 0.4, 0.9],
            glass: [0.6, 0.7, 0.8],
            decal_color_r: None,
            decal_color_g: None,
            decal_color_b: None,
            decal_texture: None,
            finish: Default::default(),
        }
    }

    fn phase_two_material_file() -> crate::mtl::MtlFile {
        crate::mtl::MtlFile {
            materials: vec![crate::mtl::SubMaterial {
                name: "glass_console".into(),
                shader: "GlassPBR".into(),
                diffuse: [0.6, 0.7, 0.8],
                opacity: 0.6,
                alpha_test: 0.0,
                string_gen_mask: "%STENCIL_MAP %PARALLAX_OCCLUSION_MAPPING".into(),
                is_nodraw: false,
                specular: [0.18, 0.22, 0.28],
                shininess: 196.0,
                emissive: [0.8, 0.4, 0.2],
                glow: 3.0,
                surface_type: "glass".into(),
                diffuse_tex: Some("textures/glass_console_diff.dds".into()),
                normal_tex: Some("textures/glass_console_ddna.dds".into()),
                layers: Vec::new(),
                palette_tint: 0,
                texture_slots: vec![crate::mtl::TextureSlotBinding {
                    slot: "TexSlot1".into(),
                    path: "textures/glass_console_diff.dds".into(),
                    is_virtual: false,
                }],
                public_params: vec![
                    crate::mtl::PublicParam {
                        name: "IOR".into(),
                        value: "1.65".into(),
                    },
                    crate::mtl::PublicParam {
                        name: "Thickness".into(),
                        value: "0.12".into(),
                    },
                    crate::mtl::PublicParam {
                        name: "AttenuationDistance".into(),
                        value: "0.75".into(),
                    },
                ],
                authored_attributes: Vec::new(),
                authored_textures: Vec::new(),
                authored_child_blocks: Vec::new(),
            }],
            source_path: Some("Data/Objects/phase_two_glass.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        }
    }

    fn transparent_material_file() -> crate::mtl::MtlFile {
        crate::mtl::MtlFile {
            materials: vec![crate::mtl::SubMaterial {
                name: "hidden_panel".into(),
                shader: "HardSurface".into(),
                diffuse: [1.0, 1.0, 1.0],
                opacity: 0.0,
                alpha_test: 0.0,
                string_gen_mask: String::new(),
                is_nodraw: false,
                specular: [0.04, 0.04, 0.04],
                shininess: 128.0,
                emissive: [0.0, 0.0, 0.0],
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
            }],
            source_path: Some("Data/Objects/hidden_panel.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        }
    }

    fn screen_material_file() -> crate::mtl::MtlFile {
        crate::mtl::MtlFile {
            materials: vec![crate::mtl::SubMaterial {
                name: "mfd_screen".into(),
                shader: "DisplayScreen".into(),
                diffuse: [0.1, 0.2, 0.3],
                opacity: 1.0,
                alpha_test: 0.0,
                string_gen_mask: String::new(),
                is_nodraw: false,
                specular: [0.04, 0.04, 0.04],
                shininess: 32.0,
                emissive: [0.2, 0.6, 0.9],
                glow: 1.5,
                surface_type: String::new(),
                diffuse_tex: None,
                normal_tex: None,
                layers: Vec::new(),
                palette_tint: 0,
                texture_slots: vec![crate::mtl::TextureSlotBinding {
                    slot: "TexSlot3".into(),
                    path: "$RenderToTexture".into(),
                    is_virtual: true,
                }],
                public_params: vec![crate::mtl::PublicParam {
                    name: "BackColour".into(),
                    value: "0.1,0.2,0.3".into(),
                }],
                authored_attributes: Vec::new(),
                authored_textures: Vec::new(),
                authored_child_blocks: Vec::new(),
            }],
            source_path: Some("Data/Objects/mfd_screen.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        }
    }

    fn semantic_variant_material_file(pattern_index: &str) -> crate::mtl::MtlFile {
        crate::mtl::MtlFile {
            materials: vec![crate::mtl::SubMaterial {
                name: "shared".into(),
                shader: "Illum".into(),
                diffuse: [1.0, 1.0, 1.0],
                opacity: 1.0,
                alpha_test: 0.0,
                string_gen_mask: String::new(),
                is_nodraw: false,
                specular: [0.04, 0.04, 0.04],
                shininess: 128.0,
                emissive: [0.0, 0.0, 0.0],
                glow: 0.0,
                surface_type: String::new(),
                diffuse_tex: Some("shared_diffuse.dds".into()),
                normal_tex: None,
                layers: Vec::new(),
                palette_tint: 0,
                texture_slots: vec![crate::mtl::TextureSlotBinding {
                    slot: "TexSlot1".into(),
                    path: "shared_diffuse.dds".into(),
                    is_virtual: false,
                }],
                public_params: vec![crate::mtl::PublicParam {
                    name: "PatternIndex".into(),
                    value: pattern_index.into(),
                }],
                authored_attributes: Vec::new(),
                authored_textures: Vec::new(),
                authored_child_blocks: Vec::new(),
            }],
            source_path: Some("Data/Objects/shared.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        }
    }

    fn nodraw_material_file() -> crate::mtl::MtlFile {
        crate::mtl::MtlFile {
            materials: vec![crate::mtl::SubMaterial {
                name: "nodraw_panel".into(),
                shader: "NoDraw".into(),
                diffuse: [1.0, 1.0, 1.0],
                opacity: 1.0,
                alpha_test: 0.0,
                string_gen_mask: String::new(),
                is_nodraw: true,
                specular: [0.04, 0.04, 0.04],
                shininess: 128.0,
                emissive: [0.0, 0.0, 0.0],
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
            }],
            source_path: Some("Data/Objects/nodraw.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        }
    }

    fn decal_material_file() -> crate::mtl::MtlFile {
        crate::mtl::MtlFile {
            materials: vec![crate::mtl::SubMaterial {
                name: "livery_decal".into(),
                shader: "MeshDecal".into(),
                diffuse: [1.0, 1.0, 1.0],
                opacity: 1.0,
                alpha_test: 0.0,
                string_gen_mask: "%DECAL".into(),
                is_nodraw: false,
                specular: [0.04, 0.04, 0.04],
                shininess: 128.0,
                emissive: [0.0, 0.0, 0.0],
                glow: 0.0,
                surface_type: String::new(),
                diffuse_tex: Some("textures/livery_decal.dds".into()),
                normal_tex: None,
                layers: Vec::new(),
                palette_tint: 0,
                texture_slots: vec![crate::mtl::TextureSlotBinding {
                    slot: "TexSlot1".into(),
                    path: "textures/livery_decal.dds".into(),
                    is_virtual: false,
                }],
                public_params: Vec::new(),
                authored_attributes: Vec::new(),
                authored_textures: Vec::new(),
                authored_child_blocks: Vec::new(),
            }],
            source_path: Some("Data/Objects/decal.mtl".into()),
            paint_override: None,
            material_set: Default::default(),
        }
    }

    fn shared_textures() -> crate::types::MaterialTextures {
        crate::types::MaterialTextures {
            diffuse: vec![Some(shared_png())],
            normal: vec![None],
            roughness: vec![None],
            emissive: vec![None],
            occlusion: vec![None],
            diffuse_transform: vec![None],
            normal_transform: vec![None],
            roughness_transform: vec![None],
            emissive_transform: vec![None],
            occlusion_transform: vec![None],
            bundled_fallbacks: vec![Vec::new()],
        }
    }

    fn phase_two_textures() -> crate::types::MaterialTextures {
        crate::types::MaterialTextures {
            diffuse: vec![Some(shared_png())],
            normal: vec![Some(shared_png())],
            roughness: vec![Some(shared_png())],
            emissive: vec![Some(shared_png())],
            occlusion: vec![Some(shared_png())],
            diffuse_transform: vec![Some(crate::types::TextureTransformInfo {
                scale: [2.0, 1.5],
                tex_coord: 1,
            })],
            normal_transform: vec![Some(crate::types::TextureTransformInfo {
                scale: [1.0, 1.0],
                tex_coord: 0,
            })],
            roughness_transform: vec![Some(crate::types::TextureTransformInfo {
                scale: [2.0, 1.5],
                tex_coord: 1,
            })],
            emissive_transform: vec![Some(crate::types::TextureTransformInfo {
                scale: [1.25, 1.25],
                tex_coord: 1,
            })],
            occlusion_transform: vec![Some(crate::types::TextureTransformInfo {
                scale: [1.0, 1.0],
                tex_coord: 1,
            })],
            bundled_fallbacks: vec![vec![
                "stencil_fallback".into(),
                "screen_emissive_placeholder".into(),
                "occlusion_from_mask".into(),
            ]],
        }
    }

    fn uv0_transform_textures() -> crate::types::MaterialTextures {
        crate::types::MaterialTextures {
            diffuse: vec![Some(shared_png())],
            normal: vec![None],
            roughness: vec![None],
            emissive: vec![None],
            occlusion: vec![None],
            diffuse_transform: vec![Some(crate::types::TextureTransformInfo {
                scale: [2.0, 1.5],
                tex_coord: 0,
            })],
            normal_transform: vec![None],
            roughness_transform: vec![None],
            emissive_transform: vec![None],
            occlusion_transform: vec![None],
            bundled_fallbacks: vec![Vec::new()],
        }
    }

    fn screen_textures() -> crate::types::MaterialTextures {
        crate::types::MaterialTextures {
            diffuse: vec![Some(shared_png())],
            normal: vec![None],
            roughness: vec![None],
            emissive: vec![Some(shared_png())],
            occlusion: vec![None],
            diffuse_transform: vec![Some(crate::types::TextureTransformInfo {
                scale: [4.0, 3.0],
                tex_coord: 1,
            })],
            normal_transform: vec![None],
            roughness_transform: vec![None],
            emissive_transform: vec![Some(crate::types::TextureTransformInfo {
                scale: [4.0, 3.0],
                tex_coord: 1,
            })],
            occlusion_transform: vec![None],
            bundled_fallbacks: vec![vec![
                "rtt_placeholder".into(),
                "screen_emissive_placeholder".into(),
            ]],
        }
    }

    fn child_entity(entity_name: &str, materials: crate::mtl::MtlFile) -> crate::types::EntityPayload {
        crate::types::EntityPayload {
            mesh: triangle_mesh(),
            materials: Some(materials),
            textures: None,
            nmc: None,
            palette: None,
            geometry_path: format!("Data/Objects/{entity_name}.skin"),
            material_path: format!("Data/Objects/{entity_name}.mtl"),
            bones: Vec::new(),
            entity_name: entity_name.into(),
            parent_node_name: String::new(),
            parent_entity_name: String::new(),
            no_rotation: false,
            offset_position: [0.0; 3],
            offset_rotation: [0.0; 3],
        }
    }

    fn glb_json(glb: &[u8]) -> serde_json::Value {
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json_bytes = &glb[20..20 + json_len];
        let json_str = std::str::from_utf8(json_bytes).expect("JSON not valid UTF-8");
        serde_json::from_str(json_str).expect("GLB JSON should parse")
    }

    fn glb_root(glb: &[u8]) -> json::Root {
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json_bytes = &glb[20..20 + json_len];
        json::Root::from_slice(json_bytes).expect("GLB root JSON should parse")
    }

    fn material_snapshot(material: &serde_json::Value) -> serde_json::Value {
        let semantic = &material["extras"]["semantic"];
        serde_json::json!({
            "alphaMode": material["alphaMode"],
            "hasBaseTransform": material["pbrMetallicRoughness"]["baseColorTexture"]["extensions"]["KHR_texture_transform"].is_object(),
            "hasEmissiveTexture": material["emissiveTexture"].is_object(),
            "hasOcclusionTexture": material["occlusionTexture"].is_object(),
            "semantic": {
                "shader_family": semantic["shader_family"],
                "is_decal": semantic["is_decal"],
                "is_glass": semantic["is_glass"],
                "activation_state": semantic["activation_state"],
                "bundled_fallbacks": semantic["bundled_fallbacks"],
                "palette_channel": semantic["palette"]["material_channel"]["name"],
                "layer_manifest_len": semantic["layer_manifest"].as_array().map_or(0, |items| items.len()),
            },
        })
    }

    fn call_write_glb(mesh: Mesh) -> Result<Vec<u8>, crate::error::Error> {
        write_glb(
            GlbInput {
                root_mesh: Some(mesh),
                root_materials: None,
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &default_opts(),
        )
    }

    fn write_glb_simple(mesh: &Mesh, nmc: Option<&NodeMeshCombo>) -> Result<Vec<u8>, crate::error::Error> {
        write_glb(
            GlbInput {
                root_mesh: Some(mesh.clone()),
                root_materials: None,
                root_textures: None,
                root_nmc: nmc.cloned(),
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &default_opts(),
        )
    }

    #[test]
    fn write_glb_produces_valid_header() {
        let glb = call_write_glb(triangle_mesh()).expect("write_glb failed");
        assert!(glb.len() >= 12);
        assert_eq!(&glb[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(glb[4..8].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize,
            glb.len()
        );
    }

    #[test]
    fn write_glb_has_json_and_bin_chunks() {
        let glb = call_write_glb(triangle_mesh()).expect("write_glb failed");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json_type = u32::from_le_bytes(glb[16..20].try_into().unwrap());
        assert_eq!(json_type, 0x4E4F534A, "first chunk should be JSON");

        let json_bytes = &glb[20..20 + json_len];
        let json_str = std::str::from_utf8(json_bytes).expect("JSON not valid UTF-8");
        assert!(
            json_str.contains("\"meshes\""),
            "JSON should contain meshes"
        );

        let bin_offset = 20 + json_len;
        if glb.len() > bin_offset + 8 {
            let bin_type = u32::from_le_bytes(glb[bin_offset + 4..bin_offset + 8].try_into().unwrap());
            assert_eq!(bin_type, 0x004E4942, "second chunk should be BIN");
        }
    }

    #[test]
    fn write_glb_exports_color_0_when_vertex_colors_exist() {
        let glb = call_write_glb(colored_triangle_mesh()).expect("write_glb failed");
        let root = glb_json(&glb);
        let attributes = &root["meshes"][0]["primitives"][0]["attributes"];
        assert!(attributes.get("COLOR_0").is_some(), "vertex colors should export as COLOR_0");
    }

    #[test]
    fn write_glb_exports_texcoord_1_when_second_uvs_exist() {
        let glb = call_write_glb(multi_uv_triangle_mesh()).expect("write_glb failed");
        let root = glb_json(&glb);
        let attributes = &root["meshes"][0]["primitives"][0]["attributes"];
        assert!(attributes.get("TEXCOORD_1").is_some(), "second UVs should export as TEXCOORD_1");
    }

    #[test]
    fn write_glb_deduplicates_shared_child_materials_and_textures() {
        let shared_materials = shared_material_file();
        let children = vec![
            child_entity("child_a", shared_materials.clone()),
            child_entity("child_b", shared_materials),
        ];

        let glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: None,
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children,
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| Some(shared_textures()),
                load_interior_mesh: &mut |_| None,
            },
            &textured_opts(),
        )
        .expect("write_glb failed");

        let json = glb_json(&glb);
        let material_count = json["materials"].as_array().map_or(0, |a| a.len());
        let texture_count = json["textures"].as_array().map_or(0, |a| a.len());
        let image_count = json["images"].as_array().map_or(0, |a| a.len());
        let mesh_count = json["meshes"].as_array().map_or(0, |a| a.len());

        // Root mesh contributes one default material. The two identical child entities
        // should share one glTF material, one glTF texture, one glTF image, and one glTF mesh.
        assert_eq!(
            (material_count, texture_count, image_count, mesh_count),
            (2, 1, 1, 2),
            "expected shared child material/texture/image/mesh entries to dedupe across children",
        );
    }

    #[test]
    fn write_glb_preserves_layer_manifest_order_and_values() {
        let glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(layered_material_file()),
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &default_opts(),
        )
        .expect("write_glb failed");

        let json = glb_json(&glb);
        let manifest = &json["materials"][0]["extras"]["semantic"]["layer_manifest"];
        let layers = manifest
            .as_array()
            .expect("layer manifest should be present as an array");

        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0]["index"], serde_json::json!(0));
        assert_eq!(layers[0]["path"], serde_json::json!("libs/materials/base_steel.mtl"));
        assert_eq!(layers[0]["resolved_material"]["shader_family"], serde_json::json!("Layer"));
        assert_eq!(layers[0]["resolved_material"]["authored_attributes"][0]["name"], serde_json::json!("MatTemplate"));
        assert_eq!(layers[0]["authored_attributes"][0]["name"], serde_json::json!("CustomBlendMode"));
        assert_eq!(layers[0]["authored_child_blocks"][0]["tag"], serde_json::json!("CustomAnimation"));
        assert_eq!(layers[0]["palette_tint"], serde_json::json!(1));
        assert_eq!(layers[1]["index"], serde_json::json!(1));
        assert_eq!(layers[1]["path"], serde_json::json!("libs/materials/wear_scratches.mtl"));
        let tint = layers[1]["tint_color"]
            .as_array()
            .expect("layer tint_color should be an array");
        assert_eq!(tint.len(), 3);
        assert!((tint[0].as_f64().unwrap() - 0.8).abs() < 1e-5);
        assert!((tint[1].as_f64().unwrap() - 0.7).abs() < 1e-5);
        assert!((tint[2].as_f64().unwrap() - 0.6).abs() < 1e-5);
        assert_eq!(layers[1]["palette_tint"], serde_json::json!(2));
        assert_eq!(layers[1]["uv_tiling"], serde_json::json!(2.5));
    }

    #[test]
    fn write_glb_preserves_palette_routing_and_material_set_identity() {
        let glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(layered_material_file()),
                root_textures: None,
                root_nmc: None,
                root_palette: Some(named_palette()),
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &default_opts(),
        )
        .expect("write_glb failed");

        let json = glb_json(&glb);
        let semantic = &json["materials"][0]["extras"]["semantic"];
        let palette = &semantic["palette"];
        let palette_layers = palette["layers"]
            .as_array()
            .expect("palette layer routing should be present");
        let resolved_color = palette["resolved_color"]
            .as_array()
            .expect("resolved palette color should be present");
        let resolved_finish = palette["resolved_finish"]
            .as_object()
            .expect("resolved palette finish should be present");
        let material_set_identity = &semantic["material_set_identity"];
        let paint_override = &semantic["paint_override"];

        assert_eq!(palette["source_name"], serde_json::json!("vehicle.palette.rsi_zeus_cl"));
        assert_eq!(palette["material_channel"]["index"], serde_json::json!(1));
        assert_eq!(palette["material_channel"]["name"], serde_json::json!("primary"));
        assert_eq!(resolved_color.len(), 3);
        assert!((resolved_color[0].as_f64().unwrap() - 0.1).abs() < 1e-5);
        assert!((resolved_color[1].as_f64().unwrap() - 0.2).abs() < 1e-5);
        assert!((resolved_color[2].as_f64().unwrap() - 0.3).abs() < 1e-5);
        let resolved_finish_specular = resolved_finish["specular"]
            .as_array()
            .expect("resolved palette finish specular should be present");
        assert_eq!(resolved_finish_specular.len(), 3);
        assert!((resolved_finish_specular[0].as_f64().unwrap() - 0.6).abs() < 1e-5);
        assert!((resolved_finish_specular[1].as_f64().unwrap() - 0.5).abs() < 1e-5);
        assert!((resolved_finish_specular[2].as_f64().unwrap() - 0.4).abs() < 1e-5);
        assert!((resolved_finish["glossiness"].as_f64().unwrap() - 0.8).abs() < 1e-5);
        assert_eq!(palette_layers.len(), 2);
        assert_eq!(palette_layers[0]["index"], serde_json::json!(0));
        assert_eq!(palette_layers[0]["channel"]["name"], serde_json::json!("primary"));
        let layer_finish_specular = palette_layers[0]["resolved_finish"]["specular"]
            .as_array()
            .expect("layer finish specular should be present");
        assert_eq!(layer_finish_specular.len(), 3);
        assert!((layer_finish_specular[0].as_f64().unwrap() - 0.6).abs() < 1e-5);
        assert!((layer_finish_specular[1].as_f64().unwrap() - 0.5).abs() < 1e-5);
        assert!((layer_finish_specular[2].as_f64().unwrap() - 0.4).abs() < 1e-5);
        assert!((palette_layers[0]["resolved_finish"]["glossiness"].as_f64().unwrap() - 0.8).abs() < 1e-5);
        assert_eq!(palette_layers[1]["index"], serde_json::json!(1));
        assert_eq!(palette_layers[1]["channel"]["name"], serde_json::json!("secondary"));
        assert!(palette_layers[1]["resolved_finish"].is_null());

        assert_eq!(material_set_identity["source_path"], serde_json::json!("Data/Objects/layered.mtl"));
        assert_eq!(material_set_identity["source_stem"], serde_json::json!("layered"));
        assert_eq!(material_set_identity["submaterial_index"], serde_json::json!(0));
        assert_eq!(material_set_identity["submaterial_name"], serde_json::json!("layered"));
        assert_eq!(material_set_identity["slot_name"], serde_json::json!("test"));
        assert_eq!(paint_override["subgeometry_tag"], serde_json::json!("VariantGold"));
        assert_eq!(paint_override["paint_item_name"], serde_json::json!("paint_variant_gold"));
        assert_eq!(semantic["authored_material_set"]["attributes"][0]["name"], serde_json::json!("DefaultPalette"));
        assert_eq!(semantic["authored_material_set"]["public_params"][0]["name"], serde_json::json!("RootGlowScale"));
        assert_eq!(semantic["authored_material_set"]["child_blocks"][0]["tag"], serde_json::json!("VertexDeform"));
        assert_eq!(semantic["authored_attributes"][0]["name"], serde_json::json!("MtlFlags"));
        assert_eq!(semantic["authored_public_params"][0]["name"], serde_json::json!("WearAmount"));
        assert_eq!(semantic["authored_child_blocks"][0]["tag"], serde_json::json!("VertexDeform"));
        assert_eq!(semantic["texture_slots"][0]["authored_child_blocks"][0]["tag"], serde_json::json!("TexMod"));
        assert_eq!(semantic["activation_state"]["state"], serde_json::json!("active"));
        assert_eq!(semantic["activation_state"]["reason"], serde_json::json!("visible"));
    }

    #[test]
    fn write_glb_emits_phase_two_material_extensions_and_texture_transforms() {
        let mut opts = textured_opts();
        opts.material_mode = crate::pipeline::MaterialMode::All;
        opts.metadata.export_options.material_mode = "All".to_string();

        let glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(phase_two_material_file()),
                root_textures: Some(phase_two_textures()),
                root_nmc: None,
                root_palette: Some(named_palette()),
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &opts,
        )
        .expect("write_glb failed");

        let json = glb_json(&glb);
        let extensions_used = json["extensionsUsed"]
            .as_array()
            .expect("extensionsUsed should be present");
        for extension in [
            "KHR_materials_transmission",
            "KHR_materials_ior",
            "KHR_materials_volume",
            "KHR_materials_emissive_strength",
            "KHR_texture_transform",
        ] {
            assert!(
                extensions_used.iter().any(|value| value.as_str() == Some(extension)),
                "expected {extension} in extensionsUsed"
            );
        }

        let material = &json["materials"][0];
        let base_transform = &material["pbrMetallicRoughness"]["baseColorTexture"]["extensions"]["KHR_texture_transform"];
        let base_scale = base_transform["scale"]
            .as_array()
            .expect("base color texture transform scale should be present");
        assert_eq!(base_transform["texCoord"], serde_json::json!(1));
        assert!((base_scale[0].as_f64().unwrap() - 2.0).abs() < 1e-6);
        assert!((base_scale[1].as_f64().unwrap() - 1.5).abs() < 1e-6);

        let emissive_transform = &material["emissiveTexture"]["extensions"]["KHR_texture_transform"];
        let emissive_scale = emissive_transform["scale"]
            .as_array()
            .expect("emissive texture transform scale should be present");
        assert_eq!(emissive_transform["texCoord"], serde_json::json!(1));
        assert!((emissive_scale[0].as_f64().unwrap() - 1.25).abs() < 1e-6);
        assert!((emissive_scale[1].as_f64().unwrap() - 1.25).abs() < 1e-6);

        assert_eq!(material["normalTexture"]["texCoord"], serde_json::json!(0));
        assert_eq!(material["occlusionTexture"]["texCoord"], serde_json::json!(1));

        let palette = &material["extras"]["semantic"]["palette"];
        let palette_color = palette["resolved_color"]
            .as_array()
            .expect("glass palette color should be present");
        let palette_finish = palette["resolved_finish"]
            .as_object()
            .expect("glass palette finish should be present");
        let palette_finish_specular = palette_finish["specular"]
            .as_array()
            .expect("glass palette finish specular should be present");

        assert_eq!(palette["material_channel"]["index"], serde_json::json!(0));
        assert_eq!(palette["material_channel"]["name"], serde_json::json!("glass"));
        assert_eq!(palette_color.len(), 3);
        assert!((palette_color[0].as_f64().unwrap() - 0.2).abs() < 1e-5);
        assert!((palette_color[1].as_f64().unwrap() - 0.3).abs() < 1e-5);
        assert!((palette_color[2].as_f64().unwrap() - 0.4).abs() < 1e-5);
        assert_eq!(palette_finish_specular.len(), 3);
        assert!((palette_finish_specular[0].as_f64().unwrap() - 0.12).abs() < 1e-5);
        assert!((palette_finish_specular[1].as_f64().unwrap() - 0.22).abs() < 1e-5);
        assert!((palette_finish_specular[2].as_f64().unwrap() - 0.32).abs() < 1e-5);
        assert!((palette_finish["glossiness"].as_f64().unwrap() - 0.45).abs() < 1e-5);

        let emissive_strength = material["extensions"]["KHR_materials_emissive_strength"]["emissiveStrength"]
            .as_f64()
            .expect("emissive strength should be present");
        assert!((emissive_strength - 2.4).abs() < 1e-6);
        assert!((material["extensions"]["KHR_materials_ior"]["ior"].as_f64().unwrap() - 1.65).abs() < 1e-6);
        assert!((material["extensions"]["KHR_materials_volume"]["thicknessFactor"].as_f64().unwrap() - 0.12).abs() < 1e-6);
        assert!((material["extensions"]["KHR_materials_volume"]["attenuationDistance"].as_f64().unwrap() - 0.75).abs() < 1e-6);

        let fallbacks = material["extras"]["semantic"]["bundled_fallbacks"]
            .as_array()
            .expect("bundled fallback tags should be present");
        let fallback_names: Vec<&str> = fallbacks.iter().filter_map(|value| value.as_str()).collect();
        assert!(fallback_names.contains(&"stencil_fallback"));
        assert!(fallback_names.contains(&"screen_emissive_placeholder"));
        assert!(fallback_names.contains(&"occlusion_from_mask"));
    }

    #[test]
    fn write_glb_omits_null_texcoord_from_uv0_texture_transform_extensions() {
        let glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(shared_material_file()),
                root_textures: Some(uv0_transform_textures()),
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &textured_opts(),
        )
        .expect("write_glb failed");

        let json = glb_json(&glb);
        let transform = &json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"]["KHR_texture_transform"];
        let transform_object = transform
            .as_object()
            .expect("base color texture transform should be present");

        assert_eq!(json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["texCoord"], serde_json::json!(0));
        assert!(
            transform_object.get("texCoord").is_none(),
            "UV0 texture transforms must omit texCoord instead of serializing null"
        );
    }

    #[test]
    fn write_glb_preserves_inactive_submaterial_activation_state() {
        let glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(nodraw_material_file()),
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &default_opts(),
        )
        .expect("write_glb failed");

        let json = glb_json(&glb);
        let semantic = &json["materials"][0]["extras"]["semantic"];
        assert_eq!(semantic["activation_state"]["state"], serde_json::json!("inactive"));
        assert_eq!(semantic["activation_state"]["reason"], serde_json::json!("nodraw"));
    }

    #[test]
    fn write_glb_keeps_fully_transparent_materials_non_rendering() {
        let glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(transparent_material_file()),
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &default_opts(),
        )
        .expect("write_glb failed");

        let json = glb_json(&glb);
        let semantic = &json["materials"][0]["extras"]["semantic"];
        let primitives = json["meshes"][0]["primitives"]
            .as_array()
            .expect("mesh primitives should be present");

        assert!(primitives.is_empty(), "fully transparent materials should not emit renderable primitives");
        assert_eq!(semantic["activation_state"]["state"], serde_json::json!("inactive"));
        assert_eq!(semantic["activation_state"]["reason"], serde_json::json!("semantic_hidden"));
    }

    #[test]
    fn write_glb_marks_textureless_bundled_decals_inactive() {
        let glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(decal_material_file()),
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &default_opts(),
        )
        .expect("write_glb failed");

        let json = glb_json(&glb);
        let semantic = &json["materials"][0]["extras"]["semantic"];
        assert_eq!(semantic["activation_state"]["state"], serde_json::json!("inactive"));
        assert_eq!(
            semantic["activation_state"]["reason"],
            serde_json::json!("missing_base_color_texture"),
        );
    }

    #[test]
    fn write_glb_preserves_textureless_decals_when_requested() {
        let mut opts = default_opts();
        opts.preserve_textureless_decal_primitives = true;
        opts.metadata.export_options.kind = "Decomposed".to_string();

        let glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(decal_material_file()),
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &opts,
        )
        .expect("write_glb failed");

        let json = glb_json(&glb);
        let semantic = &json["materials"][0]["extras"]["semantic"];
        let primitives = json["meshes"][0]["primitives"]
            .as_array()
            .expect("mesh primitives should be present");

        assert_eq!(primitives.len(), 1, "decomposed GLBs should retain decal primitives for sidecar reconstruction");
        assert_eq!(semantic["activation_state"]["state"], serde_json::json!("active"));
        assert_eq!(semantic["activation_state"]["reason"], serde_json::json!("visible"));
    }

    #[test]
    fn write_glb_keeps_materials_distinct_when_semantic_metadata_differs() {
        let children = vec![
            child_entity("child_a", semantic_variant_material_file("1")),
            child_entity("child_b", semantic_variant_material_file("2")),
        ];

        let glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: None,
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children,
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| Some(shared_textures()),
                load_interior_mesh: &mut |_| None,
            },
            &textured_opts(),
        )
        .expect("write_glb failed");

        let json = glb_json(&glb);
        let material_count = json["materials"].as_array().map_or(0, |a| a.len());
        let texture_count = json["textures"].as_array().map_or(0, |a| a.len());
        let image_count = json["images"].as_array().map_or(0, |a| a.len());
        let mesh_count = json["meshes"].as_array().map_or(0, |a| a.len());
        let pattern_indices: Vec<i64> = json["materials"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|material| {
                material["extras"]["semantic"]["public_params"]["PatternIndex"].as_i64()
            })
            .collect();

        assert_eq!(
            (material_count, texture_count, image_count, mesh_count),
            (3, 1, 1, 3),
            "semantic metadata differences should prevent child materials and meshes from deduping",
        );
        assert_eq!(pattern_indices.len(), 2);
        assert!(pattern_indices.contains(&1));
        assert!(pattern_indices.contains(&2));
    }

    #[test]
    fn write_glb_reuses_geometry_for_palette_variants() {
        let mut child_a = child_entity("child_a", layered_material_file());
        child_a.palette = Some(named_palette());
        let mut child_b = child_entity("child_b", layered_material_file());
        child_b.palette = Some(alternate_palette());

        let mut opts = default_opts();
        opts.material_mode = crate::pipeline::MaterialMode::Colors;
        opts.metadata.export_options.material_mode = "Colors".to_string();

        let glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: None,
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: vec![child_a, child_b],
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &opts,
        )
        .expect("write_glb failed");

        let json = glb_json(&glb);
        let accessor_count = json["accessors"].as_array().map_or(0, |items| items.len());
        let buffer_view_count = json["bufferViews"].as_array().map_or(0, |items| items.len());
        let mesh_count = json["meshes"].as_array().map_or(0, |items| items.len());

        assert_eq!(mesh_count, 3, "root plus two palette-specific meshes should still be emitted");
        assert_eq!(accessor_count, 3, "palette variants should reuse shared geometry accessors instead of duplicating them");
        assert_eq!(buffer_view_count, 3, "palette variants should reuse shared geometry buffer views instead of duplicating them");
    }

    #[test]
    fn write_glb_decomposed_output_ignores_instance_metadata() {
        let mut first_opts = default_opts();
        first_opts.metadata.entity_name = Some("EntityVariantA".into());
        first_opts.metadata.geometry_path = Some("Data/Objects/test_variant_a.skin".into());
        first_opts.metadata.material_path = Some("Data/Objects/test_variant_a.mtl".into());
        first_opts.metadata.export_options.kind = "Decomposed".to_string();

        let mut second_opts = default_opts();
        second_opts.metadata.entity_name = Some("EntityVariantB".into());
        second_opts.metadata.geometry_path = Some("Data/Objects/test_variant_b.skin".into());
        second_opts.metadata.material_path = Some("Data/Objects/test_variant_b.mtl".into());
        second_opts.metadata.export_options.kind = "Decomposed".to_string();

        let first_glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(shared_material_file()),
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &first_opts,
        )
        .expect("write_glb failed");

        let second_glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(shared_material_file()),
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &second_opts,
        )
        .expect("write_glb failed");

        assert_eq!(first_glb, second_glb, "decomposed GLBs should stay byte-identical when only instance metadata changes");

        let json = glb_json(&first_glb);
        assert!(json["asset"]["extras"]["export_timestamp_unix"].is_null(), "decomposed GLBs should omit volatile export timestamps");
        let node_with_extras = json["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|node| node["extras"].is_object());
        assert!(node_with_extras.is_none(), "decomposed GLBs should not embed per-instance root entity metadata");
    }

    #[test]
    fn write_glb_representative_material_snapshots_stay_stable() {
        let mut all_opts = textured_opts();
        all_opts.material_mode = crate::pipeline::MaterialMode::All;
        all_opts.metadata.export_options.material_mode = "All".to_string();

        let ship_glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(phase_two_material_file()),
                root_textures: Some(phase_two_textures()),
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &all_opts,
        )
        .expect("ship write_glb failed");

        let mut color_opts = default_opts();
        color_opts.material_mode = crate::pipeline::MaterialMode::Colors;
        color_opts.metadata.export_options.material_mode = "Colors".to_string();

        let rover_glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(layered_material_file()),
                root_textures: None,
                root_nmc: None,
                root_palette: Some(named_palette()),
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &color_opts,
        )
        .expect("rover write_glb failed");

        let wearable_glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(decal_material_file()),
                root_textures: Some(shared_textures()),
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &textured_opts(),
        )
        .expect("wearable write_glb failed");

        let environment_glb = write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: Some(screen_material_file()),
                root_textures: Some(screen_textures()),
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: Vec::new(),
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| None,
                load_interior_mesh: &mut |_| None,
            },
            &textured_opts(),
        )
        .expect("environment write_glb failed");

        let ship_snapshot = material_snapshot(&glb_json(&ship_glb)["materials"][0]);
        let rover_snapshot = material_snapshot(&glb_json(&rover_glb)["materials"][0]);
        let wearable_snapshot = material_snapshot(&glb_json(&wearable_glb)["materials"][0]);
        let environment_snapshot = material_snapshot(&glb_json(&environment_glb)["materials"][0]);

        assert_eq!(
            ship_snapshot,
            serde_json::json!({
                "alphaMode": "BLEND",
                "hasBaseTransform": true,
                "hasEmissiveTexture": true,
                "hasOcclusionTexture": true,
                "semantic": {
                    "shader_family": "GlassPBR",
                    "is_decal": false,
                    "is_glass": true,
                    "activation_state": {"state": "active", "reason": "visible"},
                    "bundled_fallbacks": ["stencil_fallback", "screen_emissive_placeholder", "occlusion_from_mask"],
                    "palette_channel": "glass",
                    "layer_manifest_len": 0,
                },
            }),
        );
        assert_eq!(
            rover_snapshot,
            serde_json::json!({
                "alphaMode": "OPAQUE",
                "hasBaseTransform": false,
                "hasEmissiveTexture": false,
                "hasOcclusionTexture": false,
                "semantic": {
                    "shader_family": "LayerBlend_V2",
                    "is_decal": false,
                    "is_glass": false,
                    "activation_state": {"state": "active", "reason": "visible"},
                    "bundled_fallbacks": serde_json::Value::Null,
                    "palette_channel": "primary",
                    "layer_manifest_len": 2,
                },
            }),
        );
        assert_eq!(
            wearable_snapshot,
            serde_json::json!({
                "alphaMode": "BLEND",
                "hasBaseTransform": false,
                "hasEmissiveTexture": false,
                "hasOcclusionTexture": false,
                "semantic": {
                    "shader_family": "MeshDecal",
                    "is_decal": true,
                    "is_glass": false,
                    "activation_state": {"state": "active", "reason": "visible"},
                    "bundled_fallbacks": serde_json::Value::Null,
                    "palette_channel": serde_json::Value::Null,
                    "layer_manifest_len": 0,
                },
            }),
        );
        assert_eq!(
            environment_snapshot,
            serde_json::json!({
                "alphaMode": "OPAQUE",
                "hasBaseTransform": true,
                "hasEmissiveTexture": true,
                "hasOcclusionTexture": false,
                "semantic": {
                    "shader_family": "DisplayScreen",
                    "is_decal": false,
                    "is_glass": false,
                    "activation_state": {"state": "active", "reason": "visible"},
                    "bundled_fallbacks": ["rtt_placeholder", "screen_emissive_placeholder"],
                    "palette_channel": serde_json::Value::Null,
                    "layer_manifest_len": 0,
                },
            }),
        );
    }

    #[test]
    fn write_glb_validation_smoke_test_accepts_representative_outputs() {
        use gltf_json::validation::Validate;

        let outputs = [
            (
                "ship",
                write_glb(
                    GlbInput {
                        root_mesh: Some(triangle_mesh()),
                        root_materials: Some(phase_two_material_file()),
                        root_textures: Some(phase_two_textures()),
                        root_nmc: None,
                        root_palette: None,
                        skeleton_bones: Vec::new(),
                        children: Vec::new(),
                        interiors: crate::pipeline::LoadedInteriors::default(),
                    },
                    &mut GlbLoaders {
                        load_textures: &mut |_, _| None,
                        load_interior_mesh: &mut |_| None,
                    },
                    &{
                        let mut opts = textured_opts();
                        opts.material_mode = crate::pipeline::MaterialMode::All;
                        opts.metadata.export_options.material_mode = "All".to_string();
                        opts
                    },
                )
                .expect("ship write_glb failed"),
            ),
            (
                "rover",
                write_glb(
                    GlbInput {
                        root_mesh: Some(triangle_mesh()),
                        root_materials: Some(layered_material_file()),
                        root_textures: None,
                        root_nmc: None,
                        root_palette: Some(named_palette()),
                        skeleton_bones: Vec::new(),
                        children: Vec::new(),
                        interiors: crate::pipeline::LoadedInteriors::default(),
                    },
                    &mut GlbLoaders {
                        load_textures: &mut |_, _| None,
                        load_interior_mesh: &mut |_| None,
                    },
                    &{
                        let mut opts = default_opts();
                        opts.material_mode = crate::pipeline::MaterialMode::Colors;
                        opts.metadata.export_options.material_mode = "Colors".to_string();
                        opts
                    },
                )
                .expect("rover write_glb failed"),
            ),
            (
                "wearable",
                write_glb(
                    GlbInput {
                        root_mesh: Some(triangle_mesh()),
                        root_materials: Some(decal_material_file()),
                        root_textures: Some(shared_textures()),
                        root_nmc: None,
                        root_palette: None,
                        skeleton_bones: Vec::new(),
                        children: Vec::new(),
                        interiors: crate::pipeline::LoadedInteriors::default(),
                    },
                    &mut GlbLoaders {
                        load_textures: &mut |_, _| None,
                        load_interior_mesh: &mut |_| None,
                    },
                    &textured_opts(),
                )
                .expect("wearable write_glb failed"),
            ),
            (
                "environment",
                write_glb(
                    GlbInput {
                        root_mesh: Some(triangle_mesh()),
                        root_materials: Some(screen_material_file()),
                        root_textures: Some(screen_textures()),
                        root_nmc: None,
                        root_palette: None,
                        skeleton_bones: Vec::new(),
                        children: Vec::new(),
                        interiors: crate::pipeline::LoadedInteriors::default(),
                    },
                    &mut GlbLoaders {
                        load_textures: &mut |_, _| None,
                        load_interior_mesh: &mut |_| None,
                    },
                    &textured_opts(),
                )
                .expect("environment write_glb failed"),
            ),
        ];

        for (label, glb) in outputs {
            let root = glb_root(&glb);
            let mut issues = Vec::new();
            root.validate(&root, json::Path::new, &mut |path, error| {
                issues.push(format!("{}:{error:?}", path().as_str()));
            });
            assert!(issues.is_empty(), "{label} GLB should validate cleanly: {}", issues.join(", "));
        }
    }

    #[test]
    fn mat3x4_to_gltf_identity() {
        let m: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let g = mat3x4_to_gltf(&m);
        #[rustfmt::skip]
        let expected = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(g, expected);
    }

    #[test]
    fn mat3x4_to_gltf_translation() {
        let m: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 5.0],
            [0.0, 1.0, 0.0, 10.0],
            [0.0, 0.0, 1.0, -3.0],
        ];
        let g = mat3x4_to_gltf(&m);
        // Column-major: last 4 elements should be the translation + 1.0
        assert_eq!(g[12], 5.0);
        assert_eq!(g[13], 10.0);
        assert_eq!(g[14], -3.0);
        assert_eq!(g[15], 1.0);
    }

    #[test]
    fn mul_3x4_identity_times_identity() {
        let id: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let result = mul_3x4(&id, &id);
        for r in 0..3 {
            for c in 0..4 {
                assert!(
                    (result[r][c] - id[r][c]).abs() < 1e-6,
                    "mismatch at [{r}][{c}]"
                );
            }
        }
    }

    #[test]
    fn write_glb_does_not_reload_child_textures_when_payload_is_preloaded() {
        let mut child = child_entity("child_thruster", shared_material_file());
        child.textures = Some(shared_textures());
        let mut child_texture_loads = 0;

        write_glb(
            GlbInput {
                root_mesh: Some(triangle_mesh()),
                root_materials: None,
                root_textures: None,
                root_nmc: None,
                root_palette: None,
                skeleton_bones: Vec::new(),
                children: vec![child],
                interiors: crate::pipeline::LoadedInteriors::default(),
            },
            &mut GlbLoaders {
                load_textures: &mut |_, _| {
                    child_texture_loads += 1;
                    Some(shared_textures())
                },
                load_interior_mesh: &mut |_| None,
            },
            &{
                let mut opts = textured_opts();
                opts.material_mode = crate::pipeline::MaterialMode::All;
                opts.metadata.export_options.material_mode = "All".to_string();
                opts
            },
        )
        .expect("write_glb failed");

        assert_eq!(child_texture_loads, 0, "child payloads with preloaded textures should not trigger texture reloads during GLB packing");
    }

    #[test]
    fn mul_3x4_translation_composition() {
        // A translates by (1,0,0), B translates by (0,2,0)
        let a: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let b: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 2.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        let result = mul_3x4(&a, &b);
        // A * B should translate by (1,2,0)
        assert!((result[0][3] - 1.0).abs() < 1e-6);
        assert!((result[1][3] - 2.0).abs() < 1e-6);
        assert!((result[2][3] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn is_identity_or_zero_detects_zero() {
        let zero = [[0.0f32; 4]; 3];
        assert!(is_identity_or_zero(&zero));
    }

    #[test]
    fn is_identity_or_zero_detects_identity() {
        let id: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        assert!(is_identity_or_zero(&id));
    }

    #[test]
    fn is_identity_or_zero_rejects_translation() {
        let m: [[f32; 4]; 3] = [
            [1.0, 0.0, 0.0, 5.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        assert!(!is_identity_or_zero(&m));
    }

    /// Build a mesh with two submeshes assigned to different NMC nodes,
    /// and verify the scene graph path produces hierarchical nodes.
    #[test]
    fn write_glb_scene_graph_produces_hierarchical_nodes() {
        // 6 vertices, 2 triangles = 2 submeshes on different nodes
        let mesh = Mesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
                [2.0, 1.0, 0.0],
            ],
            indices: vec![0, 1, 2, 3, 4, 5],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            model_min: [0.0; 3],
            model_max: [3.0, 1.0, 0.0],
            scaling_min: [0.0; 3],
            scaling_max: [3.0, 1.0, 0.0],
            submeshes: vec![
                SubMesh {
                    material_name: Some("mat_a".into()),
                    material_id: 0,
                    first_index: 0,
                    num_indices: 3,
                    first_vertex: 0,
                    num_vertices: 3,
                    node_parent_index: 0, // belongs to root node
                },
                SubMesh {
                    material_name: Some("mat_b".into()),
                    material_id: 0,
                    first_index: 3,
                    num_indices: 3,
                    first_vertex: 3,
                    num_vertices: 3,
                    node_parent_index: 1, // belongs to child node
                },
            ],
        };

        let nmc = NodeMeshCombo {
            nodes: vec![
                NmcNode {
                    name: "Root".into(),
                    parent_index: None,
                    world_to_bone: [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    bone_to_world: [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    scale: [1.0, 1.0, 1.0],
                    geometry_type: 0,
                    properties: std::collections::HashMap::new(),
                },
                NmcNode {
                    name: "Child".into(),
                    parent_index: Some(0),
                    world_to_bone: [
                        [1.0, 0.0, 0.0, -5.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    bone_to_world: [
                        [1.0, 0.0, 0.0, 5.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    scale: [1.0, 1.0, 1.0],
                    geometry_type: 0,
                    properties: std::collections::HashMap::new(),
                },
            ],
            material_indices: vec![0, 0],
        };

        let glb =
            write_glb_simple(&mesh, Some(&nmc)).expect("write_glb with NMC failed");

        // Verify GLB header
        assert_eq!(&glb[0..4], b"glTF");

        // Parse JSON to verify structure
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json_bytes = &glb[20..20 + json_len];
        let json_str = std::str::from_utf8(json_bytes).expect("JSON not valid UTF-8");
        let root: serde_json::Value = serde_json::from_str(json_str.trim()).expect("invalid JSON");

        // Should have 2 meshes (one per node with geometry)
        let meshes = root["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), 2, "expected 2 meshes for 2 geometry nodes");

        // Should have 3 nodes (2 content + 1 coordinate root)
        let nodes = root["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3, "expected 3 nodes (2 content + coord root)");

        // First node (Root) should have name "Root" and children [1]
        assert_eq!(nodes[0]["name"], "Root");
        let children = nodes[0]["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0], 1);

        // Second node (Child) should have name "Child" and a matrix with translation
        assert_eq!(nodes[1]["name"], "Child");
        // Root node: bone_to_world is identity → is_identity_or_zero → matrix=None
        assert!(
            nodes[0]["matrix"].is_null(),
            "root identity should be omitted"
        );
        // Child: bone_to_world (local transform) has tx=5
        let matrix = nodes[1]["matrix"].as_array().unwrap();
        assert_eq!(matrix.len(), 16);
        // Column-major: element [12] is translation X
        assert_eq!(matrix[12].as_f64().unwrap(), 5.0);

        // Scene should reference the coordinate root node which wraps the content
        let scene_nodes = root["scenes"][0]["nodes"].as_array().unwrap();
        assert_eq!(scene_nodes.len(), 1);
        // Coordinate root is at index 2 (after the 2 content nodes)
        assert_eq!(
            nodes[scene_nodes[0].as_u64().unwrap() as usize]["name"],
            "CryEngine_Z_up"
        );
    }

    /// Verify that a non-trivial child transform produces a matrix in the output.
    #[test]
    fn write_glb_scene_graph_nontrivial_transform() {
        let mesh = Mesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            model_min: [0.0; 3],
            model_max: [1.0; 3],
            scaling_min: [0.0; 3],
            scaling_max: [1.0; 3],
            submeshes: vec![SubMesh {
                material_name: None,
                material_id: 0,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 1,
            }],
        };

        let nmc = NodeMeshCombo {
            nodes: vec![
                NmcNode {
                    name: "Root".into(),
                    parent_index: None,
                    world_to_bone: [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    bone_to_world: [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    scale: [1.0, 1.0, 1.0],
                    geometry_type: 0,
                    properties: std::collections::HashMap::new(),
                },
                NmcNode {
                    name: "Offset".into(),
                    parent_index: Some(0),
                    world_to_bone: [
                        [1.0, 0.0, 0.0, -10.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    bone_to_world: [
                        [1.0, 0.0, 0.0, 10.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                    ],
                    scale: [1.0, 1.0, 1.0],
                    geometry_type: 0,
                    properties: std::collections::HashMap::new(),
                },
            ],
            material_indices: vec![0],
        };

        let glb = write_glb_simple(&mesh, Some(&nmc)).expect("write_glb failed");
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json_bytes = &glb[20..20 + json_len];
        let root: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(json_bytes).unwrap().trim()).unwrap();

        let nodes = root["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3, "2 content nodes + 1 coord root");

        // Root has no geometry (submesh belongs to node 1)
        assert!(nodes[0]["mesh"].is_null(), "root should have no mesh");

        // Offset node has the mesh
        assert!(!nodes[1]["mesh"].is_null(), "child should have mesh");

        // Only 1 mesh (only one node has geometry)
        let meshes = root["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), 1);

        // Child local transform = child.b2w = tx(10) → non-identity → should have matrix
        let matrix = nodes[1]["matrix"].as_array().unwrap();
        assert_eq!(matrix.len(), 16);
        // Column-major: element [12] is translation X
        assert_eq!(matrix[12].as_f64().unwrap(), 10.0);
    }
}
