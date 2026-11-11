//! End-to-end synthetic decomposed exports: reusable DDS assets and weapon assembly.
use super::*;

#[test]
fn final_decomposed_export_classifies_dds_as_reusable_textures() {
    let fixture = archive();
    let data = starbreaker_datacore::dcb_builder::DcbBuilder::new().build();
    let db = Database::from_bytes(&data).unwrap();
    let mut sub = super::super::tests::sample_submaterial();
    sub.shader = "Illum".into();
    sub.layers.clear();
    sub.texture_slots.clear();
    sub.diffuse_tex = Some("Data/Test/synthetic_ddna.tif".into());
    sub.normal_tex = sub.diffuse_tex.clone();
    let mut mesh = super::super::tests::sample_mesh(vec![crate::types::SubMesh {
        material_name: Some(sub.name.clone()),
        material_id: 0,
        source_material_id: None,
        first_index: 0,
        num_indices: 3,
        first_vertex: 0,
        num_vertices: 3,
        node_parent_index: 0,
    }]);
    mesh.positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let material = MtlFile {
        materials: vec![sub],
        source_path: None,
        paint_override: None,
        material_set: Default::default(),
    };
    let input = DecomposedInput {
        entity_name: "synthetic".into(),
        geometry_path: "Data/Test/synthetic.cgf".into(),
        material_path: "Data/Test/synthetic.mtl".into(),
        assembly_kind: None,
        weapon_assembly: None,
        weapon_assembly_diagnostics: None,
        root_mesh: mesh,
        root_materials: Some(material),
        root_nmc: None,
        root_palette: None,
        available_palettes: vec![],
        root_bones: vec![],
        animation_sources: vec![],
        root_animation_controller: None,
        children: vec![],
        interiors: LoadedInteriors::default(),
        paint_variants: vec![],
        animations: vec![],
    };
    let opts = ExportOptions {
        kind: crate::ExportKind::Decomposed,
        texture_container: TextureContainer::Dds,
        texture_mip: 0,
        material_mode: MaterialMode::Textures,
        ..Default::default()
    };
    let export = write_decomposed_export(
        &db,
        &fixture.p4k,
        input,
        &opts,
        None,
        None,
        None,
        PngCache::new(),
        RoughnessCache::new(),
        HashMap::new(),
        &mut |_| panic!("no interiors in synthetic fixture"),
    )
    .unwrap();
    let textures: Vec<_> = export
        .files
        .iter()
        .filter(|f| f.relative_path.ends_with(".dds"))
        .collect();
    assert!(!textures.is_empty(), "fixture must write DDS assets");
    for file in textures {
        assert_eq!(
            file.kind,
            ExportedFileKind::TextureAsset,
            "{}",
            file.relative_path
        );
        assert!(file.kind.is_reusable_asset());
    }
}

#[path = "decomposed_assembly_fixture.rs"]
mod assembly_fixture;

#[test]
fn weapon_assembly_entry_skips_bundled_texture_loading_for_root_and_attachment() {
    // A dedicated worker also runs nested Rayon prewarm jobs, isolating the
    // thread-local observation from other concurrently running unit tests.
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(|| {
            use crate::pipeline::{BUNDLED_TEXTURE_LOADS, assemble_glb_with_loadout};
            let mesh = assembly_fixture::mesh_bytes();
            assert_eq!(crate::parse_skin(&mesh).unwrap().positions.len(), 3);
            let material = crate::test_xml::encode(
                r#"<Material Name="synthetic" Shader="Illum">
            <Textures><Texture Map="Diffuse" File="Data/Test/synthetic_ddna.tif"/>
            <Texture Map="Bumpmap" File="Data/Test/synthetic_ddna.tif"/></Textures>
            </Material>"#,
            );
            let fixture = fixture::archive_with_extra(vec![
                ("Data\\Test\\synthetic.cgf", mesh.clone()),
                ("Data\\Test\\synthetic.cgfm", mesh),
                ("Data\\Test\\synthetic.mtl", material),
            ]);
            let bytes = assembly_fixture::database_bytes();
            let db = Database::from_bytes(&bytes).unwrap();
            let root = db.records()[0];
            let child = db.records()[1];
            let mut tree = starbreaker_datacore::loadout::LoadoutTree {
                root: assembly_fixture::node(root, "synthetic_fps_weapon"),
            };
            tree.root
                .children
                .push(assembly_fixture::node(child, "synthetic_attachment"));
            let opts = ExportOptions {
                kind: crate::ExportKind::Decomposed,
                texture_container: TextureContainer::Dds,
                texture_mip: 0,
                lod_level: 0,
                include_interior: false,
                include_lights: false,
                ..Default::default()
            };
            let run = |opts: &ExportOptions| {
                BUNDLED_TEXTURE_LOADS.set(0);
                let package = assemble_glb_with_loadout(&db, &fixture.p4k, &root, &tree, opts)
                    .unwrap()
                    .decomposed
                    .unwrap();
                let count = BUNDLED_TEXTURE_LOADS.get();
                (package, count)
            };
            let (dds, loads) = run(&opts);
            assert!(dds.files.iter().any(|f| f.relative_path.ends_with(".dds")));
            assert_eq!(loads, 0, "DDS assembly must not run the bundled PNG loader");
            let png_opts = ExportOptions {
                texture_container: TextureContainer::Png,
                ..opts
            };
            let (png, loads) = run(&png_opts);
            assert!(
                loads >= 2,
                "PNG still loads root and attachment textures: {loads}"
            );
            assert!(png.files.iter().any(|f| f.relative_path.ends_with(".png")));
            assert!(!png.files.iter().any(|f| f.relative_path.ends_with(".dds")));
            let scene = png
                .files
                .iter()
                .find(|f| f.relative_path.ends_with("scene.json"))
                .unwrap();
            assert!(
                std::str::from_utf8(&scene.bytes)
                    .unwrap()
                    .contains("synthetic_attachment")
            );
        });
}
