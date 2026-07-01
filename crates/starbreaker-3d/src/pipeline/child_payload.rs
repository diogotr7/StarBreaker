//! Child entity payload loading and caching helpers.
//!
//! Loads child mesh assets for loadout children and inline entity attachments.
//! `collect_child_payload_specs` gathers attachment specs from a loadout tree;
//! `load_child_payload_asset` exports a single child CGF; `load_child_payloads`
//! drives the full parallel load pipeline.
//! Also defines `LandingGearAsset` (cached landing gear geometry) and the path
//! normalisation helpers used for decomposed export deduplication.

use std::collections::{HashMap, HashSet};

use starbreaker_datacore::database::Database;
use starbreaker_datacore::query::value::Value;
use starbreaker_datacore::types::{CigGuid, Record};
use starbreaker_p4k::MappedP4k;

use crate::mtl;
use crate::nmc;
use crate::types::{MaterialTextures, UiBinding};

use super::*;

/// Derive a "screen" helper-node name from a prop's NMC + materials by structural
/// inspection only — never hardcoded by ship or asset.
///
/// Rule (BuildingBlocks vocabulary, not asset-specific):
/// 1. If exactly one NMC node has a name containing the token "rtt"
///    (case-insensitive), return that node name. CryEngine convention: helper
///    nodes named `rtt_*` mark "render-to-texture" surfaces.
/// 2. Otherwise, if the prop's MTL has materials whose names contain "rtt",
///    locate the mesh nodes wired to those materials via `material_indices`
///    (parallel to `nodes[]`). If exactly one node is wired to an RTT
///    material, return that node name.
/// 3. Otherwise return None.
pub(crate) fn derive_screen_helper_name(
    nmc: Option<&nmc::NodeMeshCombo>,
    materials: Option<&mtl::MtlFile>,
) -> Option<String> {
    let nmc = nmc?;
    const TOKEN: &str = "rtt";
    let name_matches: Vec<&str> = nmc
        .nodes
        .iter()
        .filter(|n| n.name.to_ascii_lowercase().contains(TOKEN))
        .map(|n| n.name.as_str())
        .collect();
    if name_matches.len() == 1 {
        return Some(name_matches[0].to_string());
    }
    let mtl = materials?;
    let rtt_material_indices: HashSet<usize> = mtl
        .materials
        .iter()
        .enumerate()
        .filter(|(_, m)| m.name.to_ascii_lowercase().contains(TOKEN))
        .map(|(i, _)| i)
        .collect();
    if rtt_material_indices.is_empty() {
        return None;
    }
    let node_matches: Vec<&str> = nmc
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            nmc.material_indices
                .get(*i)
                .is_some_and(|mi| rtt_material_indices.contains(&(*mi as usize)))
        })
        .map(|(_, n)| n.name.as_str())
        .collect();
    if node_matches.len() == 1 {
        Some(node_matches[0].to_string())
    } else {
        None
    }
}

/// Try loading an entity's mesh data from its resolved geometry/material paths,
/// falling back to DataCore record lookup.
pub(crate) fn load_child_mesh(
    child: &crate::types::ResolvedNode,
    db: &Database,
    p4k: &MappedP4k,
    opts: &ExportOptions,
) -> Option<(
    crate::types::Mesh,
    Option<mtl::MtlFile>,
    Option<nmc::NodeMeshCombo>,
    Option<mtl::TintPalette>,
    Vec<crate::skeleton::Bone>,
    String,
    String,
    Option<String>,
)> {
    let result = if child.geometry_path.is_some() {
        let gp = child.geometry_path.as_deref().unwrap_or("");
        let mp = child.material_path.as_deref().unwrap_or("");
        export_entity_from_paths(p4k, gp, mp, opts)
            .map_err(|e| {
                log::warn!("  {} -> load from paths failed: {e}", child.entity_name);
                e
            })
            .or_else(|_| export_entity_payload(db, p4k, &child.record, opts))
    } else {
        export_entity_payload(db, p4k, &child.record, opts)
    };

    result
        .ok()
        .map(
            |(
                mesh,
                mtl,
                _tex,
                nmc,
                palette,
                geometry_path,
                material_path,
                bones,
                skeleton_source_path,
            )| {
                let resolved_palette = palette.or_else(|| query_tint_palette(db, &child.record));
                (
                    mesh,
                    mtl,
                    nmc,
                    resolved_palette,
                    bones,
                    geometry_path,
                    material_path,
                    skeleton_source_path,
                )
            },
        )
}

