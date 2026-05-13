use std::fs;
use std::path::Path;

// Phase 6C: Blender Loading Verification Tests
// Tests verify that exported Blender files (scene.blend and individual mesh files)
// are valid, loadable, and have correct structure.

const AURORA_EXPORT_ROOT: &str = "/home/tom/projects/scorg_tools/ships/Packages/RSI Aurora Mk2_LOD0_TEX0";
const AURORA_SCENE_BLEND: &str = "/home/tom/projects/scorg_tools/ships/Packages/RSI Aurora Mk2_LOD0_TEX0/scene.blend";
const AURORA_SHIPS_ROOT: &str = "/home/tom/projects/scorg_tools/ships";

/// Tests that scene.blend file exists and is readable
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_aurora_scene_blend_exists() {
    let path = Path::new(AURORA_SCENE_BLEND);
    assert!(
        path.exists(),
        "Scene file should exist at {}",
        AURORA_SCENE_BLEND
    );
    assert!(
        path.is_file(),
        "Scene blend path should be a file: {}",
        AURORA_SCENE_BLEND
    );
}

/// Tests that scene.blend has valid Zstandard format header
/// (Blender 3.2+ exports with zstandard compression)
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_aurora_scene_blend_has_valid_format() {
    let data = fs::read(AURORA_SCENE_BLEND)
        .expect("Should read scene.blend file");
    
    // Blender .blend files exported with zstandard compression start with:
    // "BLENDER" magic bytes OR zstandard frame header (0x28, 0xB5, 0x2F, 0xFD)
    let is_zstd_compressed = data.len() >= 4 && data[0] == 0x28 && data[1] == 0xB5 && data[2] == 0x2F && data[3] == 0xFD;
    let is_blender = data.len() >= 7 && &data[0..7] == b"BLENDER";
    
    assert!(
        is_zstd_compressed || is_blender,
        "scene.blend should have valid Blender or zstandard header"
    );
}

/// Tests that scene.blend has reasonable file size (> 10KB, < 10MB)
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_aurora_scene_blend_has_valid_size() {
    let metadata = fs::metadata(AURORA_SCENE_BLEND)
        .expect("Should read scene.blend metadata");
    let size = metadata.len();
    
    assert!(size > 10_000, "scene.blend should be > 10KB, got {}", size);
    assert!(
        size < 10_000_000,
        "scene.blend should be < 10MB, got {}",
        size
    );
}

/// Tests that scene.blend JSON metadata (scene.json) exists and is valid
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_aurora_scene_json_exists_and_valid() {
    let scene_json_path = Path::new(AURORA_EXPORT_ROOT).join("scene.json");
    assert!(
        scene_json_path.exists(),
        "scene.json should exist at {}",
        scene_json_path.display()
    );
    
    // Parse JSON to verify structure
    let content = fs::read_to_string(&scene_json_path)
        .expect("Should read scene.json");
    let json: serde_json::Value = serde_json::from_str(&content)
        .expect("scene.json should be valid JSON");
    
    // Verify it has expected structure
    assert!(
        json.is_object(),
        "scene.json root should be an object"
    );
    assert!(
        json.get("children").is_some(),
        "scene.json should have 'children' field"
    );
}

/// Tests that all referenced mesh .blend files in scene.json exist
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_all_referenced_mesh_files_exist() {
    let scene_json_path = Path::new(AURORA_EXPORT_ROOT).join("scene.json");
    let content = fs::read_to_string(&scene_json_path)
        .expect("Should read scene.json");
    let json: serde_json::Value = serde_json::from_str(&content)
        .expect("scene.json should be valid JSON");
    
    let children = json.get("children")
        .and_then(|v| v.as_array())
        .expect("scene.json should have 'children' array");
    
    let mut mesh_count = 0;
    let mut missing_files = Vec::new();
    
    for child in children {
        if let Some(mesh_asset) = child.get("mesh_asset").and_then(|v| v.as_str()) {
            mesh_count += 1;
            let full_path = Path::new(AURORA_SHIPS_ROOT).join(mesh_asset);
            if !full_path.exists() {
                missing_files.push(mesh_asset.to_string());
            }
        }
    }
    
    assert!(
        mesh_count > 0,
        "scene.json should reference at least one mesh file"
    );
    assert!(
        missing_files.is_empty(),
        "All mesh files should exist. Missing: {:?}",
        missing_files
    );
    
    // Verify we have a reasonable number of meshes (Aurora should have 30+)
    assert!(
        mesh_count >= 30,
        "Aurora should have >= 30 mesh components, got {}",
        mesh_count
    );
}

