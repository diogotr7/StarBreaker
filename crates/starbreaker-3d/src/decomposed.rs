use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};

use starbreaker_common::progress::{report as report_progress, Progress};
use starbreaker_p4k::MappedP4k;

use crate::error::Error;
use crate::gltf::{GlbInput, GlbLoaders, GlbMetadata, GlbOptions};
use crate::mtl::{MtlFile, SemanticTextureBinding, SubMaterial, TextureSemanticRole, TintPalette};
use crate::nmc::NodeMeshCombo;
use crate::pipeline::{
    DecomposedExport, ExportOptions, ExportedFile, ExportedFileKind, InteriorCgfEntry,
    LoadedInteriors, MaterialMode,
    PngCache,
};
use crate::skeleton::Bone;
use crate::types::{EntityPayload, Mesh};

pub(crate) struct DecomposedInput {
    pub entity_name: String,
    pub geometry_path: String,
    pub material_path: String,
    pub root_mesh: Mesh,
    pub root_materials: Option<MtlFile>,
    pub root_nmc: Option<NodeMeshCombo>,
    pub root_palette: Option<TintPalette>,
    pub root_bones: Vec<Bone>,
    pub children: Vec<EntityPayload>,
    pub interiors: LoadedInteriors,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TextureFlavor {
    Generic,
    Normal,
    Roughness,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TextureExportRef {
    role: String,
    source_path: String,
    export_path: String,
    export_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LayerTextureExport {
    source_material_path: String,
    diffuse_export_path: Option<String>,
    normal_export_path: Option<String>,
    roughness_export_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ExtractedMaterialEntry {
    slot_exports: Vec<serde_json::Value>,
    direct_texture_exports: Vec<TextureExportRef>,
    layer_exports: Vec<LayerTextureExport>,
    derived_texture_exports: Vec<TextureExportRef>,
}

#[derive(Debug, Clone)]
struct DecomposedMaterialView {
    mesh: Mesh,
    sidecar_materials: Option<MtlFile>,
    glb_materials: Option<MtlFile>,
}

#[derive(Debug, Clone)]
struct SceneInstanceRecord {
    entity_name: String,
    geometry_path: String,
    material_path: String,
    mesh_asset: String,
    material_sidecar: Option<String>,
    palette_id: Option<String>,
    parent_node_name: Option<String>,
    parent_entity_name: Option<String>,
    no_rotation: bool,
    offset_position: [f32; 3],
    offset_rotation: [f32; 3],
}

#[derive(Debug, Clone)]
struct InteriorPlacementRecord {
    cgf_path: String,
    material_path: Option<String>,
    mesh_asset: String,
    material_sidecar: Option<String>,
    entity_class_guid: Option<String>,
    transform: [[f32; 4]; 4],
}

#[derive(Debug, Clone)]
struct InteriorContainerRecord {
    name: String,
    palette_id: Option<String>,
    container_transform: [[f32; 4]; 4],
    placements: Vec<InteriorPlacementRecord>,
    lights: Vec<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct PaletteRecord {
    id: String,
    palette: TintPalette,
}

#[derive(Debug, Clone)]
struct LiveryUsage {
    palette_id: String,
    palette_source_name: Option<String>,
    entity_names: BTreeSet<String>,
    material_sidecars: BTreeSet<String>,
}

fn export_entity_basename(name: &str) -> &str {
    let trimmed = name.trim_matches('"');
    trimmed.rsplit('.').next().unwrap_or(trimmed)
}

fn clean_export_label(name: &str) -> String {
    let mut cleaned = String::new();
    let mut last_was_space = false;

    for ch in name.chars() {
        if ch.is_alphanumeric() {
            cleaned.push(ch);
            last_was_space = false;
        } else if ch.is_whitespace() || matches!(ch, '_' | '-') {
            if !cleaned.is_empty() && !last_was_space {
                cleaned.push(' ');
                last_was_space = true;
            }
        }
    }

    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        export_entity_basename(name).replace('_', " ")
    } else {
        cleaned.to_string()
    }
}

fn package_directory_name(entity_name: &str) -> String {
    clean_export_label(export_entity_basename(entity_name))
}

fn package_relative_path(package_name: &str, file_name: &str) -> String {
    format!("Packages/{package_name}/{file_name}")
}

fn build_decomposed_material_view(
    mesh: &Mesh,
    materials: Option<&MtlFile>,
    include_nodraw: bool,
) -> DecomposedMaterialView {
    let Some(materials) = materials else {
        return DecomposedMaterialView {
            mesh: mesh.clone(),
            sidecar_materials: None,
            glb_materials: None,
        };
    };

    if include_nodraw {
        return DecomposedMaterialView {
            mesh: mesh.clone(),
            sidecar_materials: Some(materials.clone()),
            glb_materials: None,
        };
    }

    if mesh
        .submeshes
        .iter()
        .any(|submesh| submesh.material_id as usize >= materials.materials.len())
    {
        log::warn!(
            "decomposed mesh references out-of-range material ids; keeping original NoDraw layout for {}",
            materials
                .source_path
                .as_deref()
                .unwrap_or("<unknown material source>")
        );
        return DecomposedMaterialView {
            mesh: mesh.clone(),
            sidecar_materials: Some(materials.clone()),
            glb_materials: None,
        };
    }

    let mut material_id_map = Vec::with_capacity(materials.materials.len());
    let mut filtered_materials = Vec::with_capacity(materials.materials.len());
    for material in &materials.materials {
        if material.is_nodraw {
            material_id_map.push(None);
        } else {
            material_id_map.push(Some(filtered_materials.len() as u32));
            filtered_materials.push(material.clone());
        }
    }

    if filtered_materials.len() == materials.materials.len() {
        return DecomposedMaterialView {
            mesh: mesh.clone(),
            sidecar_materials: Some(materials.clone()),
            glb_materials: None,
        };
    }

    let mut filtered_mesh = mesh.clone();
    filtered_mesh.submeshes = mesh
        .submeshes
        .iter()
        .filter_map(|submesh| {
            let Some(new_material_id) = material_id_map
                .get(submesh.material_id as usize)
                .copied()
                .flatten()
            else {
                return None;
            };

            let mut filtered = submesh.clone();
            filtered.material_id = new_material_id;
            Some(filtered)
        })
        .collect();

    let filtered_materials = MtlFile {
        materials: filtered_materials,
        source_path: materials.source_path.clone(),
    };

    DecomposedMaterialView {
        mesh: filtered_mesh,
        sidecar_materials: Some(filtered_materials.clone()),
        glb_materials: Some(filtered_materials),
    }
}

pub(crate) fn write_decomposed_export(
    p4k: &MappedP4k,
    input: DecomposedInput,
    opts: &ExportOptions,
    progress: Option<&Progress>,
    existing_asset_paths: Option<&HashSet<String>>,
    load_interior_mesh: &mut dyn FnMut(
        &InteriorCgfEntry,
    ) -> Option<(Mesh, Option<MtlFile>, Option<NodeMeshCombo>)>,
) -> Result<DecomposedExport, Error> {
    let mut files = BTreeMap::new();
    let mut texture_cache: HashMap<(String, TextureFlavor), String> = HashMap::new();
    let mut png_cache = PngCache::new();
    let mut palette_records = BTreeMap::new();
    let mut livery_usage = BTreeMap::new();
    let package_name = package_directory_name(&input.entity_name);
    let scene_manifest_path = package_relative_path(&package_name, "scene.json");
    let palettes_manifest_path = package_relative_path(&package_name, "palettes.json");
    let liveries_manifest_path = package_relative_path(&package_name, "liveries.json");
    report_progress(progress, 0.05, "Writing root assets");

    let root_material_view = build_decomposed_material_view(
        &input.root_mesh,
        input.root_materials.as_ref(),
        opts.include_nodraw,
    );

    let root_mesh_asset = write_mesh_asset(
        &mut files,
        p4k,
        &input.entity_name,
        &input.geometry_path,
        &root_material_view.mesh,
        root_material_view.glb_materials.as_ref(),
        input.root_nmc.as_ref(),
        &input.root_bones,
        existing_asset_paths,
    )?;
    let root_material_sidecar = root_material_view.sidecar_materials.as_ref().map(|materials| {
        write_material_sidecar(
            &mut files,
            p4k,
            &mut png_cache,
            &mut texture_cache,
            &palettes_manifest_path,
            &input.entity_name,
            &input.geometry_path,
            &input.material_path,
            materials,
            opts.texture_mip,
            existing_asset_paths,
        )
    });
    let root_palette_id = input
        .root_palette
        .as_ref()
        .map(|palette| register_palette(&mut palette_records, palette));
    register_livery_usage(
        &mut livery_usage,
        root_palette_id.as_deref(),
        input.root_palette.as_ref(),
        &input.entity_name,
        root_material_sidecar.as_deref(),
    );
    report_progress(progress, 0.15, "Writing child assets");

    let mut child_instances = Vec::with_capacity(input.children.len());
    let child_count = input.children.len();
    for (index, child) in input.children.iter().enumerate() {
        let child_material_view = build_decomposed_material_view(
            &child.mesh,
            child.materials.as_ref(),
            opts.include_nodraw,
        );
        let mesh_asset = write_mesh_asset(
            &mut files,
            p4k,
            &child.entity_name,
            &child.geometry_path,
            &child_material_view.mesh,
            child_material_view.glb_materials.as_ref(),
            child.nmc.as_ref(),
            &child.bones,
            existing_asset_paths,
        )?;
        let material_sidecar = child_material_view.sidecar_materials.as_ref().map(|materials| {
            write_material_sidecar(
                &mut files,
                p4k,
                &mut png_cache,
                &mut texture_cache,
                &palettes_manifest_path,
                &child.entity_name,
                &child.geometry_path,
                &child.material_path,
                materials,
                opts.texture_mip,
                existing_asset_paths,
            )
        });
        let palette_id = child
            .palette
            .as_ref()
            .map(|palette| register_palette(&mut palette_records, palette));
        register_livery_usage(
            &mut livery_usage,
            palette_id.as_deref(),
            child.palette.as_ref(),
            &child.entity_name,
            material_sidecar.as_deref(),
        );

        child_instances.push(SceneInstanceRecord {
            entity_name: child.entity_name.clone(),
            geometry_path: normalize_source_path(p4k, &child.geometry_path),
            material_path: normalize_source_path(p4k, &child.material_path),
            mesh_asset,
            material_sidecar,
            palette_id,
            parent_node_name: Some(child.parent_node_name.clone()),
            parent_entity_name: Some(child.parent_entity_name.clone()),
            no_rotation: child.no_rotation,
            offset_position: child.offset_position,
            offset_rotation: child.offset_rotation,
        });

        if child_count > 0 {
            let fraction = (index + 1) as f32 / child_count as f32;
            report_progress(progress, 0.15 + 0.40 * fraction, "Writing child assets");
        }
    }
    if child_count == 0 {
        report_progress(progress, 0.55, "Writing interior assets");
    }

    let mut interior_asset_cache: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut interior_records = Vec::with_capacity(input.interiors.containers.len());
    let container_count = input.interiors.containers.len();
    for (index, container) in input.interiors.containers.iter().enumerate() {
        let palette_id = container
            .palette
            .as_ref()
            .map(|palette| register_palette(&mut palette_records, palette));
        let mut placements = Vec::with_capacity(container.placements.len());
        for (cgf_idx, transform) in &container.placements {
            let entry = &input.interiors.unique_cgfs[*cgf_idx];
            let cache_key = format!(
                "{}|{}",
                entry.cgf_path.to_lowercase(),
                entry.material_path.as_deref().unwrap_or("").to_lowercase()
            );
            let (mesh_asset, material_sidecar) = if let Some(cached) = interior_asset_cache.get(&cache_key) {
                cached.clone()
            } else {
                let Some((mesh, materials, nmc)) = load_interior_mesh(entry) else {
                    log::warn!("failed to build decomposed interior asset for {}", entry.cgf_path);
                    continue;
                };
                let interior_material_view = build_decomposed_material_view(
                    &mesh,
                    materials.as_ref(),
                    opts.include_nodraw,
                );
                let mesh_asset = write_mesh_asset(
                    &mut files,
                    p4k,
                    &entry.name,
                    &entry.cgf_path,
                    &interior_material_view.mesh,
                    interior_material_view.glb_materials.as_ref(),
                    nmc.as_ref(),
                    &[],
                    existing_asset_paths,
                )?;
                let material_sidecar = interior_material_view.sidecar_materials.as_ref().map(|materials| {
                    write_material_sidecar(
                        &mut files,
                        p4k,
                        &mut png_cache,
                        &mut texture_cache,
                        &palettes_manifest_path,
                        &entry.name,
                        &entry.cgf_path,
                        entry.material_path.as_deref().unwrap_or(""),
                        materials,
                        opts.texture_mip,
                        existing_asset_paths,
                    )
                });
                interior_asset_cache.insert(cache_key, (mesh_asset.clone(), material_sidecar.clone()));
                (mesh_asset, material_sidecar)
            };

            register_livery_usage(
                &mut livery_usage,
                palette_id.as_deref(),
                container.palette.as_ref(),
                &entry.name,
                material_sidecar.as_deref(),
            );

            placements.push(InteriorPlacementRecord {
                cgf_path: normalize_source_path(p4k, &entry.cgf_path),
                material_path: entry
                    .material_path
                    .as_ref()
                    .map(|path| normalize_source_path(p4k, path)),
                mesh_asset,
                material_sidecar,
                entity_class_guid: None,
                transform: *transform,
            });
        }

        interior_records.push(InteriorContainerRecord {
            name: container.name.clone(),
            palette_id,
            container_transform: container.container_transform,
            placements,
            lights: container
                .lights
                .iter()
                .map(|light| {
                    serde_json::json!({
                        "name": light.name,
                        "position": light.position,
                        "rotation": light.rotation,
                        "color": light.color,
                        "intensity": light.intensity,
                        "radius": light.radius,
                        "inner_angle": light.inner_angle,
                        "outer_angle": light.outer_angle,
                    })
                })
                .collect(),
        });

        if container_count > 0 {
            let fraction = (index + 1) as f32 / container_count as f32;
            report_progress(progress, 0.55 + 0.30 * fraction, "Writing interior assets");
        }
    }
    if container_count == 0 {
        report_progress(progress, 0.85, "Writing manifests");
    }

    let scene_manifest = build_scene_manifest_value(
        &input.entity_name,
        &package_name,
        &normalize_source_path(p4k, &input.geometry_path),
        &normalize_source_path(p4k, &input.material_path),
        &root_mesh_asset,
        root_material_sidecar.as_deref(),
        root_palette_id.as_deref(),
        &child_instances,
        &interior_records,
        opts,
    );
    report_progress(progress, 0.95, "Writing manifests");
    insert_json_file(&mut files, scene_manifest_path, scene_manifest);
    insert_json_file(
        &mut files,
        palettes_manifest_path.clone(),
        build_palette_manifest_value(&palette_records),
    );
    insert_json_file(
        &mut files,
        liveries_manifest_path,
        build_livery_manifest_value(&livery_usage),
    );

    Ok(DecomposedExport {
        files: files
            .into_iter()
            .map(|(relative_path, bytes)| ExportedFile {
                kind: classify_exported_file_kind(&relative_path),
                relative_path,
                bytes,
            })
            .collect(),
    })
}

fn classify_exported_file_kind(relative_path: &str) -> ExportedFileKind {
    if relative_path.ends_with(".materials.json") {
        ExportedFileKind::MaterialSidecar
    } else if relative_path.ends_with(".glb") {
        ExportedFileKind::MeshAsset
    } else if relative_path.ends_with(".png") {
        ExportedFileKind::TextureAsset
    } else {
        ExportedFileKind::PackageManifest
    }
}

fn build_scene_manifest_value(
    entity_name: &str,
    package_name: &str,
    geometry_path: &str,
    material_path: &str,
    root_mesh_asset: &str,
    root_material_sidecar: Option<&str>,
    root_palette_id: Option<&str>,
    child_instances: &[SceneInstanceRecord],
    interiors: &[InteriorContainerRecord],
    opts: &ExportOptions,
) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "export_kind": "Decomposed",
        "package_rule": {
            "root": "caller_selected_export_root",
            "package_dir": format!("Packages/{package_name}"),
            "paths_are_relative_to_export_root": true,
            "shared_asset_root": "Data",
            "normalized_p4k_relative_paths": true,
        },
        "root_entity": {
            "entity_name": entity_name,
            "geometry_path": geometry_path,
            "material_path": material_path,
            "mesh_asset": root_mesh_asset,
            "material_sidecar": root_material_sidecar,
            "palette_id": root_palette_id,
        },
        "export_options": {
            "kind": format!("{:?}", opts.kind),
            "format": format!("{:?}", opts.format),
            "material_mode": format!("{:?}", opts.material_mode),
            "lod_level": opts.lod_level,
            "texture_mip": opts.texture_mip,
            "include_attachments": opts.include_attachments,
            "include_interior": opts.include_interior,
            "include_lights": opts.include_lights,
        },
        "children": child_instances.iter().map(scene_instance_json).collect::<Vec<_>>(),
        "interiors": interiors.iter().map(interior_container_json).collect::<Vec<_>>(),
    })
}

fn build_palette_manifest_value(records: &BTreeMap<String, PaletteRecord>) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "palettes": records.values().map(|record| {
            serde_json::json!({
                "id": record.id,
                "source_name": record.palette.source_name,
                "primary": record.palette.primary,
                "secondary": record.palette.secondary,
                "tertiary": record.palette.tertiary,
                "glass": record.palette.glass,
            })
        }).collect::<Vec<_>>(),
    })
}

