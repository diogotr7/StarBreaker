//! Top-level GLB assembly: `assemble_glb_with_loadout_with_progress`.
//!
//! Orchestrates the full bundled and decomposed export pipeline: resolves the
//! loadout, exports the root entity and landing gear, flattens child attachments,
//! discovers interiors, preloads meshes and textures, then calls the GLB writer
//! (or decomposed writer) with all assembled inputs. Also contains
//! `assemble_glb_with_loadout` (thin wrapper) and `path_is_shield_related`.

use std::collections::HashMap;
use std::time::Instant;

use starbreaker_common::progress::{Progress, report as report_progress};
use starbreaker_datacore::database::Database;
use starbreaker_datacore::types::Record;
use starbreaker_p4k::MappedP4k;

use crate::error::Error;
use crate::mtl;
use crate::nmc::NodeMeshCombo;
use crate::types::MaterialTextures;

use super::*;

pub fn assemble_glb_with_loadout(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    tree: &starbreaker_datacore::loadout::LoadoutTree,
    opts: &ExportOptions,
) -> Result<ExportResult, Error> {
    assemble_glb_with_loadout_with_progress(db, p4k, record, tree, opts, None, None, None, None)
}

pub fn assemble_glb_with_loadout_with_progress(
    db: &Database,
    p4k: &MappedP4k,
    record: &Record,
    tree: &starbreaker_datacore::loadout::LoadoutTree,
    opts: &ExportOptions,
    progress: Option<&Progress>,
    existing_asset_paths: Option<&HashSet<String>>,
    existing_interior_assets: Option<&crate::decomposed::ExistingInteriorAssetMap>,
    existing_asset_loader: Option<&(dyn Fn(&str) -> Option<Vec<u8>> + Sync)>,
) -> Result<ExportResult, Error> {
    const LOADOUT_STAGE_START: f32 = 0.005;
    const ROOT_EXPORT_STAGE_START: f32 = 0.03;
    const FLATTEN_STAGE_START: f32 = 0.05;
    const ASSEMBLY_STAGE_START: f32 = 0.06;
    const ASSEMBLY_STAGE_END: f32 = 0.91;

    let total_start = Instant::now();

    ensure_supported_export_options(opts)?;

    use crate::types::EntityPayload;

    let assembly_kind = classify_entity_kind(db, record);
    let is_fps_weapon = assembly_kind == EntityAssemblyKind::FpsWeapon;

    report_progress(progress, LOADOUT_STAGE_START, "Resolving loadout");
    let phase_start = Instant::now();
    let payload_material_mode = if opts.kind == ExportKind::Decomposed && !is_fps_weapon {
        MaterialMode::Colors
    } else {
        opts.material_mode
    };
    let payload_opts = ExportOptions {
        material_mode: payload_material_mode,
        ..opts.clone()
    };

    log::info!("[mem-pipeline] resolving loadout meshes...");
    let resolved = resolve_loadout_meshes(db, p4k, record, tree, &payload_opts)?;
    log::info!(
        "[mem-pipeline] resolved: {} children",
        resolved.children.len()
    );
    log::info!(
        "[timing] resolve_loadout_meshes: {:.2}s",
        phase_start.elapsed().as_secs_f32()
    );

    report_progress(progress, ROOT_EXPORT_STAGE_START, "Exporting root mesh");
    let phase_start = Instant::now();
    let localization = load_localization_map(p4k);
    let paint_display_names = build_paint_display_name_map(db, &localization);

    // Export root entity (mesh + textures).
    let (
        root_mesh,
        root_mtl,
        root_tex,
        exported_root_nmc,
        mut root_palette,
        geometry_path,
        material_path,
        root_bones,
        root_skeleton_source_path,
    ) = export_entity_payload(db, p4k, record, &payload_opts)?;
    let root_nmc = select_root_nmc(resolved.nmc, exported_root_nmc);
    log::info!(
        "[timing] export_entity_payload: {:.2}s",
        phase_start.elapsed().as_secs_f32()
    );

    let root_animation_controller = query_animation_controller_source(db, record);
    if let Some(palette) = root_palette.as_mut() {
        populate_palette_display_name(palette, &paint_display_names);
    }
    let default_root_palette = root_palette.clone();
    log::info!(
        "[mem-pipeline] root exported: {} verts",
        root_mesh.positions.len()
    );

    // Check for equipped paint item and resolve its SubGeometry palette/material override.
    let phase_start = Instant::now();
    let (mut root_palette, root_mtl) =
        resolve_paint_override(db, p4k, record, &tree.root, root_palette, root_mtl);
    log::info!(
        "[timing] resolve_paint_override: {:.2}s",
        phase_start.elapsed().as_secs_f32()
    );

    if let Some(palette) = root_palette.as_mut() {
        populate_palette_display_name(palette, &paint_display_names);
    }

    // Load landing gear as separate child entities attached to NMC nodes.
    // Landing gear CDF geometry attaches to NMC helper bones (e.g. hardpoint_landing_gear_front).
    // Adding them as EntityPayloads lets the existing scene graph handle positioning.
    // Children skip textures to save memory, but never exceed the user's material mode.
    let child_payload_material_mode = if opts.kind == ExportKind::Decomposed && !is_fps_weapon {
        MaterialMode::Colors
    } else {
        opts.material_mode
    };
    let child_material_mode = if is_fps_weapon {
        opts.material_mode
    } else {
        match opts.material_mode {
            _ if opts.kind == ExportKind::Decomposed => MaterialMode::Colors,
            MaterialMode::None => MaterialMode::None,
            _ => MaterialMode::Colors,
        }
    };
    let child_opts = ExportOptions {
        material_mode: child_material_mode,
        ..payload_opts.clone()
    };
    let gear_parts = query_landing_gear(db, record);
    let mut child_payloads: Vec<EntityPayload> = Vec::new();
    report_progress(progress, FLATTEN_STAGE_START, "Flattening attachments");
    let phase_start = Instant::now();
    if opts.include_attachments {
        let mut gear_cache: HashMap<String, LandingGearAsset> = HashMap::new();
        for (gear_path, bone_name) in &gear_parts {
            let asset = if let Some(cached) = gear_cache.get(gear_path.as_str()) {
                log::info!("  mesh cache hit for landing gear '{gear_path}'");
                Some(cached.clone())
            } else {
                match export_entity_from_paths(p4k, gear_path, "", &child_opts) {
                    Ok((
                        gear_mesh,
                        gear_mtl,
                        _,
                        gear_nmc,
                        _,
                        gear_geometry_path,
                        gear_material_path,
                        gear_bones,
                        gear_skeleton_source_path,
                    )) => {
                        let textures = if child_payload_material_mode.include_textures() {
                            gear_mtl.as_ref().map(|materials| {
                                let mut png_cache = PngCache::new();
                                load_material_textures(
                                    p4k,
                                    materials,
                                    root_palette.as_ref(),
                                    opts.texture_mip,
                                    &mut png_cache,
                                    child_payload_material_mode.include_normals(),
                                    child_payload_material_mode.experimental(),
                                )
                            })
                        } else {
                            None
                        };
                        let new_asset = LandingGearAsset {
                            mesh: gear_mesh,
                            materials: gear_mtl,
                            textures,
                            nmc: gear_nmc,
                            geometry_path: gear_geometry_path,
                            material_path: gear_material_path,
                            bones: gear_bones,
                            skeleton_source_path: gear_skeleton_source_path,
                        };
                        gear_cache.insert(gear_path.clone(), new_asset.clone());
                        Some(new_asset)
                    }
                    Err(e) => {
                        log::warn!("  landing gear '{gear_path}' failed: {e}");
                        None
                    }
                }
            };
            if let Some(asset) = asset {
                let verts = asset.mesh.positions.len();
                child_payloads.push(EntityPayload {
                    mesh: asset.mesh,
                    materials: asset.materials,
                    textures: asset.textures,
                    nmc: asset.nmc,
                    palette: root_palette.clone(),
                    geometry_path: asset.geometry_path,
                    material_path: asset.material_path,
                    bones: asset.bones,
                    skeleton_source_path: asset.skeleton_source_path,
                    entity_name: gear_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(gear_path)
                        .to_string(),
                    entity_category: None,
                    attach_def_type: None,
                    parent_node_name: bone_name.clone(),
                    parent_entity_name: resolved.entity_name.clone(),
                    no_rotation: false,
                    offset_position: [0.0; 3],
                    offset_rotation: [0.0; 3],
                    detach_direction: [0.0; 3],
                    port_flags: String::new(),
                    ui_bindings: Vec::new(),
                });
                log::info!("  landing gear '{gear_path}' → '{bone_name}', {verts} verts");
            }
        }
        log::info!(
            "[gear] {} hardpoints, {} unique CGFs cached",
            gear_parts.len(),
            gear_cache.len(),
        );
        flatten_resolved_tree(
            &resolved.children,
            &resolved.entity_name,
            None,
            db,
            p4k,
            &child_opts,
            child_payload_material_mode,
            existing_asset_paths,
            &mut child_payloads,
        );
    }
    log::info!(
        "[timing] load_landing_gear + flatten_tree: {:.2}s",
        phase_start.elapsed().as_secs_f32()
    );
    let total_child_verts: usize = child_payloads.iter().map(|c| c.mesh.positions.len()).sum();
    log::info!(
        "[mem-pipeline] flattened: {} payloads, {} total verts",
        child_payloads.len(),
        total_child_verts
    );
    let weapon_assembly = if is_fps_weapon {
        let root_assembly_geometry_path =
            resolved.geometry_path.as_deref().unwrap_or(&geometry_path);
        Some(resolve_weapon_assembly(
            p4k,
            &resolved.entity_name,
            root_assembly_geometry_path,
            &geometry_path,
            &material_path,
            &root_mesh,
            root_nmc.as_ref(),
            &root_bones,
            root_skeleton_source_path.as_deref(),
            &mut child_payloads,
        ))
    } else {
        None
    };
    report_progress(progress, ASSEMBLY_STAGE_START, "Discovering interiors");

    // Interior discovery (no mesh loading — JIT during GLB packing).
    let phase_start = Instant::now();
    let mut loaded_interiors = if opts.include_interior && !opts.format.is_stl() {
        load_interiors(db, p4k, record, opts)
    } else {
        LoadedInteriors::default()
    };
    if opts.include_interior && !opts.format.is_stl() {
        let root_container_names: std::collections::HashSet<String> = loaded_interiors
            .containers
            .iter()
            .map(|container| container.name.to_ascii_lowercase())
            .collect();
        let child_interiors = load_child_interiors(
            db,
            p4k,
            &resolved.entity_name,
            &resolved.children,
            &root_container_names,
            opts,
        );
        merge_interiors(&mut loaded_interiors, child_interiors);
    }
    log::info!(
        "[timing] load_interiors: {:.2}s",
        phase_start.elapsed().as_secs_f32()
    );

    log::info!(
        "Assembling: root + {} child meshes + {} interior meshes ({} unique CGFs)",
        child_payloads.len(),
        loaded_interiors
            .containers
            .iter()
            .map(|c| c.placements.len())
            .sum::<usize>(),
        loaded_interiors.unique_cgfs.len()
    );

    let mut preloaded_interior_meshes = if opts.kind == ExportKind::Bundled {
        preload_interior_meshes(&loaded_interiors, p4k, &child_opts)
    } else {
        Vec::new()
    };
    if !preloaded_interior_meshes.is_empty() {
        log::info!(
            "[mem-pipeline] preloaded interior meshes: {}/{}",
            preloaded_interior_meshes
                .iter()
                .filter(|asset| asset.is_some())
                .count(),
            preloaded_interior_meshes.len()
        );
    }
    let preloaded_interior_textures = if opts.kind == ExportKind::Bundled {
        preload_interior_textures(
            &loaded_interiors,
            &preloaded_interior_meshes,
            root_palette.as_ref(),
            p4k,
            opts,
        )
    } else {
        std::collections::HashMap::new()
    };
    if !preloaded_interior_textures.is_empty() {
        log::info!(
            "[mem-pipeline] preloaded interior texture sets: {}",
            preloaded_interior_textures.len()
        );
    }
    report_progress(
        progress,
        ASSEMBLY_STAGE_START,
        if opts.kind == ExportKind::Decomposed {
            "Building structured package"
        } else {
            "Packing GLB"
        },
    );
    let preloaded_interior_mesh_indices: std::collections::HashMap<String, usize> =
        loaded_interiors
            .unique_cgfs
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.cgf_path.clone(), index))
            .collect();

    // Texture loading callback: root/child entities fall back to the normal JIT
    // path, while bundled interior meshes can consume preloaded texture sets.
    let mut png_cache = PngCache::new();
    let mut tex_loader: Box<
        dyn FnMut(
            Option<&crate::mtl::MtlFile>,
            Option<&crate::mtl::TintPalette>,
        ) -> Option<MaterialTextures>,
    > = if !opts.material_mode.include_textures() {
        Box::new(|_, _| None)
    } else {
        let mip = opts.texture_mip;
        let include_normals = opts.material_mode.include_normals();
        let experimental_textures = opts.material_mode.experimental();
        Box::new(
            move |mtl: Option<&crate::mtl::MtlFile>, palette: Option<&crate::mtl::TintPalette>| {
                if let Some(material_source) =
                    mtl.and_then(|materials| materials.source_path.as_ref())
                {
                    let cache_key = PreloadedTextureKey {
                        material_source: material_source.to_ascii_lowercase(),
                        palette_hash: tint_palette_hash(palette),
                    };
                    if let Some(textures) = preloaded_interior_textures.get(&cache_key).cloned() {
                        return Some(textures);
                    }
                }

                mtl.map(|m| {
                    load_material_textures(
                        p4k,
                        m,
                        palette,
                        mip,
                        &mut png_cache,
                        include_normals,
                        experimental_textures,
                    )
                })
            },
        )
    };

    let mut interior_png_cache = PngCache::new();
    // Bundled GLB exports preload unique interior CGFs in parallel, then fall
    // back to on-demand loading for any cache misses and for decomposed exports.
    let mut interior_mesh_loader = |entry: &crate::pipeline::InteriorCgfEntry| -> Option<(
        crate::Mesh,
        Option<mtl::MtlFile>,
        Option<crate::nmc::NodeMeshCombo>,
    )> {
        if let Some(&index) = preloaded_interior_mesh_indices.get(&entry.cgf_path) {
            if let Some(asset) = preloaded_interior_meshes
                .get_mut(index)
                .and_then(Option::take)
            {
                return Some(asset);
            }
        }

        load_interior_mesh_asset(p4k, entry, &child_opts, &mut interior_png_cache)
    };

    if opts.kind == ExportKind::Decomposed {
        let mut available_palettes =
            query_related_tint_palettes(db, record, default_root_palette.as_ref());
        for palette in &mut available_palettes {
            populate_palette_display_name(palette, &paint_display_names);
        }
        let paint_variants =
            enumerate_paint_variants_for_entity(db, p4k, record, &paint_display_names);
        let decomposed_progress =
            progress.map(|progress| progress.sub(ASSEMBLY_STAGE_START, ASSEMBLY_STAGE_END));

        let decomposed_input = crate::decomposed::DecomposedInput {
            entity_name: resolved.entity_name.clone(),
            geometry_path: geometry_path.clone(),
            material_path: material_path.clone(),
            assembly_kind: weapon_assembly.as_ref().map(|_| "fps_weapon".to_string()),
            weapon_assembly: weapon_assembly
                .as_ref()
                .map(|assembly| assembly.manifest.clone()),
            weapon_assembly_diagnostics: weapon_assembly
                .as_ref()
                .map(|assembly| assembly.diagnostics.clone()),
            root_mesh,
            root_materials: root_mtl,
            root_nmc,
            root_palette: root_palette.clone(),
            available_palettes,
            root_bones,
            root_skeleton_source_path,
            root_animation_controller,
            children: child_payloads,
            interiors: loaded_interiors,
            paint_variants,
        };

        let phase_start = Instant::now();
        let decomposed = crate::pipeline::write_decomposed_export_blend(
            db,
            p4k,
            decomposed_input,
            opts,
            decomposed_progress.as_ref(),
            existing_asset_paths,
            existing_interior_assets,
            existing_asset_loader,
        )?;
        log::info!(
            "[timing] write_decomposed_export: {:.2}s",
            phase_start.elapsed().as_secs_f32()
        );

        report_progress(progress, ASSEMBLY_STAGE_END, "Writing structured package");

        return Ok(ExportResult {
            kind: opts.kind,
            format: opts.format,
            glb: Vec::new(),
            decomposed: Some(decomposed),
            geometry_path,
            material_path,
        });
    }

    let glb_progress =
        progress.map(|progress| progress.sub(ASSEMBLY_STAGE_START, ASSEMBLY_STAGE_END));
    let phase_start = Instant::now();
    let glb = crate::gltf::write_glb_with_progress(
        crate::gltf::GlbInput {
            root_mesh: Some(root_mesh),
            root_materials: root_mtl,
            root_textures: root_tex,
            root_nmc,
            root_palette: root_palette.clone(),
            skeleton_bones: root_bones,
            children: child_payloads,
            interiors: loaded_interiors,
        },
        &mut crate::gltf::GlbLoaders {
            load_textures: &mut tex_loader,
            load_interior_mesh: &mut interior_mesh_loader,
        },
        &crate::gltf::GlbOptions {
            material_mode: opts.material_mode,
            preserve_textureless_decal_primitives: false,
            metadata: crate::gltf::GlbMetadata {
                entity_name: Some(resolved.entity_name.clone()),
                geometry_path: Some(geometry_path.clone()),
                material_path: Some(material_path.clone()),
                export_options: crate::gltf::ExportOptionsMetadata {
                    kind: format!("{:?}", opts.kind),
                    material_mode: format!("{:?}", opts.material_mode),
                    format: format!("{:?}", opts.format),
                    lod_level: opts.lod_level,
                    texture_mip: opts.texture_mip,
                    include_attachments: opts.include_attachments,
                    include_interior: opts.include_interior,
                },
            },
            fallback_palette: root_palette,
        },
        glb_progress.as_ref(),
    )?;
    log::info!(
        "[timing] write_glb_with_progress: {:.2}s",
        phase_start.elapsed().as_secs_f32()
    );

    report_progress(progress, ASSEMBLY_STAGE_END, "Writing bundled file");

    log::info!(
        "[timing] TOTAL export time: {:.2}s",
        total_start.elapsed().as_secs_f32()
    );

    Ok(ExportResult {
        kind: opts.kind,
        format: opts.format,
        glb,
        decomposed: None,
        geometry_path,
        material_path,
    })
}