pub(crate) struct ChildPayloadSpec {
    pub(crate) child: crate::types::ResolvedNode,
    pub(crate) parent_entity_name: String,
    pub(crate) parent_node_name: String,
    pub(crate) no_rotation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ChildPayloadCacheKey {
    pub(crate) record_id: starbreaker_datacore::types::CigGuid,
    pub(crate) geometry_path: Option<String>,
    pub(crate) material_path: Option<String>,
}

#[derive(Clone)]
pub(crate) struct LoadedChildPayload {
    pub(crate) mesh: crate::types::Mesh,
    pub(crate) materials: Option<mtl::MtlFile>,
    pub(crate) textures: Option<MaterialTextures>,
    pub(crate) nmc: Option<nmc::NodeMeshCombo>,
    pub(crate) palette: Option<mtl::TintPalette>,
    pub(crate) bones: Vec<crate::skeleton::Bone>,
    pub(crate) geometry_path: String,
    pub(crate) material_path: String,
    pub(crate) skeleton_source_path: Option<String>,
}

/// Cached result of loading a landing-gear CGF. Multiple gear hardpoints
/// often share the same geometry (e.g. mirrored gears), so this cache
/// dedupes P4k reads and texture decoding by gear_path.
#[derive(Clone)]
pub(crate) struct LandingGearAsset {
    pub(crate) mesh: crate::types::Mesh,
    pub(crate) materials: Option<mtl::MtlFile>,
    pub(crate) textures: Option<MaterialTextures>,
    pub(crate) nmc: Option<nmc::NodeMeshCombo>,
    pub(crate) geometry_path: String,
    pub(crate) material_path: String,
    pub(crate) bones: Vec<crate::skeleton::Bone>,
    pub(crate) skeleton_source_path: Option<String>,
}

pub(crate) fn collect_child_payload_specs(
    children: &[crate::types::ResolvedNode],
    parent_entity_name: &str,
    override_attachment: Option<(&str, bool)>,
    out: &mut Vec<ChildPayloadSpec>,
) {
    for child in children {
        let (attach_name, no_rotation) = match override_attachment {
            Some((name, parent_no_rot)) => (name.to_string(), child.no_rotation || parent_no_rot),
            None => (child.attachment_name.clone(), child.no_rotation),
        };

        let child_creates_nodes = child.has_geometry || child.nmc.is_some();
        out.push(ChildPayloadSpec {
            child: child.clone_payload_source(),
            parent_entity_name: parent_entity_name.to_string(),
            parent_node_name: attach_name,
            no_rotation,
        });
        if child_creates_nodes {
            collect_child_payload_specs(&child.children, &child.entity_name, None, out);
        } else {
            collect_child_payload_specs(
                &child.children,
                parent_entity_name,
                Some((&child.attachment_name, child.no_rotation)),
                out,
            );
        }
    }
}

pub(crate) fn empty_child_mesh() -> crate::types::Mesh {
    crate::types::Mesh {
        positions: Vec::new(),
        indices: Vec::new(),
        uvs: None,
        secondary_uvs: None,
        normals: None,
        tangents: None,
        colors: None,
        submeshes: Vec::new(),
        model_min: [0.0; 3],
        model_max: [0.0; 3],
        scaling_min: [0.0; 3],
        scaling_max: [0.0; 3],
    }
}

pub(crate) fn normalize_decomposed_source_path(p4k: &MappedP4k, path: &str) -> String {
    let p4k_path = datacore_path_to_p4k(path);
    p4k.entry_case_insensitive(&p4k_path)
        .map(|entry| entry.name.replace('\\', "/"))
        .unwrap_or_else(|| p4k_path.replace('\\', "/"))
}

pub(crate) fn replace_extension(path: &str, new_extension: &str) -> String {
    let Some((stem, _)) = path.rsplit_once('.') else {
        return format!("{path}{new_extension}");
    };
    stem.to_string() + new_extension
}

pub(crate) fn decomposed_mesh_asset_path(p4k: &MappedP4k, geometry_path: &str) -> Option<String> {
    if geometry_path.is_empty() {
        None
    } else {
        Some(replace_extension(&normalize_decomposed_source_path(p4k, geometry_path), ".glb"))
    }
}

pub(crate) fn build_child_payload_cache_key(child: &crate::types::ResolvedNode) -> ChildPayloadCacheKey {
    ChildPayloadCacheKey {
        record_id: child.record.id,
        geometry_path: child.geometry_path.clone(),
        material_path: child.material_path.clone(),
    }
}

pub(crate) fn load_child_payload_asset(
    child: &crate::types::ResolvedNode,
    db: &Database,
    p4k: &MappedP4k,
    mesh_opts: &ExportOptions,
    final_material_mode: MaterialMode,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Option<LoadedChildPayload> {
    if mesh_opts.kind == ExportKind::Decomposed {
        if let Some(geometry_path) = child.geometry_path.as_deref() {
            if let Some(mesh_asset_path) = decomposed_mesh_asset_path(p4k, geometry_path) {
                if existing_asset_paths
                    .is_some_and(|paths| paths.contains(&mesh_asset_path.to_ascii_lowercase()))
                {
                    let material_path = child.material_path.as_deref().unwrap_or("");
                    let (nmc, materials) = load_nmc_and_material(p4k, geometry_path, material_path);
                    // UI-render-target geometry must keep real vertex positions so
                    // each screen's physical display aspect can be measured
                    // (ui_pipeline::screen_aspect); the empty-mesh asset shortcut
                    // would drop them. Such geometry is rare (cockpit screens), so
                    // only it falls through to a full mesh load — every other
                    // cached asset keeps the fast path.
                    if !crate::ui_pipeline::screen_aspect::materials_contain_ui_render_target(
                        materials.as_ref(),
                    ) {
                        let skeleton_source_path = resolve_geometry_files(p4k, geometry_path)
                            .ok()
                            .and_then(|resolved| {
                                skeleton_source_paths(resolved.skeleton_path.as_deref(), &resolved.parts[0].path)
                                    .first()
                                    .map(|path| (*path).to_string())
                            });
                        return Some(LoadedChildPayload {
                            mesh: empty_child_mesh(),
                            materials,
                            textures: None,
                            nmc,
                            palette: None,
                            bones: Vec::new(),
                            geometry_path: geometry_path.to_string(),
                            material_path: material_path.to_string(),
                            skeleton_source_path,
                        });
                    }
                }
            }
        }
    }

    let (mesh, mtl, nmc, palette, bones, geometry_path, material_path, skeleton_source_path) =
        load_child_mesh(child, db, p4k, mesh_opts)?;
    let textures = if final_material_mode.include_textures() {
        mtl.as_ref().map(|materials| {
            let mut png_cache = PngCache::new();
            load_material_textures(
                p4k,
                materials,
                palette.as_ref(),
                mesh_opts.texture_mip,
                &mut png_cache,
                final_material_mode.include_normals(),
                final_material_mode.experimental(),
            )
        })
    } else {
        None
    };

    Some(LoadedChildPayload {
        mesh,
        materials: mtl,
        textures,
        nmc,
        palette,
        bones,
        geometry_path,
        material_path,
        skeleton_source_path,
    })
}

pub(crate) fn load_child_payloads(
    specs: Vec<ChildPayloadSpec>,
    db: &Database,
    p4k: &MappedP4k,
    mesh_opts: &ExportOptions,
    final_material_mode: MaterialMode,
    existing_asset_paths: Option<&HashSet<String>>,
) -> Vec<crate::types::EntityPayload> {
    use rayon::prelude::*;

    // Pre-resolve physical screen canvases for each unique parent entity, keyed
    // by geometry name so physical bindings are matched by name, not position.
    let physical_screen_canvases_by_parent: HashMap<String, HashMap<String, (String, Option<String>)>> = {
        let mut map = HashMap::new();
        for spec in &specs {
            let parent_name = &spec.parent_entity_name;
            if map.contains_key(parent_name) {
                continue;
            }
            let canvases = find_entity_record_by_name(db, parent_name)
                .map(|parent_record| collect_physical_screen_canvases(db, parent_record))
                .unwrap_or_default();
            map.insert(parent_name.clone(), canvases);
        }
        map
    };
    // Pre-resolve MFD default content canvases keyed by geometry name (helper_name).
    // Each MFD binding is looked up by name rather than positionally.
    let mfd_canvases_by_parent: HashMap<String, HashMap<String, (String, Option<String>, Option<String>)>> = {
        let mut map = HashMap::new();
        for spec in &specs {
            let parent_name = &spec.parent_entity_name;
            if map.contains_key(parent_name) {
                continue;
            }
            let canvases = find_entity_record_by_name(db, parent_name)
                .map(|parent_record| collect_mfd_default_canvases(db, parent_record))
                .unwrap_or_default();
            // Always insert (even empty) to prevent re-querying the same parent.
            map.insert(parent_name.clone(), canvases);
        }
        map
    };
    let mut ui_bindings_by_parent = HashMap::<String, Vec<UiBinding>>::new();
    let mut direct_ui_bindings = Vec::with_capacity(specs.len());
    for spec in &specs {
        let binding = ui_binding_for_record(db, &spec.child.record).map(|mut binding| {
            binding.source_entity_name = spec.child.entity_name.clone();
            binding.helper_name = if spec.child.has_geometry {
                None
            } else {
                Some(spec.parent_node_name.clone())
            };
    // For bindings without an inline content canvas, attempt to resolve
            // one from the parent vehicle's data.  MFD screens and physical screens
            // are both matched by geometry name (helper_name).
            if binding.content_canvas_guid.is_none()
                && matches!(binding.binding_kind.as_str(), "physical" | "mfd")
            {
                if binding.binding_kind == "mfd" {
                    // MFD content canvases carry the per-view screen-name loc key
                    // (from `SMFDView.name`) that labels the frame footer.
                    if let Some((guid, name, screen_name_loc_key)) =
                        binding.helper_name.as_deref().and_then(|h| {
                            mfd_canvases_by_parent.get(&spec.parent_entity_name)?.get(h)
                        })
                    {
                        binding.content_canvas_guid = Some(guid.clone());
                        binding.content_canvas_record_name = name.clone();
                        binding.screen_name_loc_key = screen_name_loc_key.clone();
                    }
                } else if let Some((guid, name)) = binding.helper_name.as_deref().and_then(|h| {
                    physical_screen_canvases_by_parent
                        .get(&spec.parent_entity_name)?
                        .get(h)
                }) {
                    binding.content_canvas_guid = Some(guid.clone());
                    binding.content_canvas_record_name = name.clone();
                }
            }
            binding
        });
        if let Some(binding) = binding.clone() {
            // Only fan binding out to the parent's aggregate map if THIS child
            // has no geometry of its own.  When the child has its own CGA, the
            // binding renders on the child object directly; propagating it to
            // the parent would smear the same PNG onto the parent's body via
            // the Blender importer's helper-less fallback.
            if !spec.child.has_geometry {
                ui_bindings_by_parent
                    .entry(spec.parent_entity_name.clone())
                    .or_default()
                    .push(binding);
            }
        }
        direct_ui_bindings.push(binding);
    }

    let mut unique_children = Vec::new();
    let mut unique_child_indices = std::collections::HashMap::new();
    let mut spec_asset_indices = Vec::with_capacity(specs.len());

    for spec in &specs {
        let child = &spec.child;
        if !child.has_geometry {
            spec_asset_indices.push(None);
            continue;
        }

        let cache_key = build_child_payload_cache_key(child);
        let unique_index = if let Some(&index) = unique_child_indices.get(&cache_key) {
            index
        } else {
            let index = unique_children.len();
            unique_children.push(child.clone_payload_source());
            unique_child_indices.insert(cache_key, index);
            index
        };
        spec_asset_indices.push(Some(unique_index));
    }

    let loaded_assets: Vec<Option<LoadedChildPayload>> = unique_children
        .into_par_iter()
        .map(|child| {
            load_child_payload_asset(
                &child,
                db,
                p4k,
                mesh_opts,
                final_material_mode,
                existing_asset_paths,
            )
        })
        .collect();

    specs
        .into_iter()
        .enumerate()
        .filter_map(|(spec_index, spec)| {
            let child = &spec.child;
            let entity_category = entity_record_category(db, &child.record);
            let attach_def_type = entity_record_attach_def_type(db, &child.record);
            let mut ui_bindings = ui_bindings_by_parent
                .get(&child.entity_name)
                .cloned()
                .unwrap_or_default();
            if let Some(binding) = direct_ui_bindings
                .get(spec_index)
                .and_then(|binding| binding.clone())
            {
                if !ui_bindings.iter().any(|existing| existing == &binding) {
                    ui_bindings.push(binding);
                }
            }
            if child.has_geometry {
                let asset_index = spec_asset_indices[spec_index]?;
                let loaded = loaded_assets.get(asset_index)?.as_ref()?;
                if ui_bindings.iter().any(|b| {
                    b.helper_name.is_none()
                        && matches!(b.binding_kind.as_str(), "physical" | "mfd")
                }) {
                    if let Some(name) = derive_screen_helper_name(
                        loaded.nmc.as_ref(),
                        loaded.materials.as_ref(),
                    ) {
                        for b in ui_bindings.iter_mut() {
                            if b.helper_name.is_none()
                                && matches!(b.binding_kind.as_str(), "physical" | "mfd")
                            {
                                b.helper_name = Some(name.clone());
                            }
                        }
                    }
                }
                // Drop inherited bindings whose helper_name doesn't resolve to a
                // node inside the host's own NMC.  This prevents a child-entity
                // binding (e.g. the door control panel's UI) from being smeared
                // onto its geometry-bearing parent (the door ramp) by the
                // helper-less fallback in the Blender importer.
                if let Some(nmc) = loaded.nmc.as_ref() {
                    let node_names: HashSet<String> = nmc
                        .nodes
                        .iter()
                        .map(|n| n.name.to_ascii_lowercase())
                        .collect();
                    ui_bindings.retain(|b| match b.helper_name.as_deref() {
                        None => true,
                        Some(h) => node_names.contains(&h.to_ascii_lowercase()),
                    });
                }
                // Measure each screen's physical display aspect from its
                // render-target (`RTT_Screen`) faces on the host geometry. The
                // UI pipeline uses it to size physical/radar render targets to
                // the real screen shape (square gauges square, the annunciator a
                // wide strip) instead of the shared 16:9 physical-screen canvas.
                // `None` when the screen quad isn't on this host or not
                // measurable; sizing then falls back to canvas/SWF.
                for b in ui_bindings.iter_mut() {
                    if let Some(helper) = b.helper_name.as_deref() {
                        b.ui_screen_aspect_w_over_h =
                            crate::ui_pipeline::screen_aspect::ui_screen_aspect(
                                &loaded.mesh,
                                loaded.nmc.as_ref(),
                                loaded.materials.as_ref(),
                                helper,
                            );
                        // SB_SCREEN_ASPECT_PROBE: per-screen aspect resolution.
                        // `aspect=None` with `mesh_verts=0` => empty mesh (wrong
                        // LOD: small HUD screens are culled in LOD1 — use
                        // `--lod 0`); with non-zero verts => no RTT_Screen submesh
                        // on the helper node (renamed material / wrong host).
                        if std::env::var_os("SB_SCREEN_ASPECT_PROBE").is_some() {
                            eprintln!(
                                "SCREEN_ASPECT helper={helper} kind={} mesh_verts={} aspect={:?}",
                                b.binding_kind,
                                loaded.mesh.positions.len(),
                                b.ui_screen_aspect_w_over_h,
                            );
                        }
                    }
                }
                Some(crate::types::EntityPayload {
                    mesh: loaded.mesh.clone(),
                    materials: loaded.materials.clone(),
                    textures: loaded.textures.clone(),
                    nmc: loaded.nmc.clone(),
                    palette: loaded.palette.clone(),
                    geometry_path: loaded.geometry_path.clone(),
                    material_path: loaded.material_path.clone(),
                    bones: loaded.bones.clone(),
                    skeleton_source_path: loaded.skeleton_source_path.clone(),
                    entity_name: child.entity_name.clone(),
                    entity_category,
                    attach_def_type,
                    parent_node_name: spec.parent_node_name.clone(),
                    parent_entity_name: spec.parent_entity_name.clone(),
                    no_rotation: spec.no_rotation,
                    offset_position: child.offset_position,
                    offset_rotation: child.offset_rotation,
                    detach_direction: child.detach_direction,
                    port_flags: child.port_flags.clone(),
                    ui_bindings,
                })
            } else if child.nmc.is_some() {
                Some(crate::types::EntityPayload {
                    mesh: empty_child_mesh(),
                    materials: None,
                    textures: None,
                    nmc: child.nmc.clone(),
                    palette: None,
                    geometry_path: child.geometry_path.clone().unwrap_or_default(),
                    material_path: child.material_path.clone().unwrap_or_default(),
                    bones: Vec::new(),
                    skeleton_source_path: None,
                    entity_name: child.entity_name.clone(),
                    entity_category,
                    attach_def_type,
                    parent_node_name: spec.parent_node_name.clone(),
                    parent_entity_name: spec.parent_entity_name.clone(),
                    no_rotation: spec.no_rotation,
                    offset_position: child.offset_position,
                    offset_rotation: child.offset_rotation,
                    detach_direction: child.detach_direction,
                    port_flags: child.port_flags.clone(),
                    ui_bindings,
                })
            } else {
                None
            }
        })
        .collect()
}

fn entity_record_category(db: &Database, record: &starbreaker_datacore::types::Record) -> Option<String> {
    db.compile_path::<String>(record.struct_id(), "Category")
        .ok()
        .and_then(|compiled| db.query_single::<String>(&compiled, record).ok().flatten())
        .filter(|category| !category.is_empty())
}

fn entity_record_attach_def_type(
    db: &Database,
    record: &starbreaker_datacore::types::Record,
) -> Option<String> {
    db.compile_path::<String>(record.struct_id(), "Components[SAttachableComponentParams].AttachDef.Type")
        .ok()
        .and_then(|compiled| db.query_single::<String>(&compiled, record).ok().flatten())
        .filter(|attach_def_type| !attach_def_type.is_empty())
}

pub(crate) fn ui_binding_for_record(db: &Database, record: &Record) -> Option<UiBinding> {
    let owner_source_file =
        query_string_path(db, record, "Components[UIOwnerEntityComponentParams].element.sourceFile");
    let runtime_image_source = query_string_path(
        db,
        record,
        "Components[UIRenderToTextureEntityComponentParams].runtimeImageSource",
    );
    let (default_state_name, default_light_color, default_light_intensity_milli) =
        default_display_screen_state(record, db);
    if let Some(canvas_guid) = query_stringish_path(
        db,
        record,
        "Components[SCItemUIViewOwnerParams].dashboardCanvasConfig",
    ) {
        let (canvas_record_name, canvas_record_path) = resolve_record_metadata(db, &canvas_guid);
        let (canvas_widget_canvas_path, canvas_widget_url_postfix, canvas_widget_url_optional, canvas_variable_binding) =
            canvas_widget_context_for_guid(db, &canvas_guid);
        if (canvas_widget_canvas_path.is_some()
            || canvas_widget_url_postfix.is_some()
            || canvas_widget_url_optional.is_some()
            || canvas_variable_binding.is_some())
            && !is_shell_canvas_guid(db, &canvas_guid)
        {
            return Some(UiBinding {
                binding_kind: "physical".to_string(),
                source_entity_name: String::new(),
                helper_name: None,
                default_view: None,
                default_state_is_off: false,
                default_state_name: default_state_name.clone(),
                default_light_color,
                default_light_intensity_milli,
                canvas_guid: Some(canvas_guid),
                canvas_record_name,
                canvas_record_path,
                canvas_widget_canvas_path,
                canvas_widget_url_postfix,
                canvas_widget_url_optional,
                canvas_variable_binding,
                content_canvas_guid: None,
                content_canvas_record_name: None,
                screen_name_loc_key: None,
                dashboard_view_index: None,
                dashboard_screen_slot: None,
                owner_source_file,
                runtime_image_source,
                generated_image_path: None,
                generated_context_manifest_path: None,
                generated_resolved_source_path: None,
                generated_backend: None,
                generated_provenance: None,
                generated_confidence: None,
                ui_screen_aspect_w_over_h: None,
            });
        }
    }
    let layers = query_value_path(db, record, "Components[UIBuildingBlocksEntityComponentParams].layers")
        .or_else(|| query_value_path(db, record, "Components[UIBuildingBlocksEntityComponentParams].layers[BuildingBlocksLayer]"));
    if layers.is_none() {
        if let Some(canvas_guid) = query_stringish_path(
            db,
            record,
            "Components[UIMapEntityComponentParams].uiElementsCanvasGUID",
        )
        .or_else(|| {
            query_stringish_path(
                db,
                record,
                "Components[UIMapEntityComponentParams].starMapParams.uiElementsCanvasGUID",
            )
        }) {
            let (canvas_record_name, canvas_record_path) = resolve_record_metadata(db, &canvas_guid);
            let (canvas_widget_canvas_path, canvas_widget_url_postfix, canvas_widget_url_optional, canvas_variable_binding) =
                canvas_widget_context_for_guid(db, &canvas_guid);
            return Some(UiBinding {
                binding_kind: "radar".to_string(),
                source_entity_name: String::new(),
                helper_name: None,
                default_view: None,
                default_state_is_off: false,
                default_state_name: default_state_name.clone(),
                default_light_color,
                default_light_intensity_milli,
                canvas_guid: Some(canvas_guid),
                canvas_record_name,
                canvas_record_path,
                canvas_widget_canvas_path,
                canvas_widget_url_postfix,
                canvas_widget_url_optional,
                canvas_variable_binding,
                content_canvas_guid: None,
                content_canvas_record_name: None,
                screen_name_loc_key: None,
                dashboard_view_index: None,
                dashboard_screen_slot: None,
                owner_source_file,
                runtime_image_source,
                generated_image_path: None,
                generated_context_manifest_path: None,
                generated_resolved_source_path: None,
                generated_backend: None,
                generated_provenance: None,
                generated_confidence: None,
                ui_screen_aspect_w_over_h: None,
            });
        }
    }

    let layers = layers?;
    let first_layer = match layers {
        Value::Array(values) => values.into_iter().next()?,
        other => other,
    };
    let default_view = value_object(&first_layer, "defaultView")
        .and_then(|default_view| value_object_string(default_view, "name"));
    let canvas_guid = value_object(&first_layer, "defaultView")
        .and_then(|default_view| value_object(default_view, "component"))
        .and_then(|component| value_object(component, "canvas"))
        .and_then(value_stringish)
        .or_else(|| {
            value_object(&first_layer, "element")
                .and_then(|element| value_object(element, "canvas"))
                .and_then(value_stringish)
        })
        .or_else(|| value_object(&first_layer, "canvas").and_then(value_stringish));

    // When the defaultView has no canvas (e.g. the entity's default is an "Off"
    // view), search the layer's views[] array for the first entry that does carry
    // a non-null canvas value.  The found value can be either a GUID or a P4K
    // path string; they must be routed differently:
    //   - GUID  → assign into canvas_guid so it flows through resolve_record_metadata
    //             and canvas_widget_context_for_guid exactly like the primary path.
    //   - Path  → store in canvas_widget_canvas_path as before.
    let canvas_guid_is_absent = canvas_guid
        .as_deref()
        .map(|g| g.is_empty() || g == "null" || is_zero_guid(g))
        .unwrap_or(true);
    // A "shell" canvas is one whose content is selected at runtime via a
    // `BindingsStringField` operation targeting `CanvasReferenceRecord` (e.g.
    // `DigitalSignageCanvas` driven by the `MainCanvasOverride` variable).
    // Such canvases have no concrete widget tree of their own; if a view's
    // `defaultView` points at one, we must fall through to another view that
    // carries a real renderable canvas. This is a structural rule derived from
    // the canvas record schema — see `docs/StarBreaker/ui-screen16x9-investigation.md`.
    let canvas_guid_is_shell = !canvas_guid_is_absent
        && canvas_guid
            .as_deref()
            .map(|g| is_shell_canvas_guid(db, g))
            .unwrap_or(false);
    let need_fallback = canvas_guid_is_absent || canvas_guid_is_shell;
    let (fallback_guid, fallback_canvas_path): (Option<String>, Option<String>) =
        if need_fallback {
            let views_val = value_object(&first_layer, "views");
            let found = views_val
                .and_then(|v| if let Value::Array(a) = v { Some(a) } else { None })
                .and_then(|views| {
                    views.iter().find_map(|view| {
                        let view_name = value_object_string(view, "name");
                        value_object(view, "component")
                            .and_then(|c| value_object(c, "canvas"))
                            .and_then(value_stringish)
                            .filter(|s| !s.is_empty() && s != "null" && !is_zero_guid(s))
                            .filter(|s| !is_shell_canvas_guid(db, s))
                            .map(|s| (view_name, s))
                    })
                })
                .or_else(|| first_non_null_layer_view_canvas(db, record).map(|s| (None, s)));
            match found {
                Some((view_name, val)) => {
                    let (fg, fp) = classify_canvas_fallback_value(db, val);
                    if fg.is_some() {
                        log::debug!(
                            "ui_binding: defaultView canvas absent; using fallback GUID {:?} from \
                             view {:?} (entity record source_entity_name=<unknown>)",
                            fg,
                            view_name,
                        );
                    }
                    (fg, fp)
                }
                None => (None, None),
            }
        } else {
            (None, None)
        };

    // Effective canvas GUID: primary (from defaultView) or fallback (from views[]).
    // Note: canvas_guid may be Some(zero-GUID) which Option::or treats as Some;
    // gate on `need_fallback` so a present-but-zero-or-shell GUID is replaced.
    let effective_canvas_guid = if need_fallback {
        fallback_guid.or(canvas_guid)
    } else {
        canvas_guid
    };

    let (canvas_record_name, canvas_record_path) = effective_canvas_guid
        .as_deref()
        .map(|guid| resolve_record_metadata(db, guid))
        .unwrap_or((None, None));
    let (mut canvas_widget_canvas_path, canvas_widget_url_postfix, canvas_widget_url_optional, canvas_variable_binding) =
        effective_canvas_guid
            .as_deref()
            .map(|guid| canvas_widget_context_for_guid(db, guid))
            .unwrap_or((None, None, None, None));
    // When the canvas_guid resolution yielded no widget-canvas path but we found
    // a fallback P4K path from views[], record it so it appears in the exported scene.
    if canvas_widget_canvas_path.is_none() {
        canvas_widget_canvas_path = fallback_canvas_path;
    }
    let has_radar_component = query_value_path(db, record, "Components[SCItemRadarDisplayScreenParams]").is_some()
        || query_value_path(db, record, "Components[UIMapEntityComponentParams]").is_some();
    let binding_kind = match default_view.as_deref() {
        Some("_mfd") => "mfd",
        Some("_physicalScreen") => "physical",
        _ if has_radar_component => "radar",
        _ => "physical",
    };
    // The originally-resolved defaultView had no concrete content if either
    // its canvas was null/zero OR was a shell. When that's the case, the
    // entity's true default state is "off"; we fall through to a concrete
    // view only to be able to render a non-blank "switched on" image.
    let default_state_is_off = need_fallback;
    Some(UiBinding {
        binding_kind: binding_kind.to_string(),
        source_entity_name: String::new(),
        helper_name: None,
        default_view,
        default_state_is_off,
        default_state_name,
        default_light_color,
        default_light_intensity_milli,
        canvas_guid: effective_canvas_guid,
        canvas_record_name,
        canvas_record_path,
        canvas_widget_canvas_path,
        canvas_widget_url_postfix,
        canvas_widget_url_optional,
        canvas_variable_binding,
        content_canvas_guid: None,
        content_canvas_record_name: None,
        screen_name_loc_key: None,
        dashboard_view_index: None,
        dashboard_screen_slot: None,
        owner_source_file,
        runtime_image_source,
        generated_image_path: None,
        generated_context_manifest_path: None,
        generated_resolved_source_path: None,
        generated_backend: None,
        generated_provenance: None,
        generated_confidence: None,
        ui_screen_aspect_w_over_h: None,
    })
}

/// Build a `physical` UI binding directly from a UIBuildingBlocks canvas GUID.
///
/// Transit-peripheral entities (elevator call consoles / in-lift screens) author
/// their display canvas inline on their `.soc`
/// `EntityComponentUIBuildingBlocks` component, which OVERRIDES the entity class
/// record's default canvas — e.g. the Carrack consoles author
/// `OLD_TransitUIPanelExterior_ANVL` over the generic class
/// `OLD_TransitUIPanelExterior`. That per-instance override is captured during
/// `.soc` parsing as `InteriorMesh::ui_canvas_guid`; this resolves it into a
/// binding the same way the UIBuildingBlocks branch of [`ui_binding_for_record`]
/// resolves a class-record canvas. Returns `None` for an absent/zero/shell
/// canvas.
pub(crate) fn ui_binding_for_building_blocks_canvas(
    db: &Database,
    canvas_guid: &str,
) -> Option<UiBinding> {
    if canvas_guid.is_empty() || canvas_guid == "null" || is_zero_guid(canvas_guid) {
        return None;
    }
    if is_shell_canvas_guid(db, canvas_guid) {
        return None;
    }
    let (canvas_record_name, canvas_record_path) = resolve_record_metadata(db, canvas_guid);
    let (
        canvas_widget_canvas_path,
        canvas_widget_url_postfix,
        canvas_widget_url_optional,
        canvas_variable_binding,
    ) = canvas_widget_context_for_guid(db, canvas_guid);
    Some(UiBinding {
        binding_kind: "physical".to_string(),
        canvas_guid: Some(canvas_guid.to_string()),
        canvas_record_name,
        canvas_record_path,
        canvas_widget_canvas_path,
        canvas_widget_url_postfix,
        canvas_widget_url_optional,
        canvas_variable_binding,
        default_view: Some("_default".to_string()),
        ..Default::default()
    })
}

/// Return `true` when the canvas at `canvas_guid` is a *shell* — i.e. its
/// content is selected at runtime via a `BindingsStringField` operation on
/// `CanvasReferenceRecord`. Such canvases (e.g. `DigitalSignageCanvas`) have
/// no concrete widget tree and would render blank if used directly.
///
/// Structural rule (not name-based):
///   - The canvas record exists in DataCore.
///   - Its `operations[]` contains a `BuildingBlocks_BindingsStringField`
///     whose `field` equals `"CanvasReferenceRecord"`.
fn is_shell_canvas_guid(db: &Database, canvas_guid: &str) -> bool {
    let Some(guid) = parse_guid(canvas_guid) else {
        return false;
    };
    let Some(record) = db.record_by_id(&guid) else {
        return false;
    };
    let q = query_value_path(db, record, "operations[BuildingBlocks_BindingsStringField]");
    let ops: Vec<Value> = match q {
        Some(Value::Array(a)) => a,
        Some(v @ Value::Object { .. }) => vec![v],
        _ => return false,
    };
    // A canvas is a "shell" when one of its operations binds the
    // `CanvasReferenceRecord` field at runtime (e.g. via a
    // `BindingsStringVariable` pointing at `MainCanvasOverride` or
    // `/Screen/CanvasGUID`). The field is stored as an Enum value.
    ops.iter().any(|op| {
        value_object_enum(op, "field").as_deref() == Some("CanvasReferenceRecord")
    })
}

fn canvas_widget_context_for_guid(
    db: &Database,
    canvas_guid: &str,
) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    let Some(guid) = parse_guid(canvas_guid) else {
        return (None, None, None, None);
    };
    let Some(record) = db.record_by_id(&guid) else {
        return (None, None, None, None);
    };
    if let Some(Value::Array(views)) = query_value_path(db, record, "views") {
        let mut fallback = (None, None, None, None);
        for view in views {
            let Some(screens) = value_object(&view, "screens") else {
                continue;
            };
            let screens: Vec<&Value<'_>> = match screens {
                Value::Array(values) => values.iter().collect(),
                other => vec![other],
            };
            for screen in screens {
                let Some(screen_guid) = value_stringish(screen) else {
                    continue;
                };
                if is_zero_guid(&screen_guid) {
                    continue;
                }
                let context = canvas_widget_context_for_guid(db, &screen_guid);
                if context.0.is_some()
                    || context.1.is_some()
                    || context.2.is_some()
                    || context.3.is_some()
                {
                    if context.3.is_some() || context.1.is_some() || context.2.is_some() {
                        return context;
                    }
                    if fallback.0.is_none() {
                        fallback = context;
                    }
                }
            }
        }
        if fallback.0.is_some() || fallback.1.is_some() || fallback.2.is_some() || fallback.3.is_some() {
            return fallback;
        }
    }
    let scene = query_value_path(db, record, "scene");
    let operations = query_value_path(db, record, "operations");

