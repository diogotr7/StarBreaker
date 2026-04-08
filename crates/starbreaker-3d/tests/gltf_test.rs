use std::path::Path;

fn read_if_exists(path: &str) -> Option<Vec<u8>> {
    if Path::new(path).exists() {
        Some(std::fs::read(path).expect("failed to read file"))
    } else {
        eprintln!("SKIP: file not found: {path}");
        None
    }
}

use starbreaker_chunks::ChunkFile;
use starbreaker_3d::dequant::dequantize_position;
use starbreaker_3d::ivo::material::MaterialName;
use starbreaker_3d::ivo::skin::{PositionData, SkinMesh};
use starbreaker_3d::{parse_skin, skin_to_glb};

// Tests that require extracted game data on disk are ignored by default.
// Run with: cargo test -- --ignored

#[test]
#[ignore = "requires extracted game data on disk"]
fn parse_ivo_skin2_from_real_file() {
    let Some(data) = read_if_exists(
        "D:/StarCitizen/P4k-470/Data/Objects/Characters/Human/male_v7/body/male_v7_body.skin",
    ) else {
        return;
    };

    let chunk_file = ChunkFile::from_bytes(&data).expect("failed to parse chunk file");
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => panic!("expected IVO format"),
    };

    let skin_entry = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == 0xB8757777)
        .expect("no IvoSkin2 chunk found");

    let skin_data = ivo.chunk_data(skin_entry);
    let skin_mesh = SkinMesh::read(skin_data).expect("failed to parse IvoSkin2");

    assert!(skin_mesh.info.num_vertices > 0, "expected vertices");
    assert!(skin_mesh.info.num_indices > 0, "expected indices");
    assert!(skin_mesh.info.num_submeshes > 0, "expected submeshes");
    assert_eq!(
        skin_mesh.submeshes.len(),
        skin_mesh.info.num_submeshes as usize
    );

    let pos_count = match &skin_mesh.streams.positions {
        PositionData::Quantized(v) => v.len(),
        PositionData::Float(v) => v.len(),
    };
    assert_eq!(
        pos_count, skin_mesh.info.num_vertices as usize,
        "position count mismatch"
    );
    assert_eq!(
        skin_mesh.streams.indices.len(),
        skin_mesh.info.num_indices as usize,
        "index count mismatch"
    );
    assert_eq!(
        skin_mesh.streams.uvs.len(),
        skin_mesh.info.num_vertices as usize,
        "uv count mismatch"
    );

    println!("vertices: {}", skin_mesh.info.num_vertices);
    println!("indices: {}", skin_mesh.info.num_indices);
    println!("submeshes: {}", skin_mesh.info.num_submeshes);
    println!(
        "bounds: {:?} → {:?}",
        skin_mesh.info.min_bound, skin_mesh.info.max_bound
    );
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn dequantize_positions_match_bounds() {
    let Some(data) = read_if_exists(
        "D:/StarCitizen/P4k-470/Data/Objects/Characters/Human/male_v7/body/male_v7_body.skin",
    ) else {
        return;
    };

    let chunk_file = ChunkFile::from_bytes(&data).expect("failed to parse chunk file");
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => panic!("expected IVO format"),
    };

    let skin_entry = ivo
        .chunks()
        .iter()
        .find(|c| c.chunk_type == 0xB8757777)
        .expect("no IvoSkin2 chunk found");
    let skin_mesh = starbreaker_3d::ivo::skin::SkinMesh::read(ivo.chunk_data(skin_entry))
        .expect("failed to parse IvoSkin2");

    let positions = match &skin_mesh.streams.positions {
        PositionData::Quantized(raw) => raw
            .iter()
            .map(|p| dequantize_position(*p, &skin_mesh.info.min_bound, &skin_mesh.info.max_bound))
            .collect::<Vec<_>>(),
        PositionData::Float(f) => f.clone(),
    };

    let mut actual_min = [f32::MAX; 3];
    let mut actual_max = [f32::MIN; 3];
    for pos in &positions {
        for i in 0..3 {
            actual_min[i] = actual_min[i].min(pos[i]);
            actual_max[i] = actual_max[i].max(pos[i]);
        }
    }

    println!("expected min: {:?}", skin_mesh.info.min_bound);
    println!("actual   min: {:?}", actual_min);
    println!("expected max: {:?}", skin_mesh.info.max_bound);
    println!("actual   max: {:?}", actual_max);

    let tolerance = 0.1;
    for i in 0..3 {
        assert!(
            actual_min[i] >= skin_mesh.info.min_bound[i] - tolerance,
            "axis {i}: actual_min {} < expected_min {}",
            actual_min[i],
            skin_mesh.info.min_bound[i]
        );
        assert!(
            actual_max[i] <= skin_mesh.info.max_bound[i] + tolerance,
            "axis {i}: actual_max {} > expected_max {}",
            actual_max[i],
            skin_mesh.info.max_bound[i]
        );
    }
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn parse_mtl_name_from_real_file() {
    let Some(data) = read_if_exists(
        "D:/StarCitizen/P4k-470/Data/Objects/Characters/Human/male_v7/body/male_v7_body.skin",
    ) else {
        return;
    };

    let chunk_file = ChunkFile::from_bytes(&data).expect("failed to parse chunk file");
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => panic!("expected IVO format"),
    };

    let mtl_entries: Vec<_> = ivo
        .chunks()
        .iter()
        .filter(|c| c.chunk_type == 0x83353333)
        .collect();

    assert!(!mtl_entries.is_empty(), "no MtlName chunks found");

    for entry in &mtl_entries {
        let mat = MaterialName::read(ivo.chunk_data(entry)).expect("failed to parse MtlName");
        assert!(!mat.name.is_empty(), "material name should not be empty");
        println!("material: {}", mat.name);
    }
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn parse_skin_from_real_file() {
    let Some(data) = read_if_exists(
        "D:/StarCitizen/P4k-470/Data/Objects/Characters/Human/male_v7/body/male_v7_body.skin",
    ) else {
        return;
    };

    let mesh = parse_skin(&data).expect("parse_skin failed");
    assert!(!mesh.positions.is_empty());
    assert!(!mesh.indices.is_empty());
    assert!(!mesh.submeshes.is_empty());
    println!("positions: {}", mesh.positions.len());
    println!("indices: {}", mesh.indices.len());
    println!("submeshes: {}", mesh.submeshes.len());
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn skin_to_glb_from_real_file() {
    let Some(data) = read_if_exists(
        "D:/StarCitizen/P4k-470/Data/Objects/Characters/Human/male_v7/body/male_v7_body.skin",
    ) else {
        return;
    };

    let glb = skin_to_glb(&data, None).expect("skin_to_glb failed");
    assert!(glb.len() > 12);
    assert_eq!(&glb[0..4], b"glTF");

    let out_path = std::env::temp_dir().join("starbreaker_test_male_v7_body.glb");
    std::fs::write(&out_path, &glb).expect("failed to write GLB");
    println!("wrote {} bytes to {}", glb.len(), out_path.display());
}

#[test]
#[ignore = "requires extracted game data on disk"]
fn skin_to_glb_cgf_file() {
    let Some(data) =
        read_if_exists("D:/StarCitizen/P4k-470/Data/Objects/buildingsets/example/designer_0.cgf")
    else {
        return;
    };

    match skin_to_glb(&data, None) {
        Ok(glb) => {
            println!("cgf → glb: {} bytes", glb.len());
        }
        Err(e) => {
            // CrCh format → UnsupportedFormat is expected and OK
            println!("cgf result: {e}");
        }
    }
}
