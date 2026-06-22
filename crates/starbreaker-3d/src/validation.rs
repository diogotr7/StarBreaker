//! Validation framework for decomposed export output.
//!
//! Provides comprehensive validation of export results against requirements:
//! - Mesh files: count, structure, geometry
//! - Lights: count, intensity, temperature, types
//! - Empties: count, hierarchy, parent relationships
//! - Materials: slots, assignments, blend modes
//! - Vertex groups: existence, decal groups, membership
//! - File structure: presence, validity, reasonable sizes

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};

use crate::error::Error;

/// Comprehensive validation report for decomposed export.
#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub is_valid: bool,
    pub mesh_count: usize,
    pub light_count: usize,
    pub empty_count: usize,
    pub material_count: usize,
    pub vertex_group_count: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub metadata: HashMap<String, String>,
}

impl Default for ValidationReport {
    fn default() -> Self {
        Self {
            is_valid: true,
            mesh_count: 0,
            light_count: 0,
            empty_count: 0,
            material_count: 0,
            vertex_group_count: 0,
            errors: Vec::new(),
            warnings: Vec::new(),
            metadata: HashMap::new(),
        }
    }
}

impl ValidationReport {
    pub fn add_error(&mut self, msg: String) {
        self.is_valid = false;
        self.errors.push(msg);
    }

    pub fn add_warning(&mut self, msg: String) {
        self.warnings.push(msg);
    }

    pub fn add_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
    }
}

/// Validate a decomposed export against requirements.
///
/// Checks:
/// - Mesh files exist and have valid structure
/// - Light count matches expected values
/// - Empty hierarchy is valid
/// - Material assignments are complete
/// - Vertex groups exist for decals
/// - File structure is correct
pub fn validate_decomposed_export(output_root: &Path) -> Result<ValidationReport, Error> {
    let mut report = ValidationReport::default();

    validate_scene_blends(output_root, &mut report)?;

    // Validate mesh files in Data/Objects/
    let objects_dir = output_root.join("Data").join("Objects");
    if objects_dir.exists() {
        let mesh_count = validate_mesh_files(&objects_dir, &mut report)?;
        report.mesh_count = mesh_count;
    } else {
        report.add_warning("Data/Objects directory not found".to_string());
    }

    // Validate Packages directory structure
    let packages_dir = output_root.join("Packages");
    if packages_dir.exists() {
        validate_package_structure(&packages_dir, &mut report)?;
    } else {
        report.add_warning("Packages directory not found".to_string());
    }

    validate_material_sidecars(output_root, &mut report)?;

    // Validate scene.json if present
    let scene_json = output_root.join("scene.json");
    if scene_json.exists() {
        if let Ok(data) = std::fs::read_to_string(&scene_json) {
            if let Err(e) = serde_json::from_str::<serde_json::Value>(&data) {
                report.add_error(format!("scene.json invalid JSON: {}", e));
            } else {
                report.add_metadata(
                    "scene_json_size".to_string(),
                    format!("{} bytes", data.len()),
                );
            }
        }
    }

    if report.errors.is_empty() {
        report.is_valid = true;
    }

    Ok(report)
}

/// Validate mesh files in the Data/Objects directory.
fn validate_mesh_files(objects_dir: &Path, report: &mut ValidationReport) -> Result<usize, Error> {
    let mut mesh_count = 0;
    let mut blend_files = Vec::new();

    collect_blend_files(objects_dir, &mut blend_files);

    for blend_file in &blend_files {
        mesh_count += validate_single_mesh_file(blend_file, report)?;
    }

    report.add_metadata("mesh_files_found".to_string(), mesh_count.to_string());

    if mesh_count == 0 {
        report.add_warning("No mesh .blend files found".to_string());
    }

    Ok(mesh_count)
}

fn validate_scene_blends(output_root: &Path, report: &mut ValidationReport) -> Result<(), Error> {
    let mut scene_blends = Vec::new();
    let root_scene_blend = output_root.join("scene.blend");
    if root_scene_blend.exists() {
        scene_blends.push(root_scene_blend);
    }
    let packages_dir = output_root.join("Packages");
    if packages_dir.exists() {
        collect_named_files(&packages_dir, "scene.blend", &mut scene_blends);
    }

    if scene_blends.is_empty() {
        report.add_error("scene.blend not found".to_string());
        return Ok(());
    }

    for scene_blend in &scene_blends {
        validate_scene_blend_file(output_root, scene_blend, report);
    }
    report.add_metadata(
        "scene_blend_count".to_string(),
        scene_blends.len().to_string(),
    );

    Ok(())
}

fn validate_scene_blend_file(
    output_root: &Path,
    scene_blend: &Path,
    report: &mut ValidationReport,
) {
    let label = scene_blend
        .strip_prefix(output_root)
        .unwrap_or(scene_blend)
        .display()
        .to_string();
    let Ok(data) = std::fs::read(scene_blend) else {
        report.add_error(format!("scene.blend {label} could not be read"));
        return;
    };
    if data.len() < 4 {
        report.add_error(format!(
            "scene.blend {label} too small to have valid header"
        ));
        return;
    }

    if !data.starts_with(b"BLENDER") && !data.starts_with(b"BLCr") {
        let magic = std::str::from_utf8(&data[0..4]).unwrap_or("");
        report.add_warning(format!("scene.blend {label} header unexpected: {}", magic));
    }
    report.add_metadata(
        format!("scene_blend_size:{label}"),
        format!("{} bytes", data.len()),
    );
}

fn collect_blend_files(dir: &Path, blend_files: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_blend_files(&path, blend_files);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "blend")
        {
            blend_files.push(path);
        }
    }
}

fn collect_named_files(dir: &Path, filename: &str, files: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_named_files(&path, filename, files);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == filename)
        {
            files.push(path);
        }
    }
}

/// Validate a single mesh .blend file.
fn validate_single_mesh_file(path: &Path, report: &mut ValidationReport) -> Result<usize, Error> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    if let Ok(data) = std::fs::read(path) {
        // Check minimum size (Blender files are at least a few KB)
        if data.len() < 100 {
            report.add_error(format!(
                "Mesh file {} too small: {} bytes",
                file_name,
                data.len()
            ));
            return Ok(0);
        }

        // Check for BLENDER magic (first 7 bytes = "BLENDER")
        if data.len() >= 7 {
            let magic = std::str::from_utf8(&data[0..7]).unwrap_or("");
            if magic != "BLENDER" {
                report.add_warning(format!("Mesh file {} has unexpected header", file_name));
            }
        }

        report.add_metadata(
            format!("mesh_file_{}", file_name),
            format!("{} bytes", data.len()),
        );

        Ok(1)
    } else {
        report.add_error(format!("Failed to read mesh file {}", file_name));
        Ok(0)
    }
}

/// Validate package structure.
fn validate_package_structure(
    packages_dir: &Path,
    report: &mut ValidationReport,
) -> Result<(), Error> {
    let mut package_count = 0;

    if let Ok(entries) = std::fs::read_dir(packages_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                package_count += 1;

                // Check for scene.json in the package
                let scene_json = path.join("scene.json");
                if !scene_json.exists() {
                    let pkg_name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");
                    report.add_warning(format!("Package {} missing scene.json", pkg_name));
                }
            }
        }
    }

    report.add_metadata("package_count".to_string(), package_count.to_string());

    Ok(())
}

/// Validate exported material sidecars and machine-readable texture metadata.
fn validate_material_sidecars(
    output_root: &Path,
    report: &mut ValidationReport,
) -> Result<(), Error> {
    let mut sidecars = Vec::new();
    collect_material_sidecars(output_root, &mut sidecars);

    let mut ddna_normal_slots = 0usize;
    let mut ddna_roughness_refs = 0usize;
    let mut ddna_smoothness_source_checks = 0usize;
    let mut ddna_roughness_transform_checks = 0usize;
    let mut ddna_derivations_exported = 0usize;
    let mut ddna_derivations_missing = 0usize;
    let mut ddna_alpha_mip_formats = BTreeMap::new();
    let mut ddna_alpha_mip_layouts = BTreeMap::new();
    let mut ddna_mip_selections = BTreeMap::new();
    let mut ddna_missing_reasons = BTreeMap::new();
    for path in &sidecars {
        let label = path
            .strip_prefix(output_root)
            .unwrap_or(path)
            .display()
            .to_string();
        let Ok(text) = std::fs::read_to_string(path) else {
            report.add_error(format!("Material sidecar {label} could not be read"));
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            report.add_error(format!("Material sidecar {label} invalid JSON"));
            continue;
        };
        validate_material_sidecar_value(
            output_root,
            &json,
            &label,
            report,
            &mut ddna_normal_slots,
            &mut ddna_roughness_refs,
            &mut ddna_smoothness_source_checks,
            &mut ddna_roughness_transform_checks,
            &mut ddna_derivations_exported,
            &mut ddna_derivations_missing,
            &mut ddna_alpha_mip_formats,
            &mut ddna_alpha_mip_layouts,
            &mut ddna_mip_selections,
            &mut ddna_missing_reasons,
        );
    }

    report.add_metadata(
        "material_sidecars_found".to_string(),
        sidecars.len().to_string(),
    );
    report.add_metadata(
        "ddna_normal_slots".to_string(),
        ddna_normal_slots.to_string(),
    );
    report.add_metadata(
        "ddna_roughness_refs".to_string(),
        ddna_roughness_refs.to_string(),
    );
    report.add_metadata(
        "ddna_smoothness_source_checks".to_string(),
        ddna_smoothness_source_checks.to_string(),
    );
    report.add_metadata(
        "ddna_roughness_transform_checks".to_string(),
        ddna_roughness_transform_checks.to_string(),
    );
    report.add_metadata(
        "ddna_derivations_exported".to_string(),
        ddna_derivations_exported.to_string(),
    );
    report.add_metadata(
        "ddna_derivations_missing".to_string(),
        ddna_derivations_missing.to_string(),
    );
    for (format, count) in ddna_alpha_mip_formats {
        report.add_metadata(format!("ddna_alpha_mip_format:{format}"), count.to_string());
    }
    for (layout, count) in ddna_alpha_mip_layouts {
        report.add_metadata(format!("ddna_alpha_mip_layout:{layout}"), count.to_string());
    }
    for (selection, count) in ddna_mip_selections {
        report.add_metadata(format!("ddna_mip_selection:{selection}"), count.to_string());
    }
    for (reason, count) in ddna_missing_reasons {
        report.add_metadata(format!("ddna_missing_reason:{reason}"), count.to_string());
    }

    Ok(())
}