    let mut canvas_path = None;
    let mut url_postfix = None;
    let mut url_optional = None;
    if let Some(Value::Array(items)) = scene {
        for item in items {
            if value_object_string(&item, "_Type_").as_deref() != Some("BuildingBlocks_WidgetCanvas") {
                continue;
            }
            canvas_path = value_object_string(&item, "canvas")
                .filter(|value| !value.is_empty() && value != "null");
            url_postfix = value_object_string(&item, "urlPostfix")
                .filter(|value| !value.is_empty());
            url_optional = value_object_string(&item, "urlOptional")
                .filter(|value| !value.is_empty());
            if canvas_path.is_some() || url_postfix.is_some() || url_optional.is_some() {
                break;
            }
        }
    }

    let mut variable_binding = None;
    if let Some(Value::Array(items)) = operations {
        for item in items {
            if value_object_string(&item, "_Type_").as_deref()
                == Some("BuildingBlocks_BindingsIntegerVariable")
            {
                variable_binding = value_object_string(&item, "binding")
                    .filter(|value| !value.is_empty());
                if variable_binding.is_some() {
                    break;
                }
            }
        }
    }

    (canvas_path, url_postfix, url_optional, variable_binding)
}

/// Return the basename of a file path without its extension.
/// E.g. `"Libraries/UI/FlightController_Annunciator.dcb"` → `"FlightController_Annunciator"`
fn path_basename_no_ext(path: &str) -> &str {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if let Some(dot_pos) = base.rfind('.') { &base[..dot_pos] } else { base }
}

