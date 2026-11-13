//! GLB package detection and structural mesh validation for decomposed exports.
use super::{ValidationReport, collect_named_files};
use std::path::Path;

pub(super) fn has_glb_manifests(root: &Path, report: &mut ValidationReport) -> bool {
    let mut scenes = Vec::new();
    collect_named_files(&root.join("Packages"), "scene.json", &mut scenes);
    if root.join("scene.json").is_file() {
        scenes.push(root.join("scene.json"));
    }
    if scenes.is_empty() {
        return false;
    }
    let mut all_glb = true;
    for scene in &scenes {
        match std::fs::read(scene)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| e.to_string())
            }) {
            Ok(value) => all_glb &= value["export_options"]["format"] == "Glb",
            Err(error) => {
                report.add_error(format!("scene.json {}: {error}", scene.display()));
                all_glb = false;
            }
        }
    }
    if all_glb {
        report.add_metadata("glb_package_count".into(), scenes.len().to_string());
    }
    all_glb
}

pub(super) fn validate_mesh(path: &Path, report: &mut ValidationReport) -> usize {
    let result = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| {
            if !bytes.starts_with(b"glTF") {
                return Err("missing GLB magic".into());
            }
            gltf::Gltf::from_slice(&bytes).map_err(|e| e.to_string())
        });
    match result {
        Ok(gltf) => {
            report.material_count += gltf.document.materials().count();
            report.add_metadata(
                format!("glb_meshes:{}", path.display()),
                gltf.document.meshes().count().to_string(),
            );
            1
        }
        Err(error) => {
            report.add_error(format!("Invalid GLB mesh {}: {error}", path.display()));
            0
        }
    }
}