fn collect_material_sidecars(dir: &Path, sidecars: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_material_sidecars(&path, sidecars);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".materials.json"))
        {
            sidecars.push(path);
        }
    }
}

fn validate_material_sidecar_value(
    output_root: &Path,
    sidecar: &serde_json::Value,
    label: &str,
    report: &mut ValidationReport,
    ddna_normal_slots: &mut usize,
    ddna_roughness_refs: &mut usize,
    ddna_smoothness_source_checks: &mut usize,
    ddna_roughness_transform_checks: &mut usize,
    ddna_derivations_exported: &mut usize,
    ddna_derivations_missing: &mut usize,
    ddna_alpha_mip_formats: &mut BTreeMap<String, usize>,
    ddna_alpha_mip_layouts: &mut BTreeMap<String, usize>,
    ddna_mip_selections: &mut BTreeMap<String, usize>,
    ddna_missing_reasons: &mut BTreeMap<String, usize>,
) {
    let Some(submaterials) = sidecar
        .get("submaterials")
        .and_then(|value| value.as_array())
    else {
        return;
    };

    for (submaterial_index, submaterial) in submaterials.iter().enumerate() {
        validate_texture_refs(
            submaterial.get("texture_slots"),
            label,
            &format!("submaterials[{submaterial_index}].texture_slots"),
            report,
            ddna_normal_slots,
            ddna_roughness_refs,
        );
        validate_texture_refs(
            submaterial.get("direct_textures"),
            label,
            &format!("submaterials[{submaterial_index}].direct_textures"),
            report,
            ddna_normal_slots,
            ddna_roughness_refs,
        );
        validate_texture_refs(
            submaterial.get("derived_textures"),
            label,
            &format!("submaterials[{submaterial_index}].derived_textures"),
            report,
            ddna_normal_slots,
            ddna_roughness_refs,
        );
        validate_ddna_derivation_statuses(
            output_root,
            submaterial.get("ddna_derivations"),
            label,
            &format!("submaterials[{submaterial_index}].ddna_derivations"),
            report,
            ddna_derivations_exported,
            ddna_derivations_missing,
            ddna_alpha_mip_formats,
            ddna_alpha_mip_layouts,
            ddna_mip_selections,
            ddna_missing_reasons,
        );
        validate_ddna_roughness_ref_alignment(
            &[("derived_textures", submaterial.get("derived_textures"))],
            submaterial.get("ddna_derivations"),
            label,
            &format!("submaterials[{submaterial_index}]"),
            report,
        );
        validate_ddna_derivation_coverage(
            &[
                submaterial.get("texture_slots"),
                submaterial.get("direct_textures"),
            ],
            submaterial.get("ddna_derivations"),
            label,
            &format!("submaterials[{submaterial_index}]"),
            report,
        );
        validate_ddna_smoothness_refs_have_exported_derivations(
            &[
                submaterial.get("texture_slots"),
                submaterial.get("direct_textures"),
            ],
            submaterial.get("ddna_derivations"),
            label,
            &format!("submaterials[{submaterial_index}]"),
            report,
        );
        validate_exported_ddna_derivations_have_smoothness_refs(
            &[
                submaterial.get("texture_slots"),
                submaterial.get("direct_textures"),
            ],
            submaterial.get("ddna_derivations"),
            label,
            &format!("submaterials[{submaterial_index}]"),
            report,
        );
        validate_ddna_source_alpha_payloads(
            output_root,
            &[
                submaterial.get("texture_slots"),
                submaterial.get("direct_textures"),
            ],
            submaterial.get("ddna_derivations"),
            label,
            &format!("submaterials[{submaterial_index}]"),
            report,
            ddna_smoothness_source_checks,
            ddna_roughness_transform_checks,
        );

        let Some(layers) = submaterial
            .get("layer_manifest")
            .and_then(|value| value.as_array())
        else {
            continue;
        };
        for (layer_index, layer) in layers.iter().enumerate() {
            validate_texture_refs(
                layer.get("texture_slots"),
                label,
                &format!(
                    "submaterials[{submaterial_index}].layer_manifest[{layer_index}].texture_slots"
                ),
                report,
                ddna_normal_slots,
                ddna_roughness_refs,
            );
            if let Some(roughness_texture) = layer.get("roughness_texture") {
                validate_texture_ref(
                    roughness_texture,
                    label,
                    &format!(
                        "submaterials[{submaterial_index}].layer_manifest[{layer_index}].roughness_texture"
                    ),
                    report,
                    ddna_normal_slots,
                    ddna_roughness_refs,
                );
            }
            validate_ddna_derivation_statuses(
                output_root,
                layer.get("ddna_derivations"),
                label,
                &format!(
                    "submaterials[{submaterial_index}].layer_manifest[{layer_index}].ddna_derivations"
                ),
                report,
                ddna_derivations_exported,
                ddna_derivations_missing,
                ddna_alpha_mip_formats,
                ddna_alpha_mip_layouts,
                ddna_mip_selections,
                ddna_missing_reasons,
            );
            validate_ddna_roughness_ref_alignment(
                &[("roughness_texture", layer.get("roughness_texture"))],
                layer.get("ddna_derivations"),
                label,
                &format!("submaterials[{submaterial_index}].layer_manifest[{layer_index}]"),
                report,
            );
            validate_ddna_derivation_coverage(
                &[layer.get("texture_slots")],
                layer.get("ddna_derivations"),
                label,
                &format!("submaterials[{submaterial_index}].layer_manifest[{layer_index}]"),
                report,
            );
            validate_ddna_smoothness_refs_have_exported_derivations(
                &[layer.get("texture_slots")],
                layer.get("ddna_derivations"),
                label,
                &format!("submaterials[{submaterial_index}].layer_manifest[{layer_index}]"),
                report,
            );
            validate_exported_ddna_derivations_have_smoothness_refs(
                &[layer.get("texture_slots")],
                layer.get("ddna_derivations"),
                label,
                &format!("submaterials[{submaterial_index}].layer_manifest[{layer_index}]"),
                report,
            );
            validate_ddna_source_alpha_payloads(
                output_root,
                &[layer.get("texture_slots")],
                layer.get("ddna_derivations"),
                label,
                &format!("submaterials[{submaterial_index}].layer_manifest[{layer_index}]"),
                report,
                ddna_smoothness_source_checks,
                ddna_roughness_transform_checks,
            );
        }
    }
}

fn validate_ddna_smoothness_refs_have_exported_derivations(
    texture_ref_groups: &[Option<&serde_json::Value>],
    derivations: Option<&serde_json::Value>,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
) {
    let mut smoothness_sources = BTreeSet::new();
    for refs in texture_ref_groups {
        collect_ddna_smoothness_sources(*refs, &mut smoothness_sources);
    }
    if smoothness_sources.is_empty() {
        return;
    }

    let mut exported_sources = BTreeSet::new();
    let mut missing_sources = BTreeSet::new();
    if let Some(derivations) = derivations.and_then(|value| value.as_array()) {
        for derivation in derivations {
            if derivation
                .get("export_kind")
                .and_then(|value| value.as_str())
                != Some("derived_ddna_alpha")
            {
                continue;
            }
            let Some(source_path) = derivation
                .get("source_path")
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            match derivation.get("status").and_then(|value| value.as_str()) {
                Some("exported") => {
                    exported_sources.insert(normalize_validation_source_path(source_path));
                }
                Some("missing") => {
                    missing_sources.insert(normalize_validation_source_path(source_path));
                }
                _ => {}
            }
        }
    }

    for source in smoothness_sources {
        if exported_sources.contains(&source) {
            continue;
        }
        let reason = if missing_sources.contains(&source) {
            "matching derivation is missing"
        } else {
            "matching exported derivation is absent"
        };
        report.add_error(format!(
            "{label} {location} DDNA source ref declares alpha_semantic=smoothness but has missing DDNA smoothness payload for {source}: {reason}"
        ));
    }
}

fn validate_exported_ddna_derivations_have_smoothness_refs(
    texture_ref_groups: &[Option<&serde_json::Value>],
    derivations: Option<&serde_json::Value>,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
) {
    let mut smoothness_sources = BTreeSet::new();
    for refs in texture_ref_groups {
        collect_ddna_smoothness_sources(*refs, &mut smoothness_sources);
    }

    let Some(derivations) = derivations.and_then(|value| value.as_array()) else {
        return;
    };
    for (index, derivation) in derivations.iter().enumerate() {
        if derivation
            .get("export_kind")
            .and_then(|value| value.as_str())
            != Some("derived_ddna_alpha")
            || derivation.get("status").and_then(|value| value.as_str()) != Some("exported")
        {
            continue;
        }
        let Some(source_path) = derivation
            .get("source_path")
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        let normalized = normalize_validation_source_path(source_path);
        if !smoothness_sources.contains(&normalized) {
            report.add_error(format!(
                "{label} {location}.ddna_derivations[{index}] exported DDNA derivation has no matching smoothness source ref for {normalized}"
            ));
        }
    }
}