/// Find a `BuildingBlocks_Canvas` record by matching a path string from a dashboard
/// `screens[]` slot.  The `path` may be a bare GUID string, a filename like
/// `"flightcontroller_annunciator.json"`, or a full library path.  GUID lookup is
/// tried first; filename-basename matching is the fallback.
fn find_canvas_record_by_path_or_guid<'a>(db: &'a Database<'a>, path: &str) -> Option<&'a Record> {
    // Try as direct GUID reference first.
    if let Some(guid) = parse_guid(path) {
        if let Some(record) = db.record_by_id(&guid) {
            return Some(record);
        }
    }
    // Fall back: match on the basename (without extension) of the record's file path.
    let needle = path_basename_no_ext(path).to_lowercase();
    if needle.is_empty() {
        return None;
    }
    db.records_by_type_name("BuildingBlocks_Canvas").find(|record| {
        let file_path = db.resolve_string(record.file_name_offset);
        path_basename_no_ext(file_path).to_lowercase() == needle
    })
}

/// Find an `EntityClassDefinition` record by exact name.
fn find_entity_record_by_name<'a>(db: &'a Database<'a>, name: &str) -> Option<&'a Record> {
    let entity_struct = db.struct_id("EntityClassDefinition")?;
    // Record names are stored as "EntityClassDefinition.ShortName". Support matching either
    // the full qualified name or just the short name after the last dot.
    let qualified = format!("EntityClassDefinition.{name}");
    db.records_of_type(entity_struct).find(|record| {
        let record_name = db.resolve_string2(record.name_offset);
        record_name == name || record_name == qualified
    })
}