///
/// When `override_attachment` is Some, the first level of children uses that
/// attachment name instead of their own (reparenting through a no-geometry parent).
pub(crate) fn path_is_shield_related(path: Option<&str>) -> bool {
    path.is_some_and(|value| value.to_ascii_lowercase().contains("/shields/"))
        || path.is_some_and(|value| value.to_ascii_lowercase().contains("\\shields\\"))
}

fn select_root_nmc(
    loadout_nmc: Option<NodeMeshCombo>,
    exported_nmc: Option<NodeMeshCombo>,
) -> Option<NodeMeshCombo> {
    if exported_nmc
        .as_ref()
        .is_some_and(|nmc| !nmc.nodes.is_empty())
    {
        exported_nmc
    } else {
        loadout_nmc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nmc::{NmcNode, NodeMeshCombo};

    #[test]
    fn exported_root_nmc_takes_priority_over_loadout_probe_nmc() {
        let selected = select_root_nmc(Some(nmc("loadout_root")), Some(nmc("exported_root")))
            .expect("expected an NMC");

        assert_eq!(selected.nodes[0].name, "exported_root");
    }

    fn nmc(root_name: &str) -> NodeMeshCombo {
        NodeMeshCombo {
            nodes: vec![NmcNode {
                name: root_name.to_string(),
                parent_index: None,
                world_to_bone: [[0.0; 4]; 3],
                bone_to_world: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                scale: [1.0; 3],
                geometry_type: 0,
                properties: std::collections::HashMap::new(),
            }],
            material_indices: vec![],
        }
    }
}