/// Tests that individual mesh .blend files have valid headers
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_mesh_blend_files_have_valid_headers() {
    let scene_json_path = Path::new(AURORA_EXPORT_ROOT).join("scene.json");
    let content = fs::read_to_string(&scene_json_path)
        .expect("Should read scene.json");
    let json: serde_json::Value = serde_json::from_str(&content)
        .expect("scene.json should be valid JSON");
    
    let children = json.get("children")
        .and_then(|v| v.as_array())
        .expect("scene.json should have 'children' array");
    
    let mut invalid_headers = Vec::new();
    let mut checked_count = 0;
    
    for child in children.iter().take(5) {
        // Sample first 5 mesh files for header validation
        if let Some(mesh_asset) = child.get("mesh_asset").and_then(|v| v.as_str()) {
            let full_path = Path::new(AURORA_SHIPS_ROOT).join(mesh_asset);
            if full_path.exists() {
                checked_count += 1;
                if let Ok(data) = fs::read(&full_path) {
                    let is_zstd = data.len() >= 4 && data[0] == 0x28 && data[1] == 0xB5 
                        && data[2] == 0x2F && data[3] == 0xFD;
                    let is_blender = data.len() >= 7 && &data[0..7] == b"BLENDER";
                    
                    if !is_zstd && !is_blender {
                        invalid_headers.push(mesh_asset.to_string());
                    }
                }
            }
        }
    }
    
    assert!(
        checked_count > 0,
        "Should have checked at least one mesh file header"
    );
    assert!(
        invalid_headers.is_empty(),
        "All mesh files should have valid headers. Invalid: {:?}",
        invalid_headers
    );
}

/// Tests that mesh files have reasonable sizes (not empty, not huge)
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_mesh_files_have_valid_sizes() {
    let scene_json_path = Path::new(AURORA_EXPORT_ROOT).join("scene.json");
    let content = fs::read_to_string(&scene_json_path)
        .expect("Should read scene.json");
    let json: serde_json::Value = serde_json::from_str(&content)
        .expect("scene.json should be valid JSON");
    
    let children = json.get("children")
        .and_then(|v| v.as_array())
        .expect("scene.json should have 'children' array");
    
    let mut size_violations = Vec::new();
    
    for child in children {
        if let Some(mesh_asset) = child.get("mesh_asset").and_then(|v| v.as_str()) {
            let full_path = Path::new(AURORA_SHIPS_ROOT).join(mesh_asset);
            if full_path.exists() {
                if let Ok(metadata) = fs::metadata(&full_path) {
                    let size = metadata.len();
                    // Each mesh file should be at least 1KB but less than 50MB
                    if size < 1_000 || size > 50_000_000 {
                        size_violations.push(format!(
                            "{}: {} bytes",
                            mesh_asset,
                            size
                        ));
                    }
                }
            }
        }
    }
    
    assert!(
        size_violations.is_empty(),
        "Mesh files should have reasonable sizes (1KB-50MB). Violations: {:?}",
        size_violations
    );
}

/// Tests that materials metadata files exist
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_materials_metadata_files_exist() {
    let materials_json = Path::new(AURORA_EXPORT_ROOT).join("paints.json");
    assert!(
        materials_json.exists(),
        "paints.json should exist"
    );
    
    let palettes_json = Path::new(AURORA_EXPORT_ROOT).join("palettes.json");
    assert!(
        palettes_json.exists(),
        "palettes.json should exist"
    );
    
    let liveries_json = Path::new(AURORA_EXPORT_ROOT).join("liveries.json");
    assert!(
        liveries_json.exists(),
        "liveries.json should exist"
    );
}