/// Collect per-physical-screen default content canvases from a dashboard entity
/// that carries `SCItemSeatDashboardParams.PhysicalScreenParams.Screens`.
/// Returns a map from geometry name (e.g. `"screen_flight_hud_left"`) to
/// `(canvas_guid, canvas_record_name)`.
fn collect_physical_screen_canvases(
    db: &Database,
    dashboard_entity_record: &Record,
) -> HashMap<String, (String, Option<String>)> {
    let phys_params = match query_value_path(
        db,
        dashboard_entity_record,
        "Components[SCItemSeatDashboardParams].PhysicalScreenParams",
    ) {
        Some(v) => v,
        None => return HashMap::new(),
    };

    let screens = match value_object(&phys_params, "Screens") {
        Some(Value::Array(v)) => v,
        _ => return HashMap::new(),
    };

    let mut result = HashMap::new();
    for screen in screens.iter() {
        let geom_name = match value_object_string(screen, "geometryName") {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };

        let canvas_record = match value_object(screen, "canvas") {
            Some(Value::Guid(g)) if !g.is_empty() => db.record_by_id(g),
            Some(other) => {
                let path = match value_stringish(other) {
                    Some(p) if !p.is_empty() && p != "null" && !is_zero_guid(&p) => p,
                    _ => continue,
                };
                find_canvas_record_by_path_or_guid(db, &path)
            }
            _ => continue,
        };

        let Some(canvas_record) = canvas_record else { continue };
        if canvas_record.id.is_empty() {
            continue;
        }
        let guid = canvas_record.id.to_string();
        let name = {
            let n = db.resolve_string2(canvas_record.name_offset).to_string();
            if n.is_empty() { None } else { Some(n) }
        };
        result.insert(geom_name, (guid, name));
    }
    result
}

/// Build a map from MFD view-type enum name (e.g. `"eView_TargetStatus"`) to
/// `(canvas_guid, canvas_record_name, screen_name_loc_key)` by scanning all
/// `SMFDView` records. `eView_Off` entries are excluded since they carry no
/// renderable content.
///
/// `landscapeCanvas` is stored as a `DataType::Reference` in DataCore; references are
/// serialised to their file path strings by `to_json_compact`, so we read that JSON
/// to obtain the path and then match it against `BuildingBlocks_Canvas` records.
/// The same JSON carries `name` (e.g. `"@ui_MFD_View_TargetStatus"`), the
/// localization key for the MFD screen title rendered in the frame footer.
fn build_mfd_view_canvas_map(
    db: &Database,
) -> HashMap<String, (String, Option<String>, Option<String>)> {
    let mut map = HashMap::new();
    for record in db.records_by_type_name("SMFDView") {
        let Some(view_type_compiled) = db.compile_path::<String>(record.struct_id(), "viewType").ok() else {
            continue;
        };
        let view_type = match db.query_single::<String>(&view_type_compiled, record).ok().flatten() {
            Some(vt) if !vt.is_empty() && vt != "eView_Off" => vt,
            _ => continue,
        };

        // `landscapeCanvas` is a Reference field; read it (and the screen-name loc
        // key) from the JSON export which follows the reference and serialises it
        // as the referenced record's file path string.
        let record_json = starbreaker_datacore::export::to_json_compact(db, record)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        let canvas_ref = record_json
            .as_ref()
            .and_then(|json| {
                json.get("_RecordValue_")?
                    .get("landscapeCanvas")?
                    .as_str()
                    .map(|s| s.to_string())
            })
            .filter(|s| !s.is_empty() && s != "null")
            .and_then(|path| find_canvas_record_by_path_or_guid(db, &path));
        let Some(canvas_record) = canvas_ref else {
            continue;
        };
        let canvas_guid = canvas_record.id.to_string();
        if canvas_guid.is_empty() || is_zero_guid(&canvas_guid) {
            continue;
        }

        let name = {
            let resolved = db.resolve_string2(canvas_record.name_offset).to_string();
            if resolved.is_empty() {
                None
            } else {
                Some(resolved)
            }
        };
        let screen_name_loc_key = record_json
            .as_ref()
            .and_then(|json| json.get("_RecordValue_")?.get("name")?.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty() && s != "null");
        // Multiple SMFDView records can author the SAME viewType — e.g. the
        // Clipper has two `eView_SelfStatus`: the pilot view
        // (`@ui_MFD_View_SelfStatus`) and a turret-operator variant
        // (`@hud_turret_status`). A plain "keep first" picks whichever the
        // record iteration hits first (the turret variant here → the SELF-STATUS
        // footer mislabelled "TURRET STATUS"). Disambiguate STRUCTURALLY via
        // `prefer_new_view_entry` (every canonical view names its loc key
        // `@ui_MFD_View_<ViewType>`; the turret variant is the lone outlier).
        match map.entry(view_type.clone()) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert((canvas_guid, name, screen_name_loc_key));
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                if prefer_new_view_entry(
                    &view_type,
                    slot.get().2.as_deref(),
                    screen_name_loc_key.as_deref(),
                ) {
                    slot.insert((canvas_guid, name, screen_name_loc_key));
                }
            }
        }
    }
    map
}