fn build_livery_manifest_value(records: &BTreeMap<String, LiveryUsage>) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "liveries": records.values().map(|usage| {
            serde_json::json!({
                "id": usage.palette_id,
                "palette_id": usage.palette_id,
                "palette_source_name": usage.palette_source_name,
                "entity_names": usage.entity_names.iter().cloned().collect::<Vec<_>>(),
                "material_sidecars": usage.material_sidecars.iter().cloned().collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn scene_instance_json(instance: &SceneInstanceRecord) -> serde_json::Value {
    serde_json::json!({
        "entity_name": instance.entity_name,
        "geometry_path": instance.geometry_path,
        "material_path": instance.material_path,
        "mesh_asset": instance.mesh_asset,
        "material_sidecar": instance.material_sidecar,
        "palette_id": instance.palette_id,
        "parent_node_name": instance.parent_node_name,
        "parent_entity_name": instance.parent_entity_name,
        "no_rotation": instance.no_rotation,
        "offset_position": instance.offset_position,
        "offset_rotation": instance.offset_rotation,
    })
}

fn interior_container_json(container: &InteriorContainerRecord) -> serde_json::Value {
    serde_json::json!({
        "name": container.name,
        "palette_id": container.palette_id,
        "container_transform": container.container_transform,
        "placements": container.placements.iter().map(|placement| {
            serde_json::json!({
                "cgf_path": placement.cgf_path,
                "material_path": placement.material_path,
                "mesh_asset": placement.mesh_asset,
                "material_sidecar": placement.material_sidecar,
                "entity_class_guid": placement.entity_class_guid,
                "transform": placement.transform,
            })
        }).collect::<Vec<_>>(),
        "lights": container.lights,
    })
}

fn write_mesh_asset(
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    fallback_name: &str,
    geometry_path: &str,
    mesh: &Mesh,
    materials: Option<&MtlFile>,
    nmc: Option<&NodeMeshCombo>,
    bones: &[Bone],
    existing_asset_paths: Option<&HashSet<String>>,
) -> Result<String, Error> {
    fn no_textures(
        _: Option<&crate::mtl::MtlFile>,
        _: Option<&crate::mtl::TintPalette>,
    ) -> Option<crate::types::MaterialTextures> {
        None
    }

    fn no_interiors(
        _: &crate::pipeline::InteriorCgfEntry,
    ) -> Option<(Mesh, Option<MtlFile>, Option<NodeMeshCombo>)> {
        None
    }

    let mut no_textures_fn = no_textures;
    let mut no_interiors_fn = no_interiors;
    let requested_path = mesh_asset_relative_path(p4k, geometry_path, fallback_name);
    if existing_asset_paths
        .is_some_and(|paths| paths.contains(&requested_path.to_ascii_lowercase()))
    {
        return Ok(requested_path);
    }
    let glb = crate::gltf::write_glb(
        GlbInput {
            root_mesh: Some(mesh.clone()),
            root_materials: materials.cloned(),
            root_textures: None,
            root_nmc: nmc.cloned(),
            root_palette: None,
            skeleton_bones: bones.to_vec(),
            children: Vec::new(),
            interiors: LoadedInteriors::default(),
        },
        &mut GlbLoaders {
            load_textures: &mut no_textures_fn,
            load_interior_mesh: &mut no_interiors_fn,
        },
        &GlbOptions {
            material_mode: MaterialMode::None,
            metadata: GlbMetadata {
                entity_name: Some(fallback_name.to_string()),
                geometry_path: (!geometry_path.is_empty()).then_some(geometry_path.to_string()),
                material_path: None,
                export_options: crate::gltf::ExportOptionsMetadata {
                    kind: "Decomposed".to_string(),
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
    )?;
    Ok(insert_binary_file(files, requested_path, glb))
}

fn write_material_sidecar(
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    palettes_manifest_path: &str,
    fallback_name: &str,
    geometry_path: &str,
    material_path: &str,
    materials: &MtlFile,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
) -> String {
    let source_material_path = material_source_path(p4k, materials, material_path, geometry_path);
    let relative_path = material_sidecar_relative_path(&source_material_path, fallback_name);
    let extracted = materials
        .materials
        .iter()
        .map(|material| {
            extract_material_entry(
                files,
                p4k,
                png_cache,
                texture_cache,
                material,
                texture_mip,
                existing_asset_paths,
            )
        })
        .collect::<Vec<_>>();
    let value = build_material_sidecar_value(
        materials,
        &source_material_path,
        &relative_path,
        palettes_manifest_path,
        &extracted,
    );
    insert_json_file(files, relative_path, value)
}

fn extract_material_entry(
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    material: &SubMaterial,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
) -> ExtractedMaterialEntry {
    let semantic_slots = material.semantic_texture_slots();
    let slot_exports = semantic_slots
        .iter()
        .map(|binding| {
            build_slot_export_value(
                files,
                p4k,
                png_cache,
                texture_cache,
                binding,
                texture_mip,
                existing_asset_paths,
            )
        })
        .collect::<Vec<_>>();

    let mut direct_texture_exports = Vec::new();
    if let Some(path) = material.diffuse_tex.as_deref() {
        if let Some(export_path) = export_texture_asset(
            files,
            p4k,
            png_cache,
            texture_cache,
            path,
            TextureFlavor::Generic,
            texture_mip,
            existing_asset_paths,
        ) {
            direct_texture_exports.push(TextureExportRef {
                role: "diffuse".to_string(),
                source_path: normalize_source_path(p4k, path),
                export_path,
                export_kind: "source".to_string(),
            });
        }
    }
    if let Some(path) = material.normal_tex.as_deref() {
        if let Some(export_path) = export_texture_asset(
            files,
            p4k,
            png_cache,
            texture_cache,
            path,
            TextureFlavor::Normal,
            texture_mip,
            existing_asset_paths,
        ) {
            direct_texture_exports.push(TextureExportRef {
                role: "normal_gloss".to_string(),
                source_path: normalize_source_path(p4k, path),
                export_path,
                export_kind: "source".to_string(),
            });
        }
    }

    let mut derived_texture_exports = Vec::new();
    if let Some(path) = material.normal_tex.as_deref() {
        if path.contains("_ddna") {
            if let Some(export_path) = export_texture_asset(
                files,
                p4k,
                png_cache,
                texture_cache,
                path,
                TextureFlavor::Roughness,
                texture_mip,
                existing_asset_paths,
            ) {
                derived_texture_exports.push(TextureExportRef {
                    role: "roughness".to_string(),
                    source_path: normalize_source_path(p4k, path),
                    export_path,
                    export_kind: "roughness_from_normal_gloss".to_string(),
                });
            }
        }
    }

    let layer_exports = material
        .layers
        .iter()
        .map(|layer| {
            let layer_material_path = normalize_source_path(p4k, &layer.path);
            let layer_mtl = crate::pipeline::try_load_mtl(p4k, &crate::pipeline::datacore_path_to_p4k(&layer.path));
            let layer_sub = layer_mtl.as_ref().and_then(|mtl| mtl.materials.first());
            let diffuse_export_path = layer_sub
                .and_then(|sub| sub.diffuse_tex.as_deref())
                .and_then(|path| {
                    export_texture_asset(
                        files,
                        p4k,
                        png_cache,
                        texture_cache,
                        path,
                        TextureFlavor::Generic,
                        texture_mip,
                        existing_asset_paths,
                    )
                });
            let normal_path = layer_sub.and_then(|sub| sub.normal_tex.as_deref());
            let normal_export_path = normal_path.and_then(|path| {
                export_texture_asset(
                    files,
                    p4k,
                    png_cache,
                    texture_cache,
                    path,
                    TextureFlavor::Normal,
                    texture_mip,
                    existing_asset_paths,
                )
            });
            let roughness_export_path = normal_path
                .filter(|path| path.contains("_ddna"))
                .and_then(|path| {
                    export_texture_asset(
                        files,
                        p4k,
                        png_cache,
                        texture_cache,
                        path,
                        TextureFlavor::Roughness,
                        texture_mip,
                        existing_asset_paths,
                    )
                });

            LayerTextureExport {
                source_material_path: layer_material_path,
                diffuse_export_path,
                normal_export_path,
                roughness_export_path,
            }
        })
        .collect::<Vec<_>>();

    ExtractedMaterialEntry {
        slot_exports,
        direct_texture_exports,
        layer_exports,
        derived_texture_exports,
    }
}

fn build_material_sidecar_value(
    materials: &MtlFile,
    source_material_path: &str,
    relative_path: &str,
    palettes_manifest_path: &str,
    extracted: &[ExtractedMaterialEntry],
) -> serde_json::Value {
    let source_stem = source_material_path
        .rsplit('/')
        .next()
        .unwrap_or(source_material_path)
        .strip_suffix(".mtl")
        .unwrap_or(source_material_path);
    let blender_material_names = preferred_blender_material_names(&materials.materials, source_stem);

    serde_json::json!({
        "version": 1,
        "source_material_path": source_material_path,
        "normalized_export_relative_path": relative_path,
        "palette_contract": {
            "shared_manifest": palettes_manifest_path,
            "scene_instance_field": "palette_id",
        },
        "submaterials": materials.materials.iter().enumerate().map(|(index, material)| {
            build_submaterial_json(
                material,
                source_material_path,
                source_stem,
                &blender_material_names[index],
                index,
                &extracted[index],
            )
        }).collect::<Vec<_>>(),
    })
}

fn preferred_blender_material_names(materials: &[SubMaterial], source_stem: &str) -> Vec<String> {
    let mut name_counts: HashMap<&str, usize> = HashMap::new();
    for material in materials {
        *name_counts.entry(material.name.as_str()).or_default() += 1;
    }

    materials
        .iter()
        .enumerate()
        .map(|(index, material)| {
            if name_counts.get(material.name.as_str()).copied().unwrap_or_default() > 1 {
                format!("{source_stem}:{}_{}", material.name, index)
            } else {
                format!("{source_stem}:{}", material.name)
            }
        })
        .collect()
}

fn build_submaterial_json(
    material: &SubMaterial,
    source_material_path: &str,
    source_stem: &str,
    blender_material_name: &str,
    index: usize,
    extracted: &ExtractedMaterialEntry,
) -> serde_json::Value {
    let semantic_slots = material.semantic_texture_slots();
    let decoded_flags = material.decoded_string_gen_mask();
    let (activation_state, activation_reason) = material_activation_state(material, &semantic_slots);
    let public_params = material
        .public_params
        .iter()
        .map(|param| (param.name.clone(), string_value_to_json(&param.value)))
        .collect::<serde_json::Map<_, _>>();
    let virtual_inputs = semantic_slots
        .iter()
        .filter(|binding| binding.is_virtual)
        .map(|binding| binding.path.clone())
        .collect::<Vec<_>>();

    serde_json::json!({
        "index": index,
        "submaterial_name": material.name,
        "blender_material_name": blender_material_name,
        "shader": material.shader,
        "shader_family": material.shader_family().as_str(),
        "activation_state": {
            "state": activation_state,
            "reason": activation_reason,
        },
        "decoded_feature_flags": {
            "tokens": decoded_flags.tokens,
            "has_decal": decoded_flags.has_decal,
            "has_parallax_occlusion_mapping": decoded_flags.has_parallax_occlusion_mapping,
            "has_stencil_map": decoded_flags.has_stencil_map,
            "has_iridescence": decoded_flags.has_iridescence,
            "has_vertex_colors": decoded_flags.has_vertex_colors,
        },
        "texture_slots": extracted.slot_exports,
        "virtual_inputs": virtual_inputs,
        "public_params": public_params,
        "direct_textures": extracted.direct_texture_exports.iter().map(texture_ref_json).collect::<Vec<_>>(),
        "derived_textures": extracted.derived_texture_exports.iter().map(texture_ref_json).collect::<Vec<_>>(),
        "layer_manifest": material.layers.iter().enumerate().map(|(layer_index, layer)| {
            let extracted_layer = extracted.layer_exports.get(layer_index);
            let palette_channel = palette_channel_json(layer.palette_tint, false);
            serde_json::json!({
                "index": layer_index,
                "source_material_path": extracted_layer.map(|layer| layer.source_material_path.clone()).unwrap_or_else(|| layer.path.clone()),
                "tint_color": layer.tint_color,
                "palette_channel": palette_channel,
                "uv_tiling": layer.uv_tiling,
                "diffuse_export_path": extracted_layer.and_then(|layer| layer.diffuse_export_path.clone()),
                "normal_export_path": extracted_layer.and_then(|layer| layer.normal_export_path.clone()),
                "roughness_export_path": extracted_layer.and_then(|layer| layer.roughness_export_path.clone()),
            })
        }).collect::<Vec<_>>(),
        "palette_routing": {
            "material_channel": palette_channel_json(material.palette_tint, material.is_glass()),
            "layer_channels": material.layers.iter().enumerate().filter_map(|(layer_index, layer)| {
                let channel = palette_channel_json(layer.palette_tint, false)?;
                Some(serde_json::json!({
                    "index": layer_index,
                    "channel": channel,
                }))
            }).collect::<Vec<_>>(),
        },
        "material_set_identity": {
            "source_path": source_material_path,
            "source_stem": source_stem,
            "submaterial_index": index,
            "submaterial_name": material.name,
        },
        "variant_membership": {
            "palette_routed": material.palette_tint > 0 || material.is_glass(),
            "layer_palette_routed": material.layers.iter().any(|layer| layer.palette_tint > 0),
            "layered": !material.layers.is_empty(),
        },
    })
}

fn build_slot_export_value(
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    binding: &SemanticTextureBinding,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
) -> serde_json::Value {
    let source_path = slot_source_path(Some(p4k), binding);
    let export_flavor = slot_texture_flavor(binding.role);
    let export_path = if binding.is_virtual {
        None
    } else {
        export_texture_asset(
            files,
            p4k,
            png_cache,
            texture_cache,
            &binding.path,
            export_flavor,
            texture_mip,
            existing_asset_paths,
        )
    };

    serde_json::json!({
        "slot": binding.slot,
        "role": binding.role.as_str(),
        "is_virtual": binding.is_virtual,
        "source_path": source_path,
        "export_path": export_path,
        "export_kind": texture_export_kind(export_flavor),
    })
}

fn slot_source_path(p4k: Option<&MappedP4k>, binding: &SemanticTextureBinding) -> String {
    if binding.is_virtual {
        binding.path.clone()
    } else {
        p4k.map(|archive| normalize_source_path(archive, &binding.path))
            .unwrap_or_else(|| normalize_requested_source_path(&binding.path))
    }
}

fn export_texture_asset(
    files: &mut BTreeMap<String, Vec<u8>>,
    p4k: &MappedP4k,
    png_cache: &mut PngCache,
    texture_cache: &mut HashMap<(String, TextureFlavor), String>,
    source_path: &str,
    flavor: TextureFlavor,
    texture_mip: u32,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Option<String> {
    let normalized_source = normalize_source_path(p4k, source_path);
    let cache_key = (normalized_source.to_lowercase(), flavor);
    if let Some(existing) = texture_cache.get(&cache_key) {
        return Some(existing.clone());
    }

    let requested_path = texture_relative_path(p4k, source_path, flavor);
    if existing_asset_paths
        .is_some_and(|paths| paths.contains(&requested_path.to_ascii_lowercase()))
    {
        texture_cache.insert(cache_key, requested_path.clone());
        return Some(requested_path);
    }

    let bytes = match flavor {
        TextureFlavor::Generic => crate::pipeline::cached_load(
            p4k,
            source_path,
            texture_mip,
            png_cache,
            crate::pipeline::load_diffuse_texture,
        ),
        TextureFlavor::Normal => crate::pipeline::cached_load(
            p4k,
            source_path,
            texture_mip,
            png_cache,
            crate::pipeline::load_normal_texture,
        ),
        TextureFlavor::Roughness => {
            let cache_key = format!("{}@roughness_mip{}", source_path, texture_mip);
            if let Some(cached) = png_cache.get(&cache_key) {
                cached.clone()
            } else {
                let result = crate::pipeline::load_roughness_texture(p4k, source_path, texture_mip);
                png_cache.insert(cache_key, result.clone());
                result
            }
        }
    }?;

    let stored_path = insert_binary_file(files, requested_path, bytes);
    texture_cache.insert(cache_key, stored_path.clone());
    Some(stored_path)
}

fn register_palette(records: &mut BTreeMap<String, PaletteRecord>, palette: &TintPalette) -> String {
    let id = palette_id(palette);
    records.entry(id.clone()).or_insert_with(|| PaletteRecord {
        id: id.clone(),
        palette: palette.clone(),
    });
    id
}

fn register_livery_usage(
    usages: &mut BTreeMap<String, LiveryUsage>,
    palette_id: Option<&str>,
    palette: Option<&TintPalette>,
    entity_name: &str,
    material_sidecar: Option<&str>,
) {
    let Some(palette_id) = palette_id else {
        return;
    };
    let entry = usages.entry(palette_id.to_string()).or_insert_with(|| LiveryUsage {
        palette_id: palette_id.to_string(),
        palette_source_name: palette.and_then(|palette| palette.source_name.clone()),
        entity_names: BTreeSet::new(),
        material_sidecars: BTreeSet::new(),
    });
    entry.entity_names.insert(entity_name.to_string());
    if let Some(material_sidecar) = material_sidecar {
        entry.material_sidecars.insert(material_sidecar.to_string());
    }
}

fn material_source_path(
    p4k: &MappedP4k,
    materials: &MtlFile,
    material_path: &str,
    geometry_path: &str,
) -> String {
    normalize_source_path(
        p4k,
        &material_source_request(materials, material_path, geometry_path),
    )
}

fn material_source_request(materials: &MtlFile, material_path: &str, geometry_path: &str) -> String {
    if let Some(source_path) = materials.source_path.as_ref() {
        source_path.clone()
    } else if !material_path.is_empty() {
        if material_path.rsplit('/').next().is_some_and(|name| name.contains('.')) {
            material_path.to_string()
        } else {
            format!("{material_path}.mtl")
        }
    } else if geometry_path.is_empty() {
        "Data/generated/generated.mtl".to_string()
    } else {
        replace_extension(geometry_path, ".mtl")
    }
}

fn mesh_asset_relative_path(p4k: &MappedP4k, geometry_path: &str, fallback_name: &str) -> String {
    if geometry_path.is_empty() {
        format!("Data/generated/{}.glb", sanitize_identifier(fallback_name))
    } else {
        replace_extension(&normalize_source_path(p4k, geometry_path), ".glb")
    }
}

fn material_sidecar_relative_path(source_material_path: &str, fallback_name: &str) -> String {
    if source_material_path.is_empty() {
        format!("Data/generated/{}.materials.json", sanitize_identifier(fallback_name))
    } else {
        replace_extension(source_material_path, ".materials.json")
    }
}

fn texture_relative_path(p4k: &MappedP4k, source_path: &str, flavor: TextureFlavor) -> String {
    let normalized = normalize_source_path(p4k, source_path);
    match flavor {
        TextureFlavor::Generic => replace_extension(&normalized, ".png"),
        TextureFlavor::Normal => suffix_before_extension(&normalized, ".normal", ".png"),
        TextureFlavor::Roughness => suffix_before_extension(&normalized, ".roughness", ".png"),
    }
}

fn normalize_requested_source_path(path: &str) -> String {
    crate::pipeline::datacore_path_to_p4k(path).replace('\\', "/")
}

fn normalize_source_path(p4k: &MappedP4k, path: &str) -> String {
    let p4k_path = crate::pipeline::datacore_path_to_p4k(path);
    p4k.entry_case_insensitive(&p4k_path)
        .map(|entry| entry.name.replace('\\', "/"))
        .unwrap_or_else(|| normalize_requested_source_path(path))
}

fn replace_extension(path: &str, new_extension: &str) -> String {
    let Some((stem, _)) = path.rsplit_once('.') else {
        return format!("{path}{new_extension}");
    };
    stem.to_string() + new_extension
}

fn suffix_before_extension(path: &str, suffix: &str, new_extension: &str) -> String {
    if let Some((stem, _)) = path.rsplit_once('.') {
        format!("{stem}{suffix}{new_extension}")
    } else {
        format!("{path}{suffix}{new_extension}")
    }
}

fn palette_id(palette: &TintPalette) -> String {
    if let Some(source_name) = palette.source_name.as_ref() {
        format!("palette/{}", sanitize_identifier(source_name))
    } else {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hash_vec3(&mut hasher, &palette.primary);
        hash_vec3(&mut hasher, &palette.secondary);
        hash_vec3(&mut hasher, &palette.tertiary);
        hash_vec3(&mut hasher, &palette.glass);
        format!("palette/generated-{:016x}", hasher.finish())
    }
}

fn sanitize_identifier(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn insert_json_file(
    files: &mut BTreeMap<String, Vec<u8>>,
    requested_path: String,
    value: serde_json::Value,
) -> String {
    let bytes = serde_json::to_vec_pretty(&value).unwrap_or_else(|_| b"{}".to_vec());
    insert_binary_file(files, requested_path, bytes)
}

fn insert_binary_file(
    files: &mut BTreeMap<String, Vec<u8>>,
    requested_path: String,
    bytes: Vec<u8>,
) -> String {
    let requested_path = canonicalize_output_path_case(files, &requested_path);
    if let Some(existing) = files.get(&requested_path) {
        if existing == &bytes {
            return requested_path;
        }
    }

    let mut candidate = requested_path.clone();
    while let Some(existing) = files.get(&candidate) {
        if existing == &bytes {
            return candidate;
        }
        candidate = hashed_variant_path(&requested_path, &bytes);
    }
    files.insert(candidate.clone(), bytes);
    candidate
}

fn canonicalize_output_path_case(files: &BTreeMap<String, Vec<u8>>, requested_path: &str) -> String {
    let mut prefixes = String::new();
    let mut canonical_parts = Vec::new();

    for (depth, part) in requested_path.split('/').enumerate() {
        if depth > 0 {
            prefixes.push('/');
        }
        prefixes.push_str(&part.to_ascii_lowercase());

        let canonical_part = files
            .keys()
            .find_map(|existing| existing_segment_case(existing, depth, &prefixes))
            .unwrap_or_else(|| part.to_string());
        canonical_parts.push(canonical_part);
    }

    canonical_parts.join("/")
}

fn existing_segment_case(path: &str, depth: usize, lowercase_prefix: &str) -> Option<String> {
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() <= depth {
        return None;
    }
    let existing_prefix = parts[..=depth]
        .iter()
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("/");
    if existing_prefix == lowercase_prefix {
        Some(parts[depth].to_string())
    } else {
        None
    }
}

fn hashed_variant_path(path: &str, bytes: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let hash = hasher.finish();
    if let Some((stem, ext)) = path.rsplit_once('.') {
        format!("{stem}-{hash:08x}.{ext}")
    } else {
        format!("{path}-{hash:08x}")
    }
}

fn material_activation_state(
    material: &SubMaterial,
    semantic_slots: &[SemanticTextureBinding],
) -> (&'static str, &'static str) {
    if material.is_nodraw {
        ("inactive", "nodraw")
    } else if material.should_hide() {
        ("inactive", "semantic_hidden")
    } else if material.is_decal() && !has_base_color_source(material, semantic_slots) {
        ("inactive", "missing_base_color_texture")
    } else {
        ("active", "visible")
    }
}

fn has_base_color_source(material: &SubMaterial, semantic_slots: &[SemanticTextureBinding]) -> bool {
    material.diffuse_tex.is_some()
        || !material.layers.is_empty()
        || semantic_slots.iter().any(|binding| {
            !binding.is_virtual
                && matches!(
                    binding.role,
                    TextureSemanticRole::BaseColor
                        | TextureSemanticRole::AlternateBaseColor
                        | TextureSemanticRole::DecalSheet
                        | TextureSemanticRole::Stencil
                        | TextureSemanticRole::PatternMask
                )
        })
}

fn palette_channel_json(channel: u8, is_glass: bool) -> Option<serde_json::Value> {
    match channel {
        1 => Some(serde_json::json!({ "index": 1, "name": "primary" })),
        2 => Some(serde_json::json!({ "index": 2, "name": "secondary" })),
        3 => Some(serde_json::json!({ "index": 3, "name": "tertiary" })),
        _ if is_glass => Some(serde_json::json!({ "index": 0, "name": "glass" })),
        _ => None,
    }
}

fn texture_ref_json(texture_ref: &TextureExportRef) -> serde_json::Value {
    serde_json::json!({
        "role": texture_ref.role,
        "source_path": texture_ref.source_path,
        "export_path": texture_ref.export_path,
        "export_kind": texture_ref.export_kind,
    })
}

fn slot_texture_flavor(role: TextureSemanticRole) -> TextureFlavor {
    match role {
        TextureSemanticRole::NormalGloss => TextureFlavor::Normal,
        _ => TextureFlavor::Generic,
    }
}

fn texture_export_kind(flavor: TextureFlavor) -> &'static str {
    match flavor {
        TextureFlavor::Generic => "source",
        TextureFlavor::Normal => "normal_from_ddna",
        TextureFlavor::Roughness => "roughness_from_normal_gloss",
    }
}

fn string_value_to_json(value: &str) -> serde_json::Value {
    if value.eq_ignore_ascii_case("true") {
        return serde_json::Value::Bool(true);
    }
    if value.eq_ignore_ascii_case("false") {
        return serde_json::Value::Bool(false);
    }
    if let Ok(integer) = value.parse::<i64>() {
        return serde_json::json!(integer);
    }
    if let Ok(float) = value.parse::<f64>() {
        return serde_json::json!(float);
    }
    serde_json::json!(value)
}

fn hash_vec3(hasher: &mut std::collections::hash_map::DefaultHasher, values: &[f32; 3]) {
    values[0].to_bits().hash(hasher);
    values[1].to_bits().hash(hasher);
    values[2].to_bits().hash(hasher);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mtl;

    fn sample_submaterial() -> SubMaterial {
        SubMaterial {
            name: "hull_panel".into(),
            shader: "LayerBlend_V2".into(),
            diffuse: [0.7, 0.7, 0.7],
            opacity: 1.0,
            alpha_test: 0.0,
            string_gen_mask: "%STENCIL_MAP%VERTCOLORS".into(),
            is_nodraw: false,
            specular: [0.04, 0.04, 0.04],
            shininess: 128.0,
            emissive: [0.0, 0.0, 0.0],
            glow: 0.0,
            surface_type: String::new(),
            diffuse_tex: Some("Objects/Ships/Test/hull_diff.dds".into()),
            normal_tex: Some("Objects/Ships/Test/hull_ddna.dds".into()),
            layers: vec![mtl::MatLayer {
                path: "libs/materials/metal/test_layer.mtl".into(),
                tint_color: [1.0, 0.5, 0.25],
                palette_tint: 1,
                uv_tiling: 2.0,
            }],
            palette_tint: 2,
            texture_slots: vec![
                mtl::TextureSlotBinding {
                    slot: "TexSlot1".into(),
                    path: "Objects/Ships/Test/hull_diff.dds".into(),
                    is_virtual: false,
                },
                mtl::TextureSlotBinding {
                    slot: "TexSlot2".into(),
                    path: "Objects/Ships/Test/hull_ddna.dds".into(),
                    is_virtual: false,
                },
                mtl::TextureSlotBinding {
                    slot: "TexSlot7".into(),
                    path: "$TintPaletteDecal".into(),
                    is_virtual: true,
                },
            ],
            public_params: vec![mtl::PublicParam {
                name: "WearBlendBase".into(),
                value: "0.5".into(),
            }],
        }
    }

    fn sample_mesh(submeshes: Vec<crate::types::SubMesh>) -> Mesh {
        Mesh {
            positions: Vec::new(),
            indices: Vec::new(),
            uvs: None,
            secondary_uvs: None,
            normals: None,
            tangents: None,
            colors: None,
            submeshes,
            model_min: [0.0, 0.0, 0.0],
            model_max: [0.0, 0.0, 0.0],
            scaling_min: [0.0, 0.0, 0.0],
            scaling_max: [0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn normalize_source_paths_keep_data_prefix_and_slashes() {
        assert_eq!(
            normalize_requested_source_path("Objects/Ships/Test/hull_diff.dds"),
            "Data/Objects/Ships/Test/hull_diff.dds"
        );
        assert_eq!(
            normalize_requested_source_path("Data\\Objects\\Ships\\Test\\hull_diff.dds"),
            "Data/Objects/Ships/Test/hull_diff.dds"
        );
    }

    #[test]
    fn texture_relative_paths_keep_role_specific_suffixes() {
        assert_eq!(
            replace_extension(&normalize_requested_source_path("Objects/Ships/Test/hull_diff.dds"), ".png"),
            "Data/Objects/Ships/Test/hull_diff.png"
        );
        assert_eq!(
            suffix_before_extension(
                &normalize_requested_source_path("Objects/Ships/Test/hull_ddna.dds"),
                ".normal",
                ".png",
            ),
            "Data/Objects/Ships/Test/hull_ddna.normal.png"
        );
        assert_eq!(
            suffix_before_extension(
                &normalize_requested_source_path("Objects/Ships/Test/hull_ddna.dds"),
                ".roughness",
                ".png",
            ),
            "Data/Objects/Ships/Test/hull_ddna.roughness.png"
        );
    }

    #[test]
    fn material_sidecar_json_preserves_phase_three_semantics() {
        let materials = MtlFile {
            materials: vec![sample_submaterial()],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
        };
        let extracted = vec![ExtractedMaterialEntry {
            slot_exports: vec![serde_json::json!({
                "slot": "TexSlot1",
                "role": "base_color",
                "is_virtual": false,
                "source_path": "Data/Objects/Ships/Test/hull_diff.dds",
                "export_path": "Data/Objects/Ships/Test/hull_diff.png",
                "export_kind": "source",
            })],
            direct_texture_exports: vec![TextureExportRef {
                role: "diffuse".into(),
                source_path: "Data/Objects/Ships/Test/hull_diff.dds".into(),
                export_path: "Data/Objects/Ships/Test/hull_diff.png".into(),
                export_kind: "source".into(),
            }],
            layer_exports: vec![LayerTextureExport {
                source_material_path: "Data/libs/materials/metal/test_layer.mtl".into(),
                diffuse_export_path: Some("Data/libs/materials/metal/test_layer.png".into()),
                normal_export_path: Some("Data/libs/materials/metal/test_layer.normal.png".into()),
                roughness_export_path: Some("Data/libs/materials/metal/test_layer.roughness.png".into()),
            }],
            derived_texture_exports: vec![TextureExportRef {
                role: "roughness".into(),
                source_path: "Data/Objects/Ships/Test/hull_ddna.dds".into(),
                export_path: "Data/Objects/Ships/Test/hull_ddna.roughness.png".into(),
                export_kind: "roughness_from_normal_gloss".into(),
            }],
        }];

        let value = build_material_sidecar_value(
            &materials,
            "Data/Objects/Ships/Test/hull.mtl",
            "Data/Objects/Ships/Test/hull.materials.json",
            "Packages/ARGO MOLE/palettes.json",
            &extracted,
        );

        assert_eq!(value["source_material_path"], serde_json::json!("Data/Objects/Ships/Test/hull.mtl"));
        assert!(value.get("geometry_path").is_none());
        assert_eq!(
            value["submaterials"][0]["blender_material_name"],
            serde_json::json!("hull:hull_panel")
        );
        assert_eq!(value["submaterials"][0]["shader_family"], serde_json::json!("LayerBlend_V2"));
        assert_eq!(value["submaterials"][0]["palette_routing"]["material_channel"]["name"], serde_json::json!("secondary"));
        assert_eq!(value["submaterials"][0]["layer_manifest"][0]["palette_channel"]["name"], serde_json::json!("primary"));
        assert_eq!(value["submaterials"][0]["public_params"]["WearBlendBase"], serde_json::json!(0.5));
        assert_eq!(value["submaterials"][0]["derived_textures"][0]["export_kind"], serde_json::json!("roughness_from_normal_gloss"));
        assert_eq!(value["submaterials"][0]["virtual_inputs"][0], serde_json::json!("$TintPaletteDecal"));
    }

    #[test]
    fn duplicate_submaterial_names_get_stable_blender_suffixes() {
        let first = sample_submaterial();
        let mut second = sample_submaterial();
        second.shader = "Illum".into();
        second.palette_tint = 0;
        second.layers.clear();

        let materials = MtlFile {
            materials: vec![first, second],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
        };
        let extracted = vec![ExtractedMaterialEntry::default(), ExtractedMaterialEntry::default()];

        let value = build_material_sidecar_value(
            &materials,
            "Data/Objects/Ships/Test/hull.mtl",
            "Data/Objects/Ships/Test/hull.materials.json",
            "Packages/ARGO MOLE/palettes.json",
            &extracted,
        );

        assert_eq!(value["submaterials"][0]["blender_material_name"], serde_json::json!("hull:hull_panel_0"));
        assert_eq!(value["submaterials"][1]["blender_material_name"], serde_json::json!("hull:hull_panel_1"));
    }

    #[test]
    fn virtual_slot_source_paths_preserve_virtual_identifier() {
        let binding = SemanticTextureBinding {
            slot: "TexSlot7".into(),
            role: TextureSemanticRole::TintPaletteDecal,
            path: "$TintPaletteDecal".into(),
            is_virtual: true,
        };

        assert_eq!(slot_source_path(None, &binding), "$TintPaletteDecal");
    }

    #[test]
    fn livery_manifest_groups_scene_entries_by_shared_palette() {
        let mut records = BTreeMap::new();
        records.insert(
            "palette/test".to_string(),
            LiveryUsage {
                palette_id: "palette/test".to_string(),
                palette_source_name: Some("vehicle.palette.test".to_string()),
                entity_names: ["child_a".to_string(), "child_b".to_string()].into_iter().collect(),
                material_sidecars: [
                    "Data/Objects/A.materials.json".to_string(),
                    "Data/Objects/B.materials.json".to_string(),
                ]
                .into_iter()
                .collect(),
            },
        );

        let value = build_livery_manifest_value(&records);
        assert_eq!(value["liveries"][0]["palette_source_name"], serde_json::json!("vehicle.palette.test"));
        assert_eq!(value["liveries"][0]["entity_names"].as_array().map(|items| items.len()), Some(2));
        assert_eq!(value["liveries"][0]["material_sidecars"].as_array().map(|items| items.len()), Some(2));
    }

    #[test]
    fn palette_manifest_preserves_shared_palette_ids() {
        let mut records = BTreeMap::new();
        let palette = TintPalette {
            source_name: Some("vehicle.palette.test".into()),
            primary: [0.1, 0.2, 0.3],
            secondary: [0.3, 0.2, 0.1],
            tertiary: [0.4, 0.5, 0.6],
            glass: [0.6, 0.7, 0.8],
        };
        let palette_id = register_palette(&mut records, &palette);

        let value = build_palette_manifest_value(&records);
        assert_eq!(palette_id, "palette/vehicle_palette_test");
        assert_eq!(value["palettes"][0]["id"], serde_json::json!("palette/vehicle_palette_test"));
        assert_eq!(value["palettes"][0]["source_name"], serde_json::json!("vehicle.palette.test"));
        assert_eq!(value["palettes"][0]["glass"].as_array().map(|items| items.len()), Some(3));
    }

    #[test]
    fn material_source_request_prefers_loaded_source_path() {
        let materials = MtlFile {
            materials: Vec::new(),
            source_path: Some("Data\\Objects\\Ships\\Test\\canonical.mtl".into()),
        };

        let path = material_source_request(&materials, "Data/objects/ships/test/canonical", "Data/Objects/Ships/Test/hull.skin");

        assert_eq!(path, "Data\\Objects\\Ships\\Test\\canonical.mtl");
    }

    #[test]
    fn material_source_request_adds_missing_mtl_extension() {
        let materials = MtlFile {
            materials: Vec::new(),
            source_path: None,
        };

        let path = material_source_request(&materials, "Data/objects/ships/test/canonical", "Data/Objects/Ships/Test/hull.skin");

        assert_eq!(path, "Data/objects/ships/test/canonical.mtl");
    }

    #[test]
    fn decomposed_material_view_excludes_nodraw_and_renumbers_submeshes() {
        let mut nodraw = sample_submaterial();
        nodraw.name = "proxy".into();
        nodraw.shader = "NoDraw".into();
        nodraw.is_nodraw = true;

        let mut hull = sample_submaterial();
        hull.name = "hull".into();

        let mut glass = sample_submaterial();
        glass.name = "glass".into();
        glass.shader = "GlassPBR".into();

        let materials = MtlFile {
            materials: vec![nodraw, hull, glass],
            source_path: Some("Data/Objects/Ships/Test/hull.mtl".into()),
        };
        let mesh = sample_mesh(vec![
            crate::types::SubMesh {
                material_name: Some("proxy".into()),
                material_id: 0,
                first_index: 0,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("glass".into()),
                material_id: 2,
                first_index: 3,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
            crate::types::SubMesh {
                material_name: Some("hull".into()),
                material_id: 1,
                first_index: 6,
                num_indices: 3,
                first_vertex: 0,
                num_vertices: 3,
                node_parent_index: 0,
            },
        ]);

        let view = build_decomposed_material_view(&mesh, Some(&materials), false);
        let filtered_materials = view.sidecar_materials.expect("filtered sidecar materials");
        let glb_materials = view.glb_materials.expect("filtered glb materials");

        assert_eq!(filtered_materials.materials.len(), 2);
        assert_eq!(
            filtered_materials
                .materials
                .iter()
                .map(|material| material.name.as_str())
                .collect::<Vec<_>>(),
            vec!["hull", "glass"]
        );
        assert_eq!(glb_materials.materials.len(), 2);
        assert_eq!(view.mesh.submeshes.len(), 2);
        assert_eq!(
            view.mesh
                .submeshes
                .iter()
                .map(|submesh| submesh.material_id)
                .collect::<Vec<_>>(),
            vec![1, 0]
        );
    }

    #[test]
    fn insert_binary_file_reuses_identical_content_and_hashes_collisions() {
        let mut files = BTreeMap::new();
        let first = insert_binary_file(&mut files, "scene.json".to_string(), b"a".to_vec());
        let second = insert_binary_file(&mut files, "scene.json".to_string(), b"a".to_vec());
        let third = insert_binary_file(&mut files, "scene.json".to_string(), b"b".to_vec());

        assert_eq!(first, "scene.json");
        assert_eq!(second, "scene.json");
        assert_ne!(third, "scene.json");
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn scene_manifest_uses_relative_asset_paths_for_children_and_interiors() {
        let child = SceneInstanceRecord {
            entity_name: "child_a".into(),
            geometry_path: "Data/Objects/Ships/Test/child.skin".into(),
            material_path: "Data/Objects/Ships/Test/child.mtl".into(),
            mesh_asset: "Data/Objects/Ships/Test/child.glb".into(),
            material_sidecar: Some("Data/Objects/Ships/Test/child.materials.json".into()),
            palette_id: Some("palette/test".into()),
            parent_node_name: Some("hardpoint_weapon_left".into()),
            parent_entity_name: Some("root".into()),
            no_rotation: false,
            offset_position: [1.0, 2.0, 3.0],
            offset_rotation: [0.0, 90.0, 0.0],
        };
        let interior = InteriorContainerRecord {
            name: "interior_main".into(),
            palette_id: Some("palette/interior".into()),
            container_transform: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            placements: vec![InteriorPlacementRecord {
                cgf_path: "Data/Objects/Ships/Test/interior_panel.cgf".into(),
                material_path: Some("Data/Objects/Ships/Test/interior_panel.mtl".into()),
                mesh_asset: "Data/Objects/Ships/Test/interior_panel.glb".into(),
                material_sidecar: Some("Data/Objects/Ships/Test/interior_panel.materials.json".into()),
                entity_class_guid: Some("1234".into()),
                transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [4.0, 5.0, 6.0, 1.0],
                ],
            }],
            lights: vec![serde_json::json!({ "name": "light_a" })],
        };

        let value = build_scene_manifest_value(
            "root",
            "ARGO MOLE",
            "Data/Objects/Ships/Test/root.skin",
            "Data/Objects/Ships/Test/root.mtl",
            "Data/Objects/Ships/Test/root.glb",
            Some("Data/Objects/Ships/Test/root.materials.json"),
            Some("palette/root"),
            &[child],
            &[interior],
            &ExportOptions::default(),
        );

        assert_eq!(value["root_entity"]["mesh_asset"], serde_json::json!("Data/Objects/Ships/Test/root.glb"));
        assert_eq!(value["children"][0]["mesh_asset"], serde_json::json!("Data/Objects/Ships/Test/child.glb"));
        assert_eq!(value["children"][0]["parent_node_name"], serde_json::json!("hardpoint_weapon_left"));
        assert_eq!(value["interiors"][0]["placements"][0]["mesh_asset"], serde_json::json!("Data/Objects/Ships/Test/interior_panel.glb"));
        assert_eq!(value["package_rule"]["package_dir"], serde_json::json!("Packages/ARGO MOLE"));
        assert_eq!(value["package_rule"]["normalized_p4k_relative_paths"], serde_json::json!(true));
    }

    #[test]
    fn normalized_relative_paths_join_beneath_selected_base_directory() {
        let base_dir = std::path::PathBuf::from("/tmp/export-root");
        let texture_path = replace_extension(
            &normalize_requested_source_path("Objects/Ships/Test/hull_diff.dds"),
            ".png",
        );
        let full_path = base_dir.join(texture_path);

        assert_eq!(
            full_path.to_string_lossy(),
            "/tmp/export-root/Data/Objects/Ships/Test/hull_diff.png"
        );
    }
}