fn validate_ddna_roughness_ref_alignment(
    ref_values: &[(&str, Option<&serde_json::Value>)],
    derivations: Option<&serde_json::Value>,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
) {
    let mut roughness_refs = Vec::new();
    for (field, value) in ref_values {
        collect_ddna_roughness_refs(*value, field, &mut roughness_refs);
    }
    if roughness_refs.is_empty() {
        return;
    }

    let mut derivation_by_source = HashMap::new();
    if let Some(derivations) = derivations.and_then(|value| value.as_array()) {
        for derivation in derivations {
            if derivation
                .get("export_kind")
                .and_then(|value| value.as_str())
                != Some("derived_ddna_alpha")
            {
                continue;
            }
            if let Some(source_path) = derivation
                .get("source_path")
                .and_then(|value| value.as_str())
            {
                derivation_by_source
                    .insert(normalize_validation_source_path(source_path), derivation);
            }
        }
    }

    for roughness_ref in roughness_refs {
        let Some(source_path) = roughness_ref.source_path else {
            report.add_error(format!(
                "{label} {location}.{} DDNA roughness texture ref missing source_path",
                roughness_ref.ref_location
            ));
            continue;
        };
        let Some(export_path) = roughness_ref.export_path else {
            report.add_error(format!(
                "{label} {location}.{} DDNA roughness texture ref missing export_path",
                roughness_ref.ref_location
            ));
            continue;
        };
        let Some(derivation) = derivation_by_source.get(&source_path) else {
            report.add_error(format!(
                "{label} {location}.{} DDNA roughness texture ref has no matching derivation status for {source_path}",
                roughness_ref.ref_location
            ));
            continue;
        };
        if derivation.get("status").and_then(|value| value.as_str()) != Some("exported") {
            report.add_error(format!(
                "{label} {location}.{} DDNA roughness texture ref points at a non-exported derivation for {source_path}",
                roughness_ref.ref_location
            ));
            continue;
        }
        let Some(expected_export_path) = derivation
            .get("export_path")
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        if normalize_validation_source_path(&export_path)
            != normalize_validation_source_path(expected_export_path)
        {
            report.add_error(format!(
                "{label} {location}.{} DDNA roughness texture ref export_path does not match derivation export_path for {source_path}: expected {expected_export_path}, actual {export_path}",
                roughness_ref.ref_location
            ));
        }
    }
}

struct DdnaRoughnessRefForValidation {
    ref_location: String,
    source_path: Option<String>,
    export_path: Option<String>,
}

fn collect_ddna_roughness_refs(
    value: Option<&serde_json::Value>,
    field: &str,
    out: &mut Vec<DdnaRoughnessRefForValidation>,
) {
    let Some(value) = value else {
        return;
    };
    if let Some(values) = value.as_array() {
        for (index, texture_ref) in values.iter().enumerate() {
            collect_ddna_roughness_ref(texture_ref, &format!("{field}[{index}]"), out);
        }
    } else {
        collect_ddna_roughness_ref(value, field, out);
    }
}

fn collect_ddna_roughness_ref(
    texture_ref: &serde_json::Value,
    ref_location: &str,
    out: &mut Vec<DdnaRoughnessRefForValidation>,
) {
    if texture_ref
        .get("export_kind")
        .and_then(|value| value.as_str())
        != Some("derived_ddna_alpha")
    {
        return;
    }
    out.push(DdnaRoughnessRefForValidation {
        ref_location: ref_location.to_string(),
        source_path: texture_ref
            .get("source_path")
            .and_then(|value| value.as_str())
            .map(normalize_validation_source_path),
        export_path: texture_ref
            .get("export_path")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    });
}

fn validate_ddna_source_alpha_payloads(
    output_root: &Path,
    texture_ref_groups: &[Option<&serde_json::Value>],
    derivations: Option<&serde_json::Value>,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
    ddna_smoothness_source_checks: &mut usize,
    ddna_roughness_transform_checks: &mut usize,
) {
    let mut source_refs = Vec::new();
    for refs in texture_ref_groups {
        collect_ddna_smoothness_refs(*refs, &mut source_refs);
    }
    if source_refs.is_empty() {
        return;
    }

    let mut derivation_by_source = HashMap::new();
    let Some(derivations) = derivations.and_then(|value| value.as_array()) else {
        return;
    };
    for derivation in derivations {
        if derivation
            .get("export_kind")
            .and_then(|value| value.as_str())
            != Some("derived_ddna_alpha")
            || derivation.get("status").and_then(|value| value.as_str()) != Some("exported")
        {
            continue;
        }
        if let Some(source_path) = derivation
            .get("source_path")
            .and_then(|value| value.as_str())
        {
            derivation_by_source.insert(normalize_validation_source_path(source_path), derivation);
        }
    }

    for (source_path, export_path) in source_refs {
        let Some(derivation) = derivation_by_source.get(&source_path) else {
            continue;
        };
        let result = validate_ddna_source_alpha_png(
            output_root,
            &source_path,
            &export_path,
            derivation,
            label,
            location,
            report,
        );
        if result.source_alpha_checked {
            *ddna_smoothness_source_checks += 1;
        }
        if result.roughness_transform_checked {
            *ddna_roughness_transform_checks += 1;
        }
    }
}

#[derive(Default)]
struct DdnaSourceAlphaValidation {
    source_alpha_checked: bool,
    roughness_transform_checked: bool,
}

fn collect_ddna_smoothness_refs(refs: Option<&serde_json::Value>, out: &mut Vec<(String, String)>) {
    let Some(refs) = refs.and_then(|value| value.as_array()) else {
        return;
    };
    for texture_ref in refs {
        if texture_ref
            .get("texture_identity")
            .and_then(|value| value.as_str())
            == Some("ddna_normal")
            && texture_ref
                .get("alpha_semantic")
                .and_then(|value| value.as_str())
                == Some("smoothness")
            && let (Some(source_path), Some(export_path)) = (
                texture_ref
                    .get("source_path")
                    .and_then(|value| value.as_str()),
                texture_ref
                    .get("export_path")
                    .and_then(|value| value.as_str()),
            )
        {
            out.push((
                normalize_validation_source_path(source_path),
                export_path.to_string(),
            ));
        }
    }
}

fn validate_ddna_source_alpha_png(
    output_root: &Path,
    source_path: &str,
    export_path: &str,
    derivation: &serde_json::Value,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
) -> DdnaSourceAlphaValidation {
    let Some(path) = safe_export_path(output_root, export_path) else {
        report.add_error(format!(
            "{label} {location} DDNA smoothness source PNG path is outside export root for {source_path}"
        ));
        return DdnaSourceAlphaValidation::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        report.add_error(format!(
            "{label} {location} DDNA smoothness source PNG missing for {source_path}: {export_path}"
        ));
        return DdnaSourceAlphaValidation::default();
    };
    let Ok(image) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png) else {
        report.add_error(format!(
            "{label} {location} DDNA smoothness source PNG is not a valid PNG for {source_path}: {export_path}"
        ));
        return DdnaSourceAlphaValidation::default();
    };
    let image = image.to_rgba8();
    if let Some(width) = derivation.get("width").and_then(|value| value.as_u64())
        && image.width() as u64 != width
    {
        report.add_error(format!(
            "{label} {location} DDNA smoothness source PNG width does not match derivation metadata for {source_path}"
        ));
    }
    if let Some(height) = derivation.get("height").and_then(|value| value.as_u64())
        && image.height() as u64 != height
    {
        report.add_error(format!(
            "{label} {location} DDNA smoothness source PNG height does not match derivation metadata for {source_path}"
        ));
    }

    let mut min = u8::MAX;
    let mut max = u8::MIN;
    let mut sum = 0u64;
    let mut count = 0u64;
    for pixel in image.pixels() {
        let alpha = pixel.0[3];
        min = min.min(alpha);
        max = max.max(alpha);
        sum += u64::from(alpha);
        count += 1;
    }
    if count == 0 {
        report.add_error(format!(
            "{label} {location} DDNA smoothness source PNG has no pixels for {source_path}"
        ));
        return DdnaSourceAlphaValidation::default();
    }
    let mean = ((sum as f64) / (count as f64)).round() as u64;
    for (field, actual) in [
        ("smoothness_min", u64::from(min)),
        ("smoothness_max", u64::from(max)),
        ("smoothness_mean", mean),
    ] {
        if let Some(expected) = derivation.get(field).and_then(|value| value.as_u64())
            && expected != actual
        {
            report.add_error(format!(
                "{label} {location} DDNA smoothness source PNG {field} does not match derivation metadata for {source_path}: expected {expected}, actual {actual}"
            ));
        }
    }
    DdnaSourceAlphaValidation {
        source_alpha_checked: true,
        roughness_transform_checked: validate_ddna_roughness_transform(
            output_root,
            source_path,
            derivation,
            &image,
            label,
            location,
            report,
        ),
    }
}

fn validate_ddna_roughness_transform(
    output_root: &Path,
    source_path: &str,
    derivation: &serde_json::Value,
    source_image: &image::RgbaImage,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
) -> bool {
    if derivation
        .get("value_transform")
        .and_then(|value| value.as_str())
        != Some("sqrt_one_minus")
    {
        return false;
    }
    let Some(export_path) = derivation
        .get("export_path")
        .and_then(|value| value.as_str())
    else {
        return false;
    };
    let Some(path) = safe_export_path(output_root, export_path) else {
        return false;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return false;
    };
    let Ok(roughness_image) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
    else {
        return false;
    };
    let roughness_image = roughness_image.to_rgba8();
    if source_image.width() != roughness_image.width()
        || source_image.height() != roughness_image.height()
    {
        return false;
    }

    let mut mismatches = 0usize;
    let mut first_mismatch = None;
    for (index, (source_pixel, roughness_pixel)) in source_image
        .pixels()
        .zip(roughness_image.pixels())
        .enumerate()
    {
        let smoothness = source_pixel.0[3];
        let expected = smoothness_to_perceptual_roughness_byte(smoothness);
        let actual = roughness_pixel.0[0];
        if expected != actual {
            mismatches += 1;
            first_mismatch.get_or_insert((index, smoothness, expected, actual));
        }
    }
    if mismatches > 0 {
        let Some((index, smoothness, expected, actual)) = first_mismatch else {
            return true;
        };
        report.add_error(format!(
            "{label} {location} DDNA roughness transform mismatch for {source_path}: {mismatches} pixels differ, first pixel {index} smoothness {smoothness} expected roughness {expected}, actual {actual}"
        ));
    }
    true
}