/// On a `viewType` collision in the SMFDView map, decide whether the newly-seen
/// record should replace the one already stored. Canonical MFD views name their
/// screen-name loc key `@ui_MFD_View_<ViewType>` (`eView_Shields` →
/// `@ui_MFD_View_Shields`, etc.); the Clipper's turret-operator `eView_SelfStatus`
/// variant breaks that convention (`@hud_turret_status`). Prefer the
/// canonically-named record so the pilot SelfStatus slot labels "SELF STATUS".
fn prefer_new_view_entry(view_type: &str, existing_loc: Option<&str>, new_loc: Option<&str>) -> bool {
    let Some(canonical) = view_type
        .strip_prefix("eView_")
        .map(|t| format!("@ui_MFD_View_{t}"))
    else {
        return false;
    };
    let canonical = canonical.as_str();
    new_loc == Some(canonical) && existing_loc != Some(canonical)
}

/// Collect per-MFD default content canvases from a dashboard entity that carries
/// `SCItemSeatDashboardParams.MFDParams`.  Returns a map from geometry name
/// (e.g. `"Screen_Left_Upper_RTT"`) to
/// `(canvas_guid, canvas_record_name, screen_name_loc_key)`.
///
/// The selection logic mirrors the game's runtime default per the `SMFDModeConfig`
/// `defaultConfiguration` block:
/// - The slot whose geometry name matches `primaryMFD.geometryName` → `primaryMFDScreenView`.
/// - All remaining slots (in `MFDs[]` array order, skipping the primary) →
///   `secondaryMFDScreen1View`, `secondaryMFDScreen2View`, … up to `secondaryMFDScreen5View`.
///
/// `leftCastView` and `rightCastView` are AR portrait overlays rendered in world
/// space — they are **not** assigned to any geometry helper or item port.
pub(crate) fn collect_mfd_default_canvases(
    db: &Database,
    dashboard_entity_record: &Record,
) -> HashMap<String, (String, Option<String>, Option<String>)> {
    let struct_id = dashboard_entity_record.struct_id();

    // All MFD geometry names in MFDs[] array order.
    let all_mfd_names: Vec<String> = db
        .compile_path::<String>(
            struct_id,
            "Components[SCItemSeatDashboardParams].MFDParams.MFDs[SMFD].geometryName",
        )
        .ok()
        .and_then(|c| db.query::<String>(&c, dashboard_entity_record).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|n| !n.is_empty())
        .collect();

    if all_mfd_names.is_empty() {
        return HashMap::new();
    }

    let primary_name = query_string_path(
        db,
        dashboard_entity_record,
        "Components[SCItemSeatDashboardParams].MFDParams.primaryMFD.geometryName",
    );

    let default_base =
        "Components[SCItemSeatDashboardParams].MFDParams.modeConfiguration.defaultConfiguration";
    let default_primary_view = query_string_path(
        db,
        dashboard_entity_record,
        &format!("{default_base}.primaryMFDScreenView"),
    );
    let default_secondary_views: Vec<Option<String>> = (1..=5)
        .map(|i| {
            query_string_path(
                db,
                dashboard_entity_record,
                &format!("{default_base}.secondaryMFDScreen{i}View"),
            )
        })
        .collect();

    // Prefer no-casts operator-mode profiles when present: these match in-game
    // startup MFD layouts for dashboards that don't use cast screens.
    let no_casts_profiles = collect_operator_mode_profiles(
        db,
        dashboard_entity_record,
        "Components[SCItemSeatDashboardParams].MFDParams.modeConfiguration.operatorModeViewConfigurationsNoCasts[SMFDOperatorModeConfig]",
    );
    let cast_profiles = collect_operator_mode_profiles(
        db,
        dashboard_entity_record,
        "Components[SCItemSeatDashboardParams].MFDParams.modeConfiguration.operatorModeViewConfigurations[SMFDOperatorModeConfig]",
    );
    let (primary_view, secondary_views) = select_mfd_mode_views(
        default_primary_view,
        default_secondary_views,
        &no_casts_profiles,
        &cast_profiles,
    );

    let view_canvas = build_mfd_view_canvas_map(db);
    let mut result = HashMap::new();

    // Assign each slot its default view canvas.
    //   - The primary slot → primaryMFDScreenView (e.g. eView_ResourceNetwork).
    //   - All remaining slots in MFDs[] order → secondaryMFDScreen1View, secondaryMFDScreen2View, …
    // Cast views (leftCastView / rightCastView) are AR overlays and are deliberately
    // not assigned to any geometry helper.
    let secondary_view_iter: Vec<&str> = secondary_views.iter().filter_map(|v| v.as_deref()).collect();
    let mut secondary_cursor = 0usize;

    for name in &all_mfd_names {
        let view_opt: Option<&str> = if primary_name.as_deref() == Some(name.as_str()) {
            primary_view.as_deref()
        } else {
            let v = secondary_view_iter.get(secondary_cursor).copied();
            secondary_cursor += 1;
            v
        };
        if let Some(view) = view_opt {
            if let Some(canvas) = view_canvas.get(view) {
                result.insert(name.clone(), canvas.clone());
            }
        }
    }

    result
}

fn value_stringish(value: &Value<'_>) -> Option<String> {
    match value {
        Value::String(text) => (!text.is_empty()).then_some((*text).to_string()),
        Value::Enum(text) => (!text.is_empty()).then_some((*text).to_string()),
        Value::Guid(guid) => Some(guid.to_string()),
        Value::Object { record_id: Some(guid), .. } => Some(guid.to_string()),
        _ => None,
    }
}

fn is_zero_guid(value: &str) -> bool {
    value.trim().trim_matches('{').trim_matches('}').chars().all(|ch| ch == '0' || ch == '-')
}

fn parse_guid(value: &str) -> Option<CigGuid> {
    let trimmed = value.trim().trim_matches('{').trim_matches('}');
    trimmed.parse::<CigGuid>().ok()
}

fn default_display_screen_state(
    record: &Record,
    db: &Database,
) -> (Option<String>, Option<[u8; 4]>, Option<u16>) {
    let names = db
        .compile_path::<String>(
            record.struct_id(),
            "Components[SCItemDisplayScreenComponentParams].screenStates[SDisplayScreenState].statename",
        )
        .ok()
        .and_then(|compiled| db.query::<String>(&compiled, record).ok())
        .unwrap_or_default();
    if names.is_empty() {
        return (None, None, None);
    }

    let lights_on = db
        .compile_path::<bool>(
            record.struct_id(),
            "Components[SCItemDisplayScreenComponentParams].screenStates[SDisplayScreenState].stateLightParams.lightOn",
        )
        .ok()
        .and_then(|compiled| db.query::<bool>(&compiled, record).ok())
        .unwrap_or_default();
    let colors = query_value_vec_path(
        db,
        record,
        "Components[SCItemDisplayScreenComponentParams].screenStates[SDisplayScreenState].stateLightParams.color",
    );
    let intensities = query_value_vec_path(
        db,
        record,
        "Components[SCItemDisplayScreenComponentParams].screenStates[SDisplayScreenState].stateLightParams.intensity",
    );

    let selected_idx = names
        .iter()
        .enumerate()
        .find(|(index, name)| {
            *name == "Normal"
                && lights_on.get(*index).copied().unwrap_or(false)
        })
        .map(|(index, _)| index)
        .or_else(|| {
            names.iter()
                .enumerate()
                .find(|(_, name)| !name.is_empty())
                .map(|(index, _)| index)
        });
    let Some(selected_idx) = selected_idx else {
        return (None, None, None);
    };

    let default_state_name = names.get(selected_idx).cloned();
    let default_light_color = colors.get(selected_idx).and_then(value_rgba8);
    let default_light_intensity_milli = intensities
        .get(selected_idx)
        .and_then(value_f64)
        .map(|value| (value.clamp(0.0, 65.535) * 1000.0).round() as u16);
    (
        default_state_name,
        default_light_color,
        default_light_intensity_milli,
    )
}

fn query_string_path(db: &Database, record: &Record, path: &str) -> Option<String> {
    db.compile_path::<String>(record.struct_id(), path)
        .ok()
        .and_then(|compiled| db.query_single::<String>(&compiled, record).ok().flatten())
        .filter(|value| !value.is_empty())
}

fn query_string_vec_path(db: &Database, record: &Record, path: &str) -> Vec<String> {
    db.compile_path::<String>(record.struct_id(), path)
        .ok()
        .and_then(|compiled| db.query::<String>(&compiled, record).ok())
        .unwrap_or_default()
}

fn collect_operator_mode_profiles(
    db: &Database,
    record: &Record,
    base_path: &str,
) -> Vec<(Option<String>, Vec<Option<String>>)> {
    let primary = query_string_vec_path(db, record, &format!("{base_path}.primaryMFDScreenView"));
    let secondary: Vec<Vec<String>> = (1..=5)
        .map(|i| {
            query_string_vec_path(
                db,
                record,
                &format!("{base_path}.secondaryMFDScreen{i}View"),
            )
        })
        .collect();

    let mut count = primary.len();
    for entries in &secondary {
        count = count.max(entries.len());
    }

    let mut profiles = Vec::with_capacity(count);
    for index in 0..count {
        let primary_view = primary
            .get(index)
            .cloned()
            .filter(|value| !value.is_empty());
        let secondary_views: Vec<Option<String>> = secondary
            .iter()
            .map(|entries| entries.get(index).cloned().filter(|value| !value.is_empty()))
            .collect();
        profiles.push((primary_view, secondary_views));
    }
    profiles
}