/// Tests that paints.json is valid JSON with expected structure
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_paints_json_valid_structure() {
    let paints_json = Path::new(AURORA_EXPORT_ROOT).join("paints.json");
    let content = fs::read_to_string(&paints_json)
        .expect("Should read paints.json");
    let json: serde_json::Value = serde_json::from_str(&content)
        .expect("paints.json should be valid JSON");
    
    assert!(
        json.is_object(),
        "paints.json root should be an object"
    );
}

/// Tests that palettes.json is valid JSON and contains paint palette data
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_palettes_json_valid_structure() {
    let palettes_json = Path::new(AURORA_EXPORT_ROOT).join("palettes.json");
    let content = fs::read_to_string(&palettes_json)
        .expect("Should read palettes.json");
    let json: serde_json::Value = serde_json::from_str(&content)
        .expect("palettes.json should be valid JSON");
    
    assert!(
        json.is_object() || json.is_array(),
        "palettes.json root should be object or array"
    );
}

/// Tests that animations directory exists with animation metadata
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_animations_directory_exists() {
    let animations_dir = Path::new(AURORA_EXPORT_ROOT).join("animations");
    assert!(
        animations_dir.exists(),
        "animations directory should exist"
    );
    assert!(
        animations_dir.is_dir(),
        "animations should be a directory"
    );
    
    let entries: Vec<_> = fs::read_dir(&animations_dir)
        .expect("Should read animations directory")
        .collect();
    
    assert!(
        entries.len() > 0,
        "animations directory should not be empty"
    );
}

/// Tests that animation files are valid JSON
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_animation_files_valid_json() {
    let animations_dir = Path::new(AURORA_EXPORT_ROOT).join("animations");
    
    for entry in fs::read_dir(&animations_dir)
        .expect("Should read animations directory")
        .take(3)  // Check first 3 animation files
    {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let content = fs::read_to_string(&path)
                    .expect("Should read animation JSON");
                let _json: serde_json::Value = serde_json::from_str(&content)
                    .expect(&format!("Animation JSON should be valid: {}", path.display()));
            }
        }
    }
}

/// Tests that the export package maintains directory structure consistency
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_export_structure_consistency() {
    let root = Path::new(AURORA_EXPORT_ROOT);
    
    // Core files/dirs should exist
    assert!(root.join("scene.blend").exists(), "scene.blend should exist");
    assert!(root.join("scene.json").exists(), "scene.json should exist");
    assert!(root.join("paints.json").exists(), "paints.json should exist");
    assert!(root.join("palettes.json").exists(), "palettes.json should exist");
    assert!(root.join("liveries.json").exists(), "liveries.json should exist");
    assert!(root.join("animations").exists(), "animations dir should exist");
}

/// Tests that mesh asset paths in scene.json are consistent (use forward slashes)
#[ignore = "requires local Aurora export at hard-coded /home/tom/... path"]
#[test]
fn test_scene_json_paths_consistent() {
    let scene_json_path = Path::new(AURORA_EXPORT_ROOT).join("scene.json");
    let content = fs::read_to_string(&scene_json_path)
        .expect("Should read scene.json");
    let json: serde_json::Value = serde_json::from_str(&content)
        .expect("scene.json should be valid JSON");
    
    let children = json.get("children")
        .and_then(|v| v.as_array())
        .expect("scene.json should have 'children' array");
    
    let mut invalid_paths = Vec::new();
    
    for child in children.iter().take(5) {
        if let Some(mesh_asset) = child.get("mesh_asset").and_then(|v| v.as_str()) {
            // Paths should use forward slashes (not backslashes)
            if mesh_asset.contains('\\') {
                invalid_paths.push(mesh_asset.to_string());
            }
            // Paths should start with "Data/"
            if !mesh_asset.starts_with("Data/") {
                invalid_paths.push(format!("{} (doesn't start with Data/)", mesh_asset));
            }
        }
    }
    
    assert!(
        invalid_paths.is_empty(),
        "All mesh asset paths should be consistent. Issues: {:?}",
        invalid_paths
    );
}