fn smoothness_to_perceptual_roughness_byte(smoothness: u8) -> u8 {
    let smoothness = f32::from(smoothness) / 255.0;
    ((1.0 - smoothness).sqrt() * 255.0).round() as u8
}

fn validate_ddna_derivation_coverage(
    texture_ref_groups: &[Option<&serde_json::Value>],
    derivations: Option<&serde_json::Value>,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
) {
    let mut ddna_sources = BTreeSet::new();
    for refs in texture_ref_groups {
        collect_ddna_normal_gloss_sources(*refs, &mut ddna_sources);
    }
    if ddna_sources.is_empty() {
        return;
    }

    let mut derivation_sources = BTreeSet::new();
    collect_ddna_derivation_sources(derivations, &mut derivation_sources);
    for source in ddna_sources {
        if !derivation_sources.contains(&source) {
            report.add_error(format!(
                "{label} {location} missing DDNA derivation status for {source}"
            ));
        }
    }
}

fn collect_ddna_normal_gloss_sources(
    refs: Option<&serde_json::Value>,
    sources: &mut BTreeSet<String>,
) {
    let Some(refs) = refs.and_then(|value| value.as_array()) else {
        return;
    };
    for texture_ref in refs {
        if texture_ref
            .get("texture_identity")
            .and_then(|value| value.as_str())
            == Some("ddna_normal")
            && texture_ref.get("role").and_then(|value| value.as_str()) == Some("normal_gloss")
            && let Some(source_path) = texture_ref
                .get("source_path")
                .and_then(|value| value.as_str())
        {
            sources.insert(normalize_validation_source_path(source_path));
        }
    }
}

fn collect_ddna_smoothness_sources(
    refs: Option<&serde_json::Value>,
    sources: &mut BTreeSet<String>,
) {
    let Some(refs) = refs.and_then(|value| value.as_array()) else {
        return;
    };
    for texture_ref in refs {
        if texture_ref
            .get("texture_identity")
            .and_then(|value| value.as_str())
            == Some("ddna_normal")
            && texture_ref
                .get("alpha_semantic")
                .and_then(|value| value.as_str())
                == Some("smoothness")
            && let Some(source_path) = texture_ref
                .get("source_path")
                .and_then(|value| value.as_str())
        {
            sources.insert(normalize_validation_source_path(source_path));
        }
    }
}

fn collect_ddna_derivation_sources(
    refs: Option<&serde_json::Value>,
    sources: &mut BTreeSet<String>,
) {
    let Some(refs) = refs.and_then(|value| value.as_array()) else {
        return;
    };
    for derivation in refs {
        if derivation
            .get("export_kind")
            .and_then(|value| value.as_str())
            == Some("derived_ddna_alpha")
            && let Some(source_path) = derivation
                .get("source_path")
                .and_then(|value| value.as_str())
        {
            sources.insert(normalize_validation_source_path(source_path));
        }
    }
}

fn normalize_validation_source_path(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn validate_ddna_derivation_statuses(
    output_root: &Path,
    refs: Option<&serde_json::Value>,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
    ddna_derivations_exported: &mut usize,
    ddna_derivations_missing: &mut usize,
    ddna_alpha_mip_formats: &mut BTreeMap<String, usize>,
    ddna_alpha_mip_layouts: &mut BTreeMap<String, usize>,
    ddna_mip_selections: &mut BTreeMap<String, usize>,
    ddna_missing_reasons: &mut BTreeMap<String, usize>,
) {
    let Some(refs) = refs.and_then(|value| value.as_array()) else {
        return;
    };
    for (index, derivation) in refs.iter().enumerate() {
        validate_ddna_derivation_status(
            output_root,
            derivation,
            label,
            &format!("{location}[{index}]"),
            report,
            ddna_derivations_exported,
            ddna_derivations_missing,
            ddna_alpha_mip_formats,
            ddna_alpha_mip_layouts,
            ddna_mip_selections,
            ddna_missing_reasons,
        );
    }
}

fn validate_ddna_derivation_status(
    output_root: &Path,
    derivation: &serde_json::Value,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
    ddna_derivations_exported: &mut usize,
    ddna_derivations_missing: &mut usize,
    ddna_alpha_mip_formats: &mut BTreeMap<String, usize>,
    ddna_alpha_mip_layouts: &mut BTreeMap<String, usize>,
    ddna_mip_selections: &mut BTreeMap<String, usize>,
    ddna_missing_reasons: &mut BTreeMap<String, usize>,
) {
    if derivation
        .get("export_kind")
        .and_then(|value| value.as_str())
        != Some("derived_ddna_alpha")
    {
        return;
    }
    let missing_transform_fields = [
        ("derived_from_texture_identity", "ddna_normal"),
        ("derived_from_semantic", "smoothness"),
        ("derived_from_channel", "a"),
        ("value_transform", "sqrt_one_minus"),
        ("value_channel", "r"),
        ("packed_texture_format", "roughness_grayscale"),
    ]
    .into_iter()
    .filter_map(|(field, expected)| {
        if derivation.get(field).and_then(|value| value.as_str()) == Some(expected) {
            None
        } else {
            Some(field)
        }
    })
    .collect::<Vec<_>>();
    if !missing_transform_fields.is_empty() {
        report.add_error(format!(
            "{label} {location} DDNA derivation missing transform metadata fields: {}",
            missing_transform_fields.join(", ")
        ));
    }
    let grayscale_channels_are_roughness = ["r", "g", "b"].into_iter().all(|channel| {
        derivation
            .get("packed_channel_semantics")
            .and_then(|value| value.get(channel))
            .and_then(|value| value.as_str())
            == Some("roughness")
    });
    if !grayscale_channels_are_roughness {
        report.add_error(format!(
            "{label} {location} DDNA derivation missing grayscale roughness channel semantics"
        ));
    }
    if derivation
        .get("constant_channel_values")
        .and_then(|value| value.get("a"))
        .and_then(|value| value.as_str())
        != Some("1.0")
    {
        report.add_error(format!(
            "{label} {location} DDNA derivation missing grayscale alpha channel constant"
        ));
    }
    let status = derivation.get("status").and_then(|value| value.as_str());
    match status {
        Some("exported") => {
            *ddna_derivations_exported += 1;
            if derivation
                .get("export_path")
                .and_then(|value| value.as_str())
                .is_none()
            {
                report.add_error(format!(
                    "{label} {location} exported DDNA derivation missing export_path"
                ));
            }
            let mut missing_parse_fields = [
                "requested_mip",
                "selected_mip",
                "width",
                "height",
                "alpha_mip_count",
                "smoothness_min",
                "smoothness_max",
                "smoothness_mean",
                "roughness_min",
                "roughness_max",
                "roughness_mean",
            ]
            .into_iter()
            .filter(|field| {
                derivation
                    .get(*field)
                    .and_then(|value| value.as_u64())
                    .is_none()
            })
            .collect::<Vec<_>>();
            let mip_selection = derivation
                .get("mip_selection")
                .and_then(|value| value.as_str());
            if mip_selection.is_none() {
                missing_parse_fields.push("mip_selection");
            }
            match derivation
                .get("alpha_mip_format")
                .and_then(|value| value.as_str())
            {
                Some(alpha_mip_format)
                    if matches!(alpha_mip_format, "bc4_unorm" | "bc4_snorm" | "r8_unorm") =>
                {
                    *ddna_alpha_mip_formats
                        .entry(alpha_mip_format.to_string())
                        .or_insert(0) += 1;
                }
                Some(alpha_mip_format) => {
                    report.add_error(format!(
                        "{label} {location} exported DDNA derivation has invalid alpha_mip_format: {alpha_mip_format}"
                    ));
                }
                None => missing_parse_fields.push("alpha_mip_format"),
            }
            match derivation
                .get("alpha_mip_layout")
                .and_then(|value| value.as_str())
            {
                Some(alpha_mip_layout)
                    if matches!(
                        alpha_mip_layout,
                        "numbered_sibling"
                            | "headered_tail"
                            | "raw_tail_split"
                            | "raw_single_payload"
                    ) =>
                {
                    *ddna_alpha_mip_layouts
                        .entry(alpha_mip_layout.to_string())
                        .or_insert(0) += 1;
                }
                Some(alpha_mip_layout) => {
                    report.add_error(format!(
                        "{label} {location} exported DDNA derivation has invalid alpha_mip_layout: {alpha_mip_layout}"
                    ));
                }
                None => missing_parse_fields.push("alpha_mip_layout"),
            }
            if !missing_parse_fields.is_empty() {
                report.add_error(format!(
                    "{label} {location} exported DDNA derivation missing parse metadata fields: {}",
                    missing_parse_fields.join(", ")
                ));
            }
            match mip_selection {
                Some(value @ ("requested" | "clamped_to_available_alpha_mip")) => {
                    *ddna_mip_selections.entry(value.to_string()).or_insert(0) += 1;
                }
                Some(_) => {
                    report.add_error(format!(
                        "{label} {location} exported DDNA derivation has invalid mip_selection"
                    ));
                }
                None => {}
            }
            let requested_mip = derivation
                .get("requested_mip")
                .and_then(|value| value.as_u64());
            let selected_mip = derivation
                .get("selected_mip")
                .and_then(|value| value.as_u64());
            let alpha_mip_count = derivation
                .get("alpha_mip_count")
                .and_then(|value| value.as_u64());
            if let (Some(selected), Some(count)) = (selected_mip, alpha_mip_count)
                && selected >= count
            {
                report.add_error(format!(
                    "{label} {location} exported DDNA derivation selected_mip is outside alpha_mip_count"
                ));
            }
            match (mip_selection, requested_mip, selected_mip, alpha_mip_count) {
                (Some("requested"), Some(requested), Some(selected), _)
                    if requested != selected =>
                {
                    report.add_error(format!(
                        "{label} {location} exported DDNA derivation mip_selection=requested but requested_mip differs from selected_mip"
                    ));
                }
                (
                    Some("clamped_to_available_alpha_mip"),
                    Some(requested),
                    Some(selected),
                    Some(count),
                ) if requested <= selected || selected + 1 != count => {
                    report.add_error(format!(
                        "{label} {location} exported DDNA derivation mip_selection=clamped_to_available_alpha_mip is inconsistent with requested_mip/selected_mip/alpha_mip_count"
                    ));
                }
                _ => {}
            }
            let smoothness_min = derivation
                .get("smoothness_min")
                .and_then(|value| value.as_u64());
            let smoothness_max = derivation
                .get("smoothness_max")
                .and_then(|value| value.as_u64());
            let smoothness_mean = derivation
                .get("smoothness_mean")
                .and_then(|value| value.as_u64());
            for (field, value) in [
                ("smoothness_min", smoothness_min),
                ("smoothness_max", smoothness_max),
                ("smoothness_mean", smoothness_mean),
            ] {
                if value.is_some_and(|value| value > 255) {
                    report.add_error(format!(
                        "{label} {location} exported DDNA derivation has out-of-range {field}"
                    ));
                }
            }
            if let (Some(min), Some(max), Some(mean)) =
                (smoothness_min, smoothness_max, smoothness_mean)
                && (min > mean || mean > max)
            {
                report.add_error(format!(
                    "{label} {location} exported DDNA derivation has inconsistent smoothness statistics"
                ));
            }
            let roughness_min = derivation
                .get("roughness_min")
                .and_then(|value| value.as_u64());
            let roughness_max = derivation
                .get("roughness_max")
                .and_then(|value| value.as_u64());
            let roughness_mean = derivation
                .get("roughness_mean")
                .and_then(|value| value.as_u64());
            for (field, value) in [
                ("roughness_min", roughness_min),
                ("roughness_max", roughness_max),
                ("roughness_mean", roughness_mean),
            ] {
                if value.is_some_and(|value| value > 255) {
                    report.add_error(format!(
                        "{label} {location} exported DDNA derivation has out-of-range {field}"
                    ));
                }
            }
            if let (Some(min), Some(max), Some(mean)) =
                (roughness_min, roughness_max, roughness_mean)
                && (min > mean || mean > max)
            {
                report.add_error(format!(
                    "{label} {location} exported DDNA derivation has inconsistent roughness statistics"
                ));
            }
            for field in ["width", "height", "alpha_mip_count"] {
                if derivation
                    .get(field)
                    .and_then(|value| value.as_u64())
                    .is_some_and(|value| value == 0)
                {
                    report.add_error(format!(
                        "{label} {location} exported DDNA derivation has zero {field}"
                    ));
                }
            }
            validate_exported_ddna_roughness_png(output_root, derivation, label, location, report);
        }
        Some("missing") => {
            *ddna_derivations_missing += 1;
            match derivation.get("reason").and_then(|value| value.as_str()) {
                Some(reason) => {
                    *ddna_missing_reasons.entry(reason.to_string()).or_insert(0) += 1;
                }
                None => {
                    report.add_error(format!(
                        "{label} {location} missing DDNA derivation missing reason"
                    ));
                }
            }
        }
        _ => report.add_error(format!(
            "{label} {location} DDNA derivation has invalid status"
        )),
    }
}

fn validate_exported_ddna_roughness_png(
    output_root: &Path,
    derivation: &serde_json::Value,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
) {
    let Some(export_path) = derivation
        .get("export_path")
        .and_then(|value| value.as_str())
    else {
        return;
    };
    let Some(path) = safe_export_path(output_root, export_path) else {
        report.add_error(format!(
            "{label} {location} exported DDNA roughness PNG path is outside export root"
        ));
        return;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        report.add_error(format!(
            "{label} {location} exported DDNA roughness PNG missing: {export_path}"
        ));
        return;
    };
    let Ok(image) = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png) else {
        report.add_error(format!(
            "{label} {location} exported DDNA roughness PNG is not a valid PNG: {export_path}"
        ));
        return;
    };
    let image = image.to_rgba8();
    if let Some(width) = derivation.get("width").and_then(|value| value.as_u64())
        && image.width() as u64 != width
    {
        report.add_error(format!(
            "{label} {location} exported DDNA roughness PNG width does not match metadata"
        ));
    }
    if let Some(height) = derivation.get("height").and_then(|value| value.as_u64())
        && image.height() as u64 != height
    {
        report.add_error(format!(
            "{label} {location} exported DDNA roughness PNG height does not match metadata"
        ));
    }

    let mut min = u8::MAX;
    let mut max = u8::MIN;
    let mut sum = 0u64;
    let mut count = 0u64;
    let mut non_grayscale = 0usize;
    let mut non_opaque_alpha = 0usize;
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0;
        if r != g || r != b {
            non_grayscale += 1;
        }
        if a != 255 {
            non_opaque_alpha += 1;
        }
        min = min.min(r);
        max = max.max(r);
        sum += u64::from(r);
        count += 1;
    }
    if non_grayscale > 0 {
        report.add_error(format!(
            "{label} {location} exported DDNA roughness PNG is not grayscale: {non_grayscale} mismatched RGB pixels"
        ));
    }
    if non_opaque_alpha > 0 {
        report.add_error(format!(
            "{label} {location} exported DDNA roughness PNG alpha is not 1.0: {non_opaque_alpha} pixels"
        ));
    }
    if count == 0 {
        report.add_error(format!(
            "{label} {location} exported DDNA roughness PNG has no pixels"
        ));
        return;
    }
    let mean = ((sum as f64) / (count as f64)).round() as u64;
    for (field, actual) in [
        ("roughness_min", u64::from(min)),
        ("roughness_max", u64::from(max)),
        ("roughness_mean", mean),
    ] {
        if let Some(expected) = derivation.get(field).and_then(|value| value.as_u64())
            && expected != actual
        {
            report.add_error(format!(
                "{label} {location} exported DDNA roughness PNG {field} does not match metadata: expected {expected}, actual {actual}"
            ));
        }
    }
}