fn is_view_off(view: &str) -> bool {
    view.is_empty() || view == "eView_Off"
}

fn mode_profile_has_visible_view(primary: Option<&str>, secondary: &[Option<String>]) -> bool {
    if let Some(value) = primary {
        if !is_view_off(value) {
            return true;
        }
    }
    secondary
        .iter()
        .flatten()
        .any(|value| !is_view_off(value.as_str()))
}

fn select_mfd_mode_views(
    default_primary: Option<String>,
    default_secondary: Vec<Option<String>>,
    no_casts_profiles: &[(Option<String>, Vec<Option<String>>)],
    cast_profiles: &[(Option<String>, Vec<Option<String>>)],
) -> (Option<String>, Vec<Option<String>>) {
    if let Some((primary, secondary)) = no_casts_profiles
        .iter()
        .find(|(primary, secondary)| mode_profile_has_visible_view(primary.as_deref(), secondary))
    {
        return (primary.clone(), secondary.clone());
    }
    if let Some((primary, secondary)) = cast_profiles
        .iter()
        .find(|(primary, secondary)| mode_profile_has_visible_view(primary.as_deref(), secondary))
    {
        return (primary.clone(), secondary.clone());
    }
    (default_primary, default_secondary)
}

fn query_stringish_path(db: &Database, record: &Record, path: &str) -> Option<String> {
    query_value_path(db, record, path).and_then(|value| match value {
        Value::String(text) => (!text.is_empty()).then_some(text.to_string()),
        Value::Enum(text) => (!text.is_empty()).then_some(text.to_string()),
        Value::Guid(guid) => Some(guid.to_string()),
        Value::Object { record_id: Some(guid), .. } => Some(guid.to_string()),
        _ => None,
    })
}

fn query_value_path<'a>(db: &'a Database<'a>, record: &'a Record, path: &str) -> Option<Value<'a>> {
    db.compile_path::<Value>(record.struct_id(), path)
        .ok()
        .and_then(|compiled| db.query_no_references(&compiled, record).ok())
        .and_then(|mut values| if values.is_empty() { None } else { Some(values.remove(0)) })
}

fn query_value_vec_path<'a>(db: &'a Database<'a>, record: &'a Record, path: &str) -> Vec<Value<'a>> {
    db.compile_path::<Value>(record.struct_id(), path)
        .ok()
        .and_then(|compiled| db.query_no_references(&compiled, record).ok())
        .unwrap_or_default()
}

fn first_non_null_layer_view_canvas(db: &Database, record: &Record) -> Option<String> {
    let path = "Components[UIBuildingBlocksEntityComponentParams].layers[BuildingBlocksLayer].views[BuildingBlocksView].component.canvas";
    db.compile_path::<Value>(record.struct_id(), path)
        .ok()
        .and_then(|compiled| db.query_no_references(&compiled, record).ok())
        .into_iter()
        .flatten()
        .filter_map(|value| value_stringish(&value))
        .find(|s| !s.is_empty() && s != "null" && !is_zero_guid(s) && !is_shell_canvas_guid(db, s))
        .or_else(|| first_non_null_layer_view_canvas_from_json(db, record))
}

fn first_non_null_layer_view_canvas_from_json(db: &Database, record: &Record) -> Option<String> {
    let bytes = starbreaker_datacore::export::to_json_compact(db, record).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let components = json
        .get("_RecordValue_")?
        .get("Components")?
        .as_array()?;
    components.iter().find_map(|component| {
        if component.get("_Type_").and_then(|v| v.as_str())
            != Some("UIBuildingBlocksEntityComponentParams")
        {
            return None;
        }
        component.get("layers")?.as_array()?.iter().find_map(|layer| {
            layer.get("views")?.as_array()?.iter().find_map(|view| {
                let canvas = view.get("component")?.get("canvas")?;
                json_stringish(canvas)
                    .filter(|s| !s.is_empty() && s != "null" && !is_zero_guid(s) && !is_shell_canvas_guid(db, s))
            })
        })
    })
}

fn json_stringish(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.get("_RecordId_").and_then(|v| v.as_str()).map(str::to_owned))
}

fn value_object<'a>(value: &'a Value<'a>, key: &str) -> Option<&'a Value<'a>> {
    match value {
        Value::Object { fields, .. } => fields.iter().find(|(name, _)| *name == key).map(|(_, value)| value),
        _ => None,
    }
}

fn value_object_enum(value: &Value<'_>, key: &str) -> Option<String> {
    match value_object(value, key) {
        Some(Value::Enum(text)) if !text.is_empty() => Some((*text).to_string()),
        Some(Value::String(text)) if !text.is_empty() => Some((*text).to_string()),
        _ => None,
    }
}

fn value_object_string(value: &Value<'_>, key: &str) -> Option<String> {
    match value_object(value, key) {
        Some(Value::String(text)) if !text.is_empty() => Some((*text).to_string()),
        Some(Value::Guid(guid)) => Some(guid.to_string()),
        _ => None,
    }
}

fn value_rgba8(value: &Value<'_>) -> Option<[u8; 4]> {
    Some([
        value_object(value, "r").and_then(value_u8)?,
        value_object(value, "g").and_then(value_u8)?,
        value_object(value, "b").and_then(value_u8)?,
        value_object(value, "a").and_then(value_u8)?,
    ])
}

fn value_u8(value: &Value<'_>) -> Option<u8> {
    match value {
        Value::Int8(value) => Some(*value as u8),
        Value::UInt8(value) => Some(*value),
        Value::Int16(value) => u8::try_from(*value).ok(),
        Value::UInt16(value) => u8::try_from(*value).ok(),
        Value::Int32(value) => u8::try_from(*value).ok(),
        Value::UInt32(value) => u8::try_from(*value).ok(),
        _ => None,
    }
}

fn value_f64(value: &Value<'_>) -> Option<f64> {
    match value {
        Value::Float(value) => Some(f64::from(*value)),
        Value::Double(value) => Some(*value),
        Value::Int8(value) => Some(f64::from(*value)),
        Value::UInt8(value) => Some(f64::from(*value)),
        Value::Int16(value) => Some(f64::from(*value)),
        Value::UInt16(value) => Some(f64::from(*value)),
        Value::Int32(value) => Some(f64::from(*value)),
        Value::UInt32(value) => Some(f64::from(*value)),
        Value::Int64(value) => Some(*value as f64),
        Value::UInt64(value) => Some(*value as f64),
        _ => None,
    }
}

fn resolve_record_metadata(db: &Database, guid: &str) -> (Option<String>, Option<String>) {
    let Ok(record_guid) = guid.parse::<CigGuid>() else {
        return (None, None);
    };
    let Some(record) = db.record_by_id(&record_guid) else {
        return (None, None);
    };
    let name = Some(db.resolve_string2(record.name_offset).to_string()).filter(|value| !value.is_empty());
    let path = Some(db.resolve_string(record.file_name_offset).replace('\\', "/"))
        .filter(|value| !value.is_empty());
    (name, path)
}

/// Determine whether a canvas value string is a GUID or a P4K path.
///
/// Returns `(Some(guid), None)` when `value` parses as a valid non-zero UUID.
/// Returns `(None, Some(path))` otherwise (treat as a P4K file-path string).
///
/// Used by the views[] fallback scan to correctly route the found canvas value:
/// GUIDs flow through the normal `canvas_guid` pipeline while P4K paths are
/// stored in `canvas_widget_canvas_path`.
fn classify_canvas_fallback_value(db: &Database, value: String) -> (Option<String>, Option<String>) {
    if let (Some(guid), None) = classify_canvas_fallback_literal(value.clone()) {
        return (Some(guid), None);
    }
    if let Some(record) = find_canvas_record_by_path_or_guid(db, &value) {
        if !record.id.is_empty() {
            return (Some(record.id.to_string()), None);
        }
    }
    (None, Some(value))
}

fn classify_canvas_fallback_literal(value: String) -> (Option<String>, Option<String>) {
    if parse_guid(&value).is_some() && !is_zero_guid(&value) {
        (Some(value), None)
    } else {
        (None, Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefer_new_view_entry_disambiguates_self_status_collision() {
        // Two SMFDView records author eView_SelfStatus: the pilot view
        // (@ui_MFD_View_SelfStatus, canonical) and the turret variant
        // (@hud_turret_status). Whichever is seen first, the canonical pilot
        // loc key must win so the SELF-STATUS footer labels "SELF STATUS".
        // Turret seen first, then the canonical self view → replace.
        assert!(prefer_new_view_entry(
            "eView_SelfStatus",
            Some("@hud_turret_status"),
            Some("@ui_MFD_View_SelfStatus"),
        ));
        // Canonical seen first, then the turret variant → keep the canonical.
        assert!(!prefer_new_view_entry(
            "eView_SelfStatus",
            Some("@ui_MFD_View_SelfStatus"),
            Some("@hud_turret_status"),
        ));
        // Non-colliding canonical views never get displaced (idempotent).
        assert!(!prefer_new_view_entry(
            "eView_Shields",
            Some("@ui_MFD_View_Shields"),
            Some("@ui_MFD_View_Shields"),
        ));
        // A non-eView_ key (no canonical form) never replaces.
        assert!(!prefer_new_view_entry("Weird", Some("a"), Some("b")));
    }

    #[test]
    fn classify_real_guid_returns_guid_slot() {
        let guid = "abcdef01-1234-5678-9abc-def012345678".to_string();
        let (g, p) = classify_canvas_fallback_literal(guid.clone());
        assert_eq!(g, Some(guid));
        assert_eq!(p, None);
    }

    #[test]
    fn classify_zero_guid_returns_path_slot() {
        let zero = "00000000-0000-0000-0000-000000000000".to_string();
        let (g, p) = classify_canvas_fallback_literal(zero.clone());
        assert_eq!(g, None);
        assert_eq!(p, Some(zero));
    }

    #[test]
    fn classify_p4k_path_returns_path_slot() {
        let path = "Data/UI/Canvas/radar_screen.json".to_string();
        let (g, p) = classify_canvas_fallback_literal(path.clone());
        assert_eq!(g, None);
        assert_eq!(p, Some(path));
    }

    #[test]
    fn classify_file_url_returns_path_slot() {
        let url = "file://UI/Canvas/gen_mc_s_target.json".to_string();
        let (g, p) = classify_canvas_fallback_literal(url.clone());
        assert_eq!(g, None);
        assert_eq!(p, Some(url));
    }

    // ── MFD default-view assignment (pure logic, no DataCore required) ──────

    /// Pure view-assignment logic mirroring `collect_mfd_default_canvases`.
    /// Given ordered slot names, an optional primary slot name, and ordered view
    /// enum strings, returns (slot_name, view_enum) pairs.
    fn assign_mfd_default_views<'a>(
        all_mfd_names: &'a [String],
        primary_name: Option<&str>,
        primary_view: Option<&'a str>,
        secondary_views: &'a [Option<String>],
    ) -> Vec<(&'a String, &'a str)> {
        let secondary_strs: Vec<&str> =
            secondary_views.iter().filter_map(|v| v.as_deref()).collect();
        let mut secondary_cursor = 0usize;
        let mut result = Vec::new();
        for name in all_mfd_names {
            let view_opt = if primary_name == Some(name.as_str()) {
                primary_view
            } else {
                let v = secondary_strs.get(secondary_cursor).copied();
                secondary_cursor += 1;
                v
            };
            if let Some(view) = view_opt {
                result.push((name, view));
            }
        }
        result
    }

    fn names(s: &[&str]) -> Vec<String> {
        s.iter().map(|n| n.to_string()).collect()
    }

    fn oviews(s: &[Option<&str>]) -> Vec<Option<String>> {
        s.iter().map(|v| v.map(|s| s.to_string())).collect()
    }

    /// Clipper layout: 3 MFD slots, primary is slot 0 (Left_Upper).
    /// Expected: Left_Upper → primaryMFDScreenView (eView_ResourceNetwork),
    ///           Right_Upper → secondaryMFDScreen1View (eView_Scanning),
    ///           Left_Lower  → secondaryMFDScreen2View (eView_Diagnostics).
    #[test]
    fn mfd_assignment_clipper_layout() {
        let all = names(&["Screen_Left_Upper_RTT", "Screen_Right_Upper_RTT", "Screen_Left_Lower_RTT"]);
        let primary = Some("Screen_Left_Upper_RTT");
        let primary_view = Some("eView_ResourceNetwork");
        let secondary = oviews(&[Some("eView_Scanning"), Some("eView_Diagnostics"), None, None, None]);

        let assignments = assign_mfd_default_views(&all, primary, primary_view, &secondary);
        let map: std::collections::HashMap<&str, &str> = assignments
            .iter()
            .map(|(n, v)| (n.as_str(), *v))
            .collect();

        assert_eq!(map.get("Screen_Left_Upper_RTT"), Some(&"eView_ResourceNetwork"), "primary must get primaryMFDScreenView");
        assert_eq!(map.get("Screen_Right_Upper_RTT"), Some(&"eView_Scanning"), "first secondary must get secondaryMFDScreen1View");
        assert_eq!(map.get("Screen_Left_Lower_RTT"), Some(&"eView_Diagnostics"), "second secondary must get secondaryMFDScreen2View");
    }

    /// When the primary slot is in the middle of the array, the slots before it
    /// should consume secondaryMFD views starting from screen 1, then the primary
    /// gets primaryMFDScreenView, then remaining get the next secondary views.
    #[test]
    fn mfd_assignment_primary_in_middle() {
        let all = names(&["Slot_A", "Slot_B", "Slot_C"]);
        let primary = Some("Slot_B");
        let primary_view = Some("eView_ResourceNetwork");
        let secondary = oviews(&[Some("eView_Scanning"), Some("eView_Diagnostics"), None, None, None]);

        let assignments = assign_mfd_default_views(&all, primary, primary_view, &secondary);
        let map: std::collections::HashMap<&str, &str> = assignments
            .iter()
            .map(|(n, v)| (n.as_str(), *v))
            .collect();

        assert_eq!(map.get("Slot_A"), Some(&"eView_Scanning"));
        assert_eq!(map.get("Slot_B"), Some(&"eView_ResourceNetwork"));
        assert_eq!(map.get("Slot_C"), Some(&"eView_Diagnostics"));
    }

    /// When there is no primary (no primaryMFD pointer), all slots consume
    /// secondary views in order.
    #[test]
    fn mfd_assignment_no_primary() {
        let all = names(&["Slot_A", "Slot_B"]);
        let secondary = oviews(&[Some("eView_Scanning"), Some("eView_Diagnostics"), None, None, None]);

        let assignments = assign_mfd_default_views(&all, None, None, &secondary);
        let map: std::collections::HashMap<&str, &str> = assignments
            .iter()
            .map(|(n, v)| (n.as_str(), *v))
            .collect();

        assert_eq!(map.get("Slot_A"), Some(&"eView_Scanning"));
        assert_eq!(map.get("Slot_B"), Some(&"eView_Diagnostics"));
    }

    /// Cast views (leftCastView, rightCastView) must NOT appear in the output.
    /// The assignment only uses primaryMFDScreenView + secondaryMFDScreenNView.
    #[test]
    fn mfd_assignment_cast_views_excluded() {
        let all = names(&["Screen_Left_Upper_RTT", "Screen_Right_Upper_RTT", "Screen_Left_Lower_RTT"]);
        let primary = Some("Screen_Left_Upper_RTT");
        let primary_view = Some("eView_ResourceNetwork");
        let secondary = oviews(&[Some("eView_Scanning"), Some("eView_Diagnostics"), None, None, None]);

        let assignments = assign_mfd_default_views(&all, primary, primary_view, &secondary);
        // Neither eView_SelfStatus nor eView_TargetStatus (the cast views) should appear.
        for (_, view) in &assignments {
            assert_ne!(*view, "eView_SelfStatus", "leftCastView must not be assigned to any slot");
            assert_ne!(*view, "eView_TargetStatus", "rightCastView must not be assigned to any slot");
        }
    }

    #[test]
    fn mfd_mode_selection_prefers_no_casts_profile() {
        let default_primary = Some("eView_ResourceNetwork".to_string());
        let default_secondary = oviews(&[
            Some("eView_Scanning"),
            Some("eView_Diagnostics"),
            None,
            None,
            None,
        ]);
        let no_casts_profiles = vec![
            (
                Some("eView_SelfStatus".to_string()),
                oviews(&[
                    Some("eView_TargetStatus"),
                    Some("eView_ResourceNetwork"),
                    Some("eView_Scanning"),
                    None,
                    None,
                ]),
            ),
        ];

        let (primary, secondary) = select_mfd_mode_views(
            default_primary,
            default_secondary,
            &no_casts_profiles,
            &[],
        );

        assert_eq!(primary.as_deref(), Some("eView_SelfStatus"));
        assert_eq!(secondary[0].as_deref(), Some("eView_TargetStatus"));
        assert_eq!(secondary[1].as_deref(), Some("eView_ResourceNetwork"));
    }

    #[test]
    fn mfd_mode_selection_falls_back_to_default_when_profiles_off() {
        let default_primary = Some("eView_ResourceNetwork".to_string());
        let default_secondary = oviews(&[
            Some("eView_Scanning"),
            Some("eView_Diagnostics"),
            None,
            None,
            None,
        ]);
        let off_profile = vec![
            (
                Some("eView_Off".to_string()),
                oviews(&[Some("eView_Off"), Some("eView_Off"), None, None, None]),
            ),
        ];

        let (primary, secondary) = select_mfd_mode_views(
            default_primary,
            default_secondary,
            &off_profile,
            &off_profile,
        );

        assert_eq!(primary.as_deref(), Some("eView_ResourceNetwork"));
        assert_eq!(secondary[0].as_deref(), Some("eView_Scanning"));
        assert_eq!(secondary[1].as_deref(), Some("eView_Diagnostics"));
    }
}