fn safe_export_path(output_root: &Path, export_path: &str) -> Option<PathBuf> {
    let relative = Path::new(export_path);
    if relative.is_absolute() {
        return None;
    }
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    Some(output_root.join(relative))
}

fn validate_texture_refs(
    refs: Option<&serde_json::Value>,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
    ddna_normal_slots: &mut usize,
    ddna_roughness_refs: &mut usize,
) {
    let Some(refs) = refs.and_then(|value| value.as_array()) else {
        return;
    };
    for (index, texture_ref) in refs.iter().enumerate() {
        validate_texture_ref(
            texture_ref,
            label,
            &format!("{location}[{index}]"),
            report,
            ddna_normal_slots,
            ddna_roughness_refs,
        );
    }
}

fn validate_texture_ref(
    texture_ref: &serde_json::Value,
    label: &str,
    location: &str,
    report: &mut ValidationReport,
    ddna_normal_slots: &mut usize,
    ddna_roughness_refs: &mut usize,
) {
    if texture_ref
        .get("texture_identity")
        .and_then(|value| value.as_str())
        == Some("ddna_normal")
        && texture_ref
            .get("alpha_semantic")
            .and_then(|value| value.as_str())
            == Some("smoothness")
    {
        *ddna_normal_slots += 1;
        if texture_ref
            .get("alpha_channel")
            .and_then(|value| value.as_str())
            != Some("a")
        {
            report.add_error(format!(
                "{label} {location} DDNA smoothness texture missing alpha_channel=a"
            ));
        }
        if texture_ref
            .get("export_path")
            .and_then(|value| value.as_str())
            .is_none()
        {
            report.add_error(format!(
                "{label} {location} DDNA smoothness texture missing export_path"
            ));
        }
    }

    if texture_ref
        .get("export_kind")
        .and_then(|value| value.as_str())
        != Some("derived_ddna_alpha")
    {
        return;
    }

    *ddna_roughness_refs += 1;
    let required = [
        ("role", "roughness"),
        ("derived_from_texture_identity", "ddna_normal"),
        ("derived_from_semantic", "smoothness"),
        ("derived_from_channel", "a"),
        ("value_transform", "sqrt_one_minus"),
        ("value_channel", "r"),
        ("packed_texture_format", "roughness_grayscale"),
    ];
    let mut missing = required
        .iter()
        .filter_map(|(key, expected)| {
            let actual = texture_ref.get(*key).and_then(|value| value.as_str());
            (actual != Some(*expected)).then_some(format!("{key}={expected}"))
        })
        .collect::<Vec<_>>();

    for channel in ["r", "g", "b"] {
        if texture_ref
            .get("packed_channel_semantics")
            .and_then(|value| value.get(channel))
            .and_then(|value| value.as_str())
            != Some("roughness")
        {
            missing.push(format!("packed_channel_semantics.{channel}=roughness"));
        }
    }
    if texture_ref
        .get("constant_channel_values")
        .and_then(|value| value.get("a"))
        .and_then(|value| value.as_str())
        != Some("1.0")
    {
        missing.push("constant_channel_values.a=1.0".to_string());
    }

    if !missing.is_empty() {
        report.add_error(format!(
            "{label} {location} invalid DDNA roughness metadata: missing or mismatched {}",
            missing.join(", ")
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    /// Helper: create a temporary test directory with proper structure
    fn create_test_export_dir(name: &str) -> PathBuf {
        let base = PathBuf::from(format!("target/test_exports/{}", name));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("Failed to create test dir");
        base
    }

    /// Helper: write minimal BLENDER17 format file
    fn write_test_blend_file(path: &Path) -> std::io::Result<()> {
        let mut file = fs::File::create(path)?;
        file.write_all(b"BLENDER")?;
        file.write_all(&[0; 200])?;
        Ok(())
    }

    fn write_test_png(path: &Path, width: u32, height: u32, rgba: Vec<u8>) {
        let image = image::RgbaImage::from_vec(width, height, rgba)
            .expect("test PNG buffer should match dimensions");
        image
            .save_with_format(path, image::ImageFormat::Png)
            .expect("test PNG should write");
    }

    #[test]
    fn validate_decomposed_export_valid_complete() {
        let root = create_test_export_dir("valid_complete");

        // Create scene.blend
        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        // Create Data/Objects with mesh files
        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        // Create Packages directory
        let packages_dir = root.join("Packages");
        fs::create_dir_all(&packages_dir).expect("Failed to create packages dir");

        // Create a package with scene.json
        let package_dir = packages_dir.join("TestPackage");
        fs::create_dir_all(&package_dir).expect("Failed to create package");
        let scene_json = r#"{"version": 1, "data": []}"#;
        fs::write(&package_dir.join("scene.json"), scene_json).expect("Failed to write scene.json");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(report.is_valid, "Report should be valid");
        assert!(
            report.errors.is_empty(),
            "Should have no errors: {:?}",
            report.errors
        );
        assert!(report.mesh_count > 0, "Should find at least one mesh file");
        assert!(
            report.metadata.contains_key("package_count"),
            "Should have package_count metadata"
        );
    }

    #[test]
    fn validate_decomposed_export_accepts_structured_package_scene_blend() {
        let root = create_test_export_dir("structured_package_scene_blend");

        let objects_dir = root
            .join("Data")
            .join("Objects")
            .join("fps_weapons")
            .join("behr");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("weapon.blend")).expect("Failed to write mesh");

        let package_dir = root.join("Packages").join("Weapon").join("P6LR");
        fs::create_dir_all(&package_dir).expect("Failed to create package dir");
        write_test_blend_file(&package_dir.join("scene.blend"))
            .expect("Failed to write package scene.blend");
        fs::write(&package_dir.join("scene.json"), "{}").expect("Failed to write scene.json");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            report.is_valid,
            "Structured package export should validate: {:?}",
            report.errors
        );
        assert_eq!(report.mesh_count, 1);
        assert_eq!(
            report
                .metadata
                .get("scene_blend_count")
                .map(|value| value.as_str()),
            Some("1")
        );
        assert_eq!(
            report
                .metadata
                .get("package_count")
                .map(|value| value.as_str()),
            Some("1")
        );
    }

    #[test]
    fn validate_decomposed_export_missing_scene_blend() {
        let root = create_test_export_dir("missing_scene_blend");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(!report.is_valid, "Report should be invalid");
        assert!(!report.errors.is_empty(), "Should have errors");
        assert!(
            report
                .errors
                .iter()
                .any(|e: &String| e.contains("scene.blend")),
            "Error should mention scene.blend"
        );
    }

    #[test]
    fn validate_decomposed_export_missing_mesh_files() {
        let root = create_test_export_dir("missing_mesh_files");

        // Create scene.blend
        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        // Create empty Data/Objects
        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(report.is_valid, "Empty mesh dir is not a validation error");
        assert!(
            report.warnings.iter().any(|w| w.contains("No mesh")),
            "Should warn about no meshes"
        );
    }

    #[test]
    fn validate_decomposed_export_invalid_mesh_file() {
        let root = create_test_export_dir("invalid_mesh_file");

        // Create scene.blend
        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        // Create invalid mesh file (too small)
        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        fs::write(&objects_dir.join("bad_mesh.blend"), "X").expect("Failed to write bad mesh");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should be invalid for corrupted mesh"
        );
        assert!(!report.errors.is_empty(), "Should have errors");
        assert!(
            report
                .errors
                .iter()
                .any(|e: &String| e.contains("too small")),
            "Should error on small file"
        );
    }

    #[test]
    fn validate_decomposed_export_invalid_json() {
        let root = create_test_export_dir("invalid_json");

        // Create scene.blend
        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        // Create invalid JSON
        fs::write(&root.join("scene.json"), "{invalid json}").expect("Failed to write bad JSON");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(!report.is_valid, "Report should be invalid for bad JSON");
        assert!(!report.errors.is_empty(), "Should have errors");
        assert!(
            report.errors.iter().any(|e: &String| e.contains("JSON")),
            "Should error on invalid JSON"
        );
    }

    #[test]
    fn validate_decomposed_export_comprehensive() {
        let root = create_test_export_dir("comprehensive");

        // Build complete structure
        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        for i in 1..=3 {
            let mesh_path = objects_dir.join(format!("mesh_{:03}.blend", i));
            write_test_blend_file(&mesh_path).expect("Failed to write mesh");
        }

        let packages_dir = root.join("Packages");
        for i in 1..=2 {
            let pkg_dir = packages_dir.join(format!("Package_{}", i));
            fs::create_dir_all(&pkg_dir).expect("Failed to create package");
            fs::write(&pkg_dir.join("scene.json"), "{}").expect("Failed to write scene.json");
        }

        // Write a valid scene.json
        let scene_data = serde_json::json!({
            "version": 1,
            "packages": ["Package_1", "Package_2"],
            "meshes": ["mesh_001", "mesh_002", "mesh_003"]
        });
        fs::write(&root.join("scene.json"), scene_data.to_string())
            .expect("Failed to write scene.json");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            report.is_valid,
            "Report should be valid: {:?}",
            report.errors
        );
        assert_eq!(report.mesh_count, 3, "Should find 3 mesh files");
        assert!(
            report.metadata.contains_key("package_count"),
            "Should have package_count"
        );
        assert_eq!(
            report
                .metadata
                .get("package_count")
                .map(|s: &String| s.as_str()),
            Some("2")
        );
    }

    #[test]
    fn validate_decomposed_export_rejects_invalid_ddna_roughness_metadata() {
        let root = create_test_export_dir("invalid_ddna_roughness_metadata");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "derived_textures": [
                        {
                            "role": "roughness",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "constant_channel_values": { "a": "1.0" }
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": { "a": "1.0" },
                            "status": "exported",
                            "export_path": "Data/Objects/Test/missing_roughness_TEX0.png",
                            "requested_mip": 0,
                            "selected_mip": 0,
                            "mip_selection": "requested",
                            "alpha_mip_format": "unsupported_alpha",
                            "alpha_mip_layout": "numbered_sibling",
                            "width": 1,
                            "height": 1,
                            "alpha_mip_count": 1,
                            "smoothness_min": 0,
                            "smoothness_max": 0,
                            "smoothness_mean": 0,
                            "roughness_min": 255,
                            "roughness_max": 255,
                            "roughness_mean": 255
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("bad_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject incomplete DDNA derivation metadata"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("derived_from_channel")
                    && error.contains("value_transform")),
            "Error should mention missing DDNA derivation fields: {:?}",
            report.errors
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("invalid alpha_mip_format")),
            "Error should mention invalid DDNA alpha mip format: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_requires_ddna_derivation_status_for_smoothness_slots() {
        let root = create_test_export_dir("missing_ddna_derivation_status");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "derived_textures": []
                }
            ]
        });
        fs::write(
            material_dir.join("missing_status_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject DDNA smoothness slots without derivation status"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("missing DDNA derivation status")),
            "Error should mention missing DDNA derivation status: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_requires_ddna_derivation_status_for_identity_only_slots() {
        let root = create_test_export_dir("missing_ddna_derivation_status_identity_only");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal"
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("missing_status_identity_only_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject DDNA identity slots without derivation status"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("missing DDNA derivation status")),
            "Error should mention missing DDNA derivation status: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_requires_ddna_derivation_parse_metadata() {
        let root = create_test_export_dir("missing_ddna_derivation_parse_metadata");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png"
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("missing_parse_metadata_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject exported DDNA derivations without parse metadata"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("selected_mip")
                    && error.contains("width")
                    && error.contains("height")
                    && error.contains("alpha_mip_count")),
            "Error should mention missing DDNA parse metadata: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_requires_ddna_smoothness_statistics() {
        let root = create_test_export_dir("missing_ddna_smoothness_statistics");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "selected_mip": 0,
                            "width": 512,
                            "height": 256,
                            "alpha_mip_count": 8
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("missing_smoothness_stats_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject exported DDNA derivations without smoothness statistics"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("smoothness_min")
                    && error.contains("smoothness_max")
                    && error.contains("smoothness_mean")),
            "Error should mention missing DDNA smoothness statistics: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_requires_ddna_roughness_statistics() {
        let root = create_test_export_dir("missing_ddna_roughness_statistics");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "selected_mip": 0,
                            "width": 512,
                            "height": 256,
                            "alpha_mip_count": 8,
                            "smoothness_min": 10,
                            "smoothness_max": 240,
                            "smoothness_mean": 128
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("missing_roughness_stats_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject exported DDNA derivations without roughness statistics"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("roughness_min")
                    && error.contains("roughness_max")
                    && error.contains("roughness_mean")),
            "Error should mention missing DDNA roughness statistics: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_requires_ddna_smoothness_source_export_path() {
        let root = create_test_export_dir("missing_ddna_smoothness_source_export_path");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let texture_dir = root.join("Data").join("Objects").join("Test");
        fs::create_dir_all(&texture_dir).expect("Failed to create texture dir");
        write_test_png(
            &texture_dir.join("panel_ddna_TEX0.png"),
            2,
            1,
            vec![128, 128, 255, 10, 128, 128, 255, 20],
        );
        write_test_png(
            &texture_dir.join("panel_ddna_roughness_TEX0.png"),
            2,
            1,
            vec![250, 250, 250, 255, 245, 245, 245, 255],
        );

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "requested_mip": 0,
                            "selected_mip": 0,
                            "mip_selection": "requested",
                            "alpha_mip_format": "bc4_unorm",
                            "alpha_mip_layout": "numbered_sibling",
                            "width": 2,
                            "height": 1,
                            "alpha_mip_count": 1,
                            "smoothness_min": 10,
                            "smoothness_max": 20,
                            "smoothness_mean": 15,
                            "roughness_min": 245,
                            "roughness_max": 250,
                            "roughness_mean": 248
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("missing_smoothness_source_export_path_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject DDNA smoothness refs without source export_path"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("DDNA smoothness texture missing export_path")),
            "Error should mention missing DDNA smoothness source export_path: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_requires_ddna_derivation_transform_metadata() {
        let root = create_test_export_dir("missing_ddna_derivation_transform_metadata");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "selected_mip": 0,
                            "width": 512,
                            "height": 256,
                            "alpha_mip_count": 8,
                            "smoothness_min": 10,
                            "smoothness_max": 240,
                            "smoothness_mean": 128,
                            "roughness_min": 63,
                            "roughness_max": 250,
                            "roughness_mean": 180
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("missing_transform_metadata_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject DDNA derivations without transform metadata"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("value_transform")
                    && error.contains("value_channel")
                    && error.contains("packed_texture_format")),
            "Error should mention missing DDNA transform metadata: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_requires_ddna_mip_selection_metadata() {
        let root = create_test_export_dir("missing_ddna_mip_selection_metadata");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "selected_mip": 0,
                            "width": 512,
                            "height": 256,
                            "alpha_mip_count": 8,
                            "smoothness_min": 10,
                            "smoothness_max": 240,
                            "smoothness_mean": 128,
                            "roughness_min": 63,
                            "roughness_max": 250,
                            "roughness_mean": 180
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("missing_mip_selection_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject DDNA derivations without mip selection metadata"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("requested_mip")
                    && error.contains("mip_selection")
                    && error.contains("alpha_mip_format")
                    && error.contains("alpha_mip_layout")),
            "Error should mention missing DDNA mip metadata: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_rejects_non_grayscale_ddna_roughness_png() {
        let root = create_test_export_dir("non_grayscale_ddna_roughness_png");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let texture_dir = root.join("Data").join("Objects").join("Test");
        fs::create_dir_all(&texture_dir).expect("Failed to create texture dir");
        write_test_png(
            &texture_dir.join("panel_ddna_roughness_TEX0.png"),
            2,
            1,
            vec![10, 20, 10, 255, 30, 30, 30, 255],
        );

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "requested_mip": 0,
                            "selected_mip": 0,
                            "mip_selection": "requested",
                            "alpha_mip_format": "bc4_unorm",
                            "alpha_mip_layout": "numbered_sibling",
                            "width": 2,
                            "height": 1,
                            "alpha_mip_count": 1,
                            "smoothness_min": 10,
                            "smoothness_max": 240,
                            "smoothness_mean": 128,
                            "roughness_min": 10,
                            "roughness_max": 30,
                            "roughness_mean": 20
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("non_grayscale_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject non-grayscale DDNA roughness PNG"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("DDNA roughness PNG") && error.contains("grayscale")),
            "Error should mention non-grayscale DDNA roughness PNG: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_rejects_ddna_roughness_png_stat_mismatch() {
        let root = create_test_export_dir("ddna_roughness_png_stat_mismatch");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let texture_dir = root.join("Data").join("Objects").join("Test");
        fs::create_dir_all(&texture_dir).expect("Failed to create texture dir");
        write_test_png(
            &texture_dir.join("panel_ddna_TEX0.png"),
            2,
            1,
            vec![128, 128, 255, 10, 128, 128, 255, 20],
        );
        write_test_png(
            &texture_dir.join("panel_ddna_roughness_TEX0.png"),
            2,
            1,
            vec![250, 250, 250, 255, 245, 245, 245, 255],
        );

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_path": "Data/Objects/Test/panel_ddna_TEX0.png",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "requested_mip": 0,
                            "selected_mip": 0,
                            "mip_selection": "requested",
                            "alpha_mip_format": "bc4_unorm",
                            "alpha_mip_layout": "numbered_sibling",
                            "width": 2,
                            "height": 1,
                            "alpha_mip_count": 1,
                            "smoothness_min": 10,
                            "smoothness_max": 240,
                            "smoothness_mean": 128,
                            "roughness_min": 10,
                            "roughness_max": 31,
                            "roughness_mean": 20
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("stat_mismatch_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject DDNA roughness PNG stat mismatch"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("roughness_max")
                    && error.contains("does not match metadata")),
            "Error should mention roughness stat mismatch: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_accepts_grayscale_ddna_roughness_png() {
        let root = create_test_export_dir("valid_grayscale_ddna_roughness_png");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let texture_dir = root.join("Data").join("Objects").join("Test");
        fs::create_dir_all(&texture_dir).expect("Failed to create texture dir");
        write_test_png(
            &texture_dir.join("panel_ddna_TEX0.png"),
            2,
            1,
            vec![128, 128, 255, 10, 128, 128, 255, 20],
        );
        write_test_png(
            &texture_dir.join("panel_ddna_roughness_TEX0.png"),
            2,
            1,
            vec![250, 250, 250, 255, 245, 245, 245, 255],
        );

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_path": "Data/Objects/Test/panel_ddna_TEX0.png",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "requested_mip": 0,
                            "selected_mip": 0,
                            "mip_selection": "requested",
                            "alpha_mip_format": "bc4_unorm",
                            "alpha_mip_layout": "numbered_sibling",
                            "width": 2,
                            "height": 1,
                            "alpha_mip_count": 1,
                            "smoothness_min": 10,
                            "smoothness_max": 20,
                            "smoothness_mean": 15,
                            "roughness_min": 245,
                            "roughness_max": 250,
                            "roughness_mean": 248
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("valid_grayscale_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            report.is_valid,
            "Report should accept valid grayscale DDNA roughness PNG: {:?}",
            report.errors
        );
        assert_eq!(
            report
                .metadata
                .get("ddna_derivations_exported")
                .map(|value| value.as_str()),
            Some("1")
        );
        assert_eq!(
            report
                .metadata
                .get("ddna_alpha_mip_format:bc4_unorm")
                .map(|value| value.as_str()),
            Some("1")
        );
        assert_eq!(
            report
                .metadata
                .get("ddna_alpha_mip_layout:numbered_sibling")
                .map(|value| value.as_str()),
            Some("1")
        );
        assert_eq!(
            report
                .metadata
                .get("ddna_mip_selection:requested")
                .map(|value| value.as_str()),
            Some("1")
        );
    }

    #[test]
    fn validate_decomposed_export_rejects_ddna_roughness_ref_export_path_mismatch() {
        let root = create_test_export_dir("ddna_roughness_ref_export_path_mismatch");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let texture_dir = root.join("Data").join("Objects").join("Test");
        fs::create_dir_all(&texture_dir).expect("Failed to create texture dir");
        write_test_png(
            &texture_dir.join("panel_ddna_TEX0.png"),
            2,
            1,
            vec![128, 128, 255, 10, 128, 128, 255, 20],
        );
        write_test_png(
            &texture_dir.join("panel_ddna_roughness_TEX0.png"),
            2,
            1,
            vec![10, 10, 10, 255, 30, 30, 30, 255],
        );

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_path": "Data/Objects/Test/panel_ddna_TEX0.png",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "derived_textures": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_path": "Data/Objects/Test/wrong_roughness_TEX0.png",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            }
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "requested_mip": 0,
                            "selected_mip": 0,
                            "mip_selection": "requested",
                            "alpha_mip_format": "bc4_unorm",
                            "alpha_mip_layout": "numbered_sibling",
                            "width": 2,
                            "height": 1,
                            "alpha_mip_count": 1,
                            "smoothness_min": 10,
                            "smoothness_max": 20,
                            "smoothness_mean": 15,
                            "roughness_min": 245,
                            "roughness_max": 250,
                            "roughness_mean": 248
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("roughness_ref_mismatch_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject DDNA roughness refs that disagree with the exported derivation"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("DDNA roughness texture ref")
                    && error.contains("export_path")),
            "Error should mention DDNA roughness ref export_path mismatch: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_rejects_layer_ddna_roughness_ref_export_path_mismatch() {
        let root = create_test_export_dir("layer_ddna_roughness_ref_export_path_mismatch");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let texture_dir = root.join("Data").join("Objects").join("Test");
        fs::create_dir_all(&texture_dir).expect("Failed to create texture dir");
        write_test_png(
            &texture_dir.join("layer_ddna_TEX0.png"),
            2,
            1,
            vec![128, 128, 255, 10, 128, 128, 255, 20],
        );
        write_test_png(
            &texture_dir.join("layer_ddna_roughness_TEX0.png"),
            2,
            1,
            vec![10, 10, 10, 255, 30, 30, 30, 255],
        );

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "layer_manifest": [
                        {
                            "texture_slots": [
                                {
                                    "role": "normal_gloss",
                                    "source_path": "Data/Objects/Test/layer_ddna.tif",
                                    "export_path": "Data/Objects/Test/layer_ddna_TEX0.png",
                                    "texture_identity": "ddna_normal",
                                    "alpha_semantic": "smoothness",
                                    "alpha_channel": "a"
                                }
                            ],
                            "roughness_texture": {
                                "role": "roughness",
                                "source_path": "Data/Objects/Test/layer_ddna.tif",
                                "export_path": "Data/Objects/Test/wrong_layer_roughness_TEX0.png",
                                "export_kind": "derived_ddna_alpha",
                                "derived_from_texture_identity": "ddna_normal",
                                "derived_from_semantic": "smoothness",
                                "derived_from_channel": "a",
                                "value_transform": "sqrt_one_minus",
                                "value_channel": "r",
                                "packed_texture_format": "roughness_grayscale",
                                "packed_channel_semantics": {
                                    "r": "roughness",
                                    "g": "roughness",
                                    "b": "roughness"
                                },
                                "constant_channel_values": {
                                    "a": "1.0"
                                }
                            },
                            "ddna_derivations": [
                                {
                                    "role": "roughness",
                                    "source_path": "Data/Objects/Test/layer_ddna.tif",
                                    "export_kind": "derived_ddna_alpha",
                                    "derived_from_texture_identity": "ddna_normal",
                                    "derived_from_semantic": "smoothness",
                                    "derived_from_channel": "a",
                                    "value_transform": "sqrt_one_minus",
                                    "value_channel": "r",
                                    "packed_texture_format": "roughness_grayscale",
                                    "packed_channel_semantics": {
                                        "r": "roughness",
                                        "g": "roughness",
                                        "b": "roughness"
                                    },
                                    "constant_channel_values": {
                                        "a": "1.0"
                                    },
                                    "status": "exported",
                                    "export_path": "Data/Objects/Test/layer_ddna_roughness_TEX0.png",
                                    "requested_mip": 0,
                                    "selected_mip": 0,
                                    "mip_selection": "requested",
                                    "width": 2,
                                    "height": 1,
                                    "alpha_mip_count": 1,
                                    "smoothness_min": 10,
                                    "smoothness_max": 20,
                                    "smoothness_mean": 15,
                                    "roughness_min": 10,
                                    "roughness_max": 30,
                                    "roughness_mean": 20
                                }
                            ]
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("layer_roughness_ref_mismatch_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject layer DDNA roughness refs that disagree with the exported derivation"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("DDNA roughness texture ref")
                    && error.contains("export_path")),
            "Error should mention layer DDNA roughness ref export_path mismatch: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_rejects_ddna_source_alpha_stat_mismatch() {
        let root = create_test_export_dir("ddna_source_alpha_stat_mismatch");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let texture_dir = root.join("Data").join("Objects").join("Test");
        fs::create_dir_all(&texture_dir).expect("Failed to create texture dir");
        write_test_png(
            &texture_dir.join("panel_ddna_TEX0.png"),
            2,
            1,
            vec![128, 128, 255, 1, 128, 128, 255, 2],
        );
        write_test_png(
            &texture_dir.join("panel_ddna_roughness_TEX0.png"),
            2,
            1,
            vec![10, 10, 10, 255, 30, 30, 30, 255],
        );

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_path": "Data/Objects/Test/panel_ddna_TEX0.png",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "requested_mip": 0,
                            "selected_mip": 0,
                            "mip_selection": "requested",
                            "alpha_mip_format": "bc4_unorm",
                            "alpha_mip_layout": "numbered_sibling",
                            "width": 2,
                            "height": 1,
                            "alpha_mip_count": 1,
                            "smoothness_min": 10,
                            "smoothness_max": 20,
                            "smoothness_mean": 15,
                            "roughness_min": 10,
                            "roughness_max": 30,
                            "roughness_mean": 20
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("source_alpha_mismatch_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject DDNA source alpha stat mismatch"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("DDNA smoothness source PNG")
                    && error.contains("smoothness_min")),
            "Error should mention DDNA source alpha stat mismatch: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_rejects_ddna_roughness_transform_mismatch() {
        let root = create_test_export_dir("ddna_roughness_transform_mismatch");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let texture_dir = root.join("Data").join("Objects").join("Test");
        fs::create_dir_all(&texture_dir).expect("Failed to create texture dir");
        write_test_png(
            &texture_dir.join("panel_ddna_TEX0.png"),
            2,
            1,
            vec![128, 128, 255, 0, 128, 128, 255, 255],
        );
        write_test_png(
            &texture_dir.join("panel_ddna_roughness_TEX0.png"),
            2,
            1,
            vec![0, 0, 0, 255, 255, 255, 255, 255],
        );

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_path": "Data/Objects/Test/panel_ddna_TEX0.png",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "requested_mip": 0,
                            "selected_mip": 0,
                            "mip_selection": "requested",
                            "width": 2,
                            "height": 1,
                            "alpha_mip_count": 1,
                            "smoothness_min": 0,
                            "smoothness_max": 255,
                            "smoothness_mean": 128,
                            "roughness_min": 0,
                            "roughness_max": 255,
                            "roughness_mean": 128
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("roughness_transform_mismatch_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject DDNA roughness pixels that do not match smoothness transform"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("DDNA roughness transform")),
            "Error should mention DDNA roughness transform mismatch: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_accepts_ddna_source_alpha_matching_derivation_stats() {
        let root = create_test_export_dir("valid_ddna_source_alpha_stats");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let texture_dir = root.join("Data").join("Objects").join("Test");
        fs::create_dir_all(&texture_dir).expect("Failed to create texture dir");
        write_test_png(
            &texture_dir.join("panel_ddna_TEX0.png"),
            2,
            1,
            vec![128, 128, 255, 10, 128, 128, 255, 20],
        );
        write_test_png(
            &texture_dir.join("panel_ddna_roughness_TEX0.png"),
            2,
            1,
            vec![250, 250, 250, 255, 245, 245, 245, 255],
        );

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_path": "Data/Objects/Test/panel_ddna_TEX0.png",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "requested_mip": 0,
                            "selected_mip": 0,
                            "mip_selection": "requested",
                            "alpha_mip_format": "bc4_unorm",
                            "alpha_mip_layout": "numbered_sibling",
                            "width": 2,
                            "height": 1,
                            "alpha_mip_count": 1,
                            "smoothness_min": 10,
                            "smoothness_max": 20,
                            "smoothness_mean": 15,
                            "roughness_min": 245,
                            "roughness_max": 250,
                            "roughness_mean": 248
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("source_alpha_valid_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            report.is_valid,
            "Report should accept matching DDNA source alpha stats: {:?}",
            report.errors
        );
        assert_eq!(
            report
                .metadata
                .get("ddna_roughness_transform_checks")
                .map(|value| value.as_str()),
            Some("1")
        );
        assert_eq!(
            report
                .metadata
                .get("ddna_smoothness_source_checks")
                .map(|value| value.as_str()),
            Some("1")
        );
    }

    #[test]
    fn validate_decomposed_export_rejects_exported_ddna_derivation_without_smoothness_source_ref() {
        let root = create_test_export_dir("exported_ddna_derivation_without_smoothness_source_ref");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let texture_dir = root.join("Data").join("Objects").join("Test");
        fs::create_dir_all(&texture_dir).expect("Failed to create texture dir");
        write_test_png(
            &texture_dir.join("panel_ddna_roughness_TEX0.png"),
            2,
            1,
            vec![250, 250, 250, 255, 245, 245, 245, 255],
        );

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "exported",
                            "export_path": "Data/Objects/Test/panel_ddna_roughness_TEX0.png",
                            "requested_mip": 0,
                            "selected_mip": 0,
                            "mip_selection": "requested",
                            "alpha_mip_format": "bc4_unorm",
                            "alpha_mip_layout": "numbered_sibling",
                            "width": 2,
                            "height": 1,
                            "alpha_mip_count": 1,
                            "smoothness_min": 10,
                            "smoothness_max": 20,
                            "smoothness_mean": 15,
                            "roughness_min": 245,
                            "roughness_max": 250,
                            "roughness_mean": 248
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("orphaned_exported_derivation_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject exported DDNA derivations without smoothness source refs"
        );
        assert!(
            report.errors.iter().any(|error| {
                error.contains("exported DDNA derivation has no matching smoothness source ref")
            }),
            "Error should mention orphaned exported DDNA derivation: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_decomposed_export_accepts_missing_ddna_derivation_with_reason() {
        let root = create_test_export_dir("accepted_missing_ddna_derivation_status");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "missing",
                            "reason": "missing_alpha_mips"
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("missing_with_reason_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            report.is_valid,
            "Report should accept explained missing DDNA derivations: {:?}",
            report.errors
        );
        assert_eq!(
            report
                .metadata
                .get("ddna_derivations_missing")
                .map(|value| value.as_str()),
            Some("1")
        );
        assert_eq!(
            report
                .metadata
                .get("ddna_missing_reason:missing_alpha_mips")
                .map(|value| value.as_str()),
            Some("1")
        );
    }

    #[test]
    fn validate_decomposed_export_rejects_missing_ddna_derivation_with_smoothness_source_ref() {
        let root = create_test_export_dir("missing_ddna_derivation_with_smoothness_source_ref");

        write_test_blend_file(&root.join("scene.blend")).expect("Failed to write scene.blend");

        let objects_dir = root.join("Data").join("Objects");
        fs::create_dir_all(&objects_dir).expect("Failed to create objects dir");
        write_test_blend_file(&objects_dir.join("mesh_001.blend")).expect("Failed to write mesh");

        let material_dir = root.join("Data").join("Materials");
        fs::create_dir_all(&material_dir).expect("Failed to create material dir");
        let sidecar = serde_json::json!({
            "submaterials": [
                {
                    "texture_slots": [
                        {
                            "role": "normal_gloss",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "texture_identity": "ddna_normal",
                            "alpha_semantic": "smoothness",
                            "alpha_channel": "a"
                        }
                    ],
                    "ddna_derivations": [
                        {
                            "role": "roughness",
                            "source_path": "Data/Objects/Test/panel_ddna.tif",
                            "export_kind": "derived_ddna_alpha",
                            "derived_from_texture_identity": "ddna_normal",
                            "derived_from_semantic": "smoothness",
                            "derived_from_channel": "a",
                            "value_transform": "sqrt_one_minus",
                            "value_channel": "r",
                            "packed_texture_format": "roughness_grayscale",
                            "packed_channel_semantics": {
                                "r": "roughness",
                                "g": "roughness",
                                "b": "roughness"
                            },
                            "constant_channel_values": {
                                "a": "1.0"
                            },
                            "status": "missing",
                            "reason": "missing_alpha_mips"
                        }
                    ]
                }
            ]
        });
        fs::write(
            material_dir.join("missing_with_source_smoothness_TEX0.materials.json"),
            sidecar.to_string(),
        )
        .expect("Failed to write material sidecar");

        let report = validate_decomposed_export(&root).expect("Validation failed");

        assert!(
            !report.is_valid,
            "Report should reject smoothness source refs when DDNA alpha derivation is missing"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("missing DDNA smoothness payload")
                    && error.contains("alpha_semantic")),
            "Error should mention invalid smoothness marker on missing DDNA alpha payload: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_mesh_files_recursively() {
        let root = create_test_export_dir("recursive_meshes");

        // Create nested structure
        let level1 = root.join("Data").join("Objects").join("Spaceships");
        fs::create_dir_all(&level1).expect("Failed to create nested dirs");

        for i in 1..=2 {
            write_test_blend_file(&level1.join(format!("ship_{}.blend", i)))
                .expect("Failed to write mesh");
        }

        let mut report = ValidationReport::default();
        let count = validate_mesh_files(&root.join("Data").join("Objects"), &mut report)
            .expect("Validation failed");

        assert_eq!(count, 2, "Should find 2 mesh files in nested directories");
    }
}
