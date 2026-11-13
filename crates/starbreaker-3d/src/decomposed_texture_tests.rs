//! Container defaults and package texture path contract, independent of game data.
use super::*;
use crate::TextureContainer;
#[test]
fn png_container_is_the_unchanged_default() {
    let opts = ExportOptions::default();
    assert_eq!(opts.texture_container, TextureContainer::Png);
    assert!(
        serde_json::to_value(opts)
            .unwrap()
            .get("texture_container")
            .is_none()
    );
    for (flavor, expected) in [
        (TextureFlavor::Generic, "Data/Test/synthetic_TEX2.png"),
        (TextureFlavor::Normal, "Data/Test/synthetic_TEX2.png"),
        (
            TextureFlavor::Roughness,
            "Data/Test/synthetic_roughness_TEX2.png",
        ),
    ] {
        assert_eq!(
            texture_relative_path_from_normalized(
                "Data/Test/synthetic.dds",
                flavor,
                2,
                TextureContainer::Png
            ),
            expected
        );
    }
}
#[test]
fn dds_paths_cover_generic_normal_and_smoothness() {
    for (flavor, expected) in [
        (TextureFlavor::Generic, "Data/Test/synthetic_TEX0.dds"),
        (TextureFlavor::Normal, "Data/Test/synthetic_TEX0.dds"),
        (
            TextureFlavor::Roughness,
            "Data/Test/synthetic_smoothness_TEX0.dds",
        ),
    ] {
        assert_eq!(
            texture_relative_path_from_normalized(
                "Data/Test/synthetic.tif",
                flavor,
                0,
                TextureContainer::Dds
            ),
            expected
        );
    }
}

#[path = "decomposed_texture_fixture.rs"]
mod fixture;
use fixture::archive;

#[test]
fn generic_and_normal_emit_dds_reuse_dds_and_preserve_png_fallback() {
    let fixture = archive();
    let p4k = &fixture.p4k;
    for flavor in [TextureFlavor::Generic, TextureFlavor::Normal] {
        let source = "Data/Test/synthetic_ddna.tif";
        let mut files = OutputFiles::new();
        let mut png = PngCache::new();
        let mut cache = HashMap::new();
        let path = export_texture_asset(
            &mut files,
            p4k,
            &mut png,
            &mut cache,
            source,
            flavor,
            0,
            None,
            TextureContainer::Dds,
        )
        .unwrap();
        assert!(path.ends_with("_TEX0.dds"));
        assert!(files.get(&path).unwrap().starts_with(b"DDS "));
        assert!(png.is_empty());
        let existing = HashSet::from([path.to_ascii_lowercase()]);
        files = OutputFiles::new();
        cache.clear();
        assert_eq!(
            export_texture_asset(
                &mut files,
                p4k,
                &mut png,
                &mut cache,
                source,
                flavor,
                0,
                Some(&existing),
                TextureContainer::Dds
            ),
            Some(path)
        );
        assert_eq!(files.len(), 0);
        // A format without an sRGB variant must use precisely the existing PNG loader.
        let source = "Data/Test/fallback_ddna.tif";
        cache.clear();
        let path = export_texture_asset(
            &mut files,
            p4k,
            &mut png,
            &mut cache,
            source,
            flavor,
            0,
            None,
            TextureContainer::Dds,
        )
        .unwrap();
        let expected = if flavor == TextureFlavor::Normal {
            crate::pipeline::load_normal_texture(p4k, source, 0)
        } else {
            crate::pipeline::load_diffuse_texture(p4k, source, 0)
        }
        .unwrap();
        assert!(path.ends_with(".png"));
        assert_eq!(files.get(&path).unwrap(), &expected);
        let existing = HashSet::from([path.to_ascii_lowercase()]);
        files = OutputFiles::new();
        cache.clear();
        assert_eq!(
            export_texture_asset(
                &mut files,
                p4k,
                &mut png,
                &mut cache,
                source,
                flavor,
                0,
                Some(&existing),
                TextureContainer::Dds
            ),
            Some(path)
        );
        assert_eq!(files.len(), 0);
        // Default container produces the same PNG bytes as the original loader.
        files = OutputFiles::new();
        cache.clear();
        png.clear();
        let path = export_texture_asset(
            &mut files,
            p4k,
            &mut png,
            &mut cache,
            source,
            flavor,
            0,
            None,
            ExportOptions::default().texture_container,
        )
        .unwrap();
        assert_eq!(files.get(&path).unwrap(), &expected);
    }
    assert!(load_texture_dds(p4k, "Data/Test/synthetic.tif", TextureFlavor::Normal, 0).is_none());
}
#[test]
fn smoothness_ref_and_prewarm_do_not_decode_or_encode_png() {
    let fixture = archive();
    let p4k = &fixture.p4k;
    let source = "Data/Test/synthetic_ddna.tif";
    let mut sub = super::tests::sample_submaterial();
    sub.diffuse_tex = Some(source.into());
    sub.normal_tex = Some(source.into());
    sub.layers.clear();
    sub.texture_slots.clear();
    let material = MtlFile {
        materials: vec![sub],
        source_path: None,
        paint_override: None,
        material_set: Default::default(),
    };
    let assets = vec![(
        material,
        "Data/Test/synthetic.mtl".into(),
        "Data/Test/synthetic.cgf".into(),
    )];
    let (png, _) = prewarm_decomposed_textures(p4k, &assets, 0, TextureContainer::Dds);
    assert!(png.is_empty());
    let rough = prewarm_decomposed_roughness(p4k, &assets, 0, TextureContainer::Dds);
    let loaded = rough.get(source).unwrap().as_ref().unwrap();
    assert!(loaded.png.is_empty());
    assert!(loaded.smoothness_mean.is_none());
    assert_eq!(loaded.container, TextureContainer::Dds);
    let mut files = OutputFiles::new();
    let mut tc = HashMap::new();
    let mut sc = HashMap::new();
    let (reference, status) = export_ddna_roughness_asset_with_status(
        &mut files,
        p4k,
        &mut tc,
        &mut sc,
        &rough,
        source,
        0,
        None,
        TextureContainer::Dds,
    );
    let reference = reference.unwrap();
    assert!(reference.export_path.ends_with("_smoothness_TEX0.dds"));
    assert_eq!(
        reference.packed_texture_format.as_deref(),
        Some("smoothness")
    );
    assert_eq!(reference.value_transform.as_deref(), Some("identity"));
    assert_eq!(reference.value_channel.as_deref(), Some("r"));
    assert!(reference.constant_channel_values.is_none());
    assert_eq!(
        reference.packed_channel_semantics,
        Some(BTreeMap::from([("r".into(), "smoothness".into())]))
    );
    assert_eq!(
        status.packed_texture_format,
        reference.packed_texture_format
    );
    assert_eq!(status.value_transform, reference.value_transform);
    assert!(status.smoothness_min.is_none());
    assert_eq!(
        files.get(&reference.export_path),
        Some(&loaded.package_bytes)
    );
    let existing = HashSet::from([reference.export_path.to_ascii_lowercase()]);
    files = OutputFiles::new();
    tc.clear();
    sc.clear();
    let reused = export_ddna_roughness_asset_with_status(
        &mut files,
        p4k,
        &mut tc,
        &mut sc,
        &rough,
        source,
        0,
        Some(&existing),
        TextureContainer::Dds,
    );
    assert_eq!(reused, (Some(reference), status));
    assert_eq!(files.len(), 0);
}

#[test]
fn png_gobo_and_dds_material_caches_do_not_alias() {
    let fixture = archive();
    let p4k = &fixture.p4k;
    let source = "Data/Test/synthetic_ddna.tif";
    let mut files = OutputFiles::new();
    let mut png = PngCache::new();
    let mut cache = HashMap::new();
    for container in [TextureContainer::Dds, TextureContainer::Png] {
        let path = export_texture_asset(
            &mut files,
            p4k,
            &mut png,
            &mut cache,
            source,
            TextureFlavor::Generic,
            0,
            None,
            container,
        )
        .unwrap();
        assert!(path.ends_with(if container == TextureContainer::Dds {
            ".dds"
        } else {
            ".png"
        }));
    }
}

#[test]
fn dds_normal_source_does_not_claim_an_alpha_channel() {
    let fixture = archive();
    let p4k = &fixture.p4k;
    let source = "Data/Test/synthetic_ddna.tif";
    let mut sub = super::tests::sample_submaterial();
    sub.layers.clear();
    sub.texture_slots.clear();
    sub.diffuse_tex = None;
    sub.normal_tex = Some(source.into());
    sub.texture_slots.push(crate::mtl::TextureSlotBinding {
        slot: "TexSlot2".into(),
        path: source.into(),
        is_virtual: false,
    });
    let extracted = extract_material_entry(
        &mut OutputFiles::new(),
        p4k,
        &mut PngCache::new(),
        &mut HashMap::new(),
        &mut HashMap::new(),
        &RoughnessCache::new(),
        &sub,
        0,
        None,
        &mut HashMap::new(),
        "Data/Test/synthetic.mtl",
        TextureContainer::Dds,
    );
    let normal = &extracted.direct_texture_exports[0];
    assert!(normal.export_path.ends_with(".dds"));
    assert_eq!(normal.alpha_channel, None);
    assert_eq!(normal.alpha_semantic, None);
    assert!(extracted.slot_exports[0].get("alpha_channel").is_none());
    assert!(extracted.slot_exports[0].get("alpha_semantic").is_none());
}

#[test]
fn palette_decal_uses_selected_container() {
    let fixture = archive();
    let p4k = &fixture.p4k;
    let palette = TintPalette {
        decal_texture: Some("Data/Test/synthetic_ddna.tif".into()),
        ..Default::default()
    };
    let mut records = BTreeMap::new();
    register_palette_with_id(&mut records, "synthetic".into(), &palette);
    let mut files = OutputFiles::new();
    finalize_palette_records(
        &mut records,
        &mut files,
        p4k,
        &mut PngCache::new(),
        &mut HashMap::new(),
        0,
        None,
        TextureContainer::Dds,
    );
    let path = records["synthetic"]
        .decal_texture_export_path
        .as_ref()
        .unwrap();
    assert!(path.ends_with(".dds"));
    assert!(files.get(path).unwrap().starts_with(b"DDS "));
}

#[test]
fn smoothness_size_mismatch_preserves_png_roughness_fallback() {
    let fixture = archive();
    let p4k = &fixture.p4k;
    let source = "Data/Test/fallback_ddna.tif";
    let expected =
        crate::pipeline::load_roughness_texture_result(p4k, source, 0, TextureContainer::Png)
            .unwrap();
    let mut files = OutputFiles::new();
    let mut tc = HashMap::new();
    let mut sc = HashMap::new();
    let (reference, status) = export_ddna_roughness_asset_with_status(
        &mut files,
        p4k,
        &mut tc,
        &mut sc,
        &RoughnessCache::new(),
        source,
        0,
        None,
        TextureContainer::Dds,
    );
    let reference = reference.unwrap();
    assert!(reference.export_path.ends_with("_roughness_TEX0.png"));
    assert_eq!(reference.value_transform.as_deref(), Some("sqrt_one_minus"));
    assert_eq!(
        reference.packed_texture_format.as_deref(),
        Some("roughness_grayscale")
    );
    assert_eq!(
        files.get(&reference.export_path),
        Some(&expected.package_bytes)
    );
    assert!(status.smoothness_mean.is_some());
    let existing = HashSet::from([reference.export_path.to_ascii_lowercase()]);
    files = OutputFiles::new();
    tc.clear();
    sc.clear();
    let reused = export_ddna_roughness_asset_with_status(
        &mut files,
        p4k,
        &mut tc,
        &mut sc,
        &RoughnessCache::new(),
        source,
        0,
        Some(&existing),
        TextureContainer::Dds,
    );
    assert_eq!(reused, (Some(reference), status));
    assert_eq!(files.len(), 0);
}

#[test]
fn unaligned_compressed_tops_use_identical_png_fallback() {
    let fixture = archive();
    for source in [
        "Data/Test/small1_ddna.tif",
        "Data/Test/small2_ddna.tif",
        "Data/Test/odd_ddna.tif",
    ] {
        for flavor in [TextureFlavor::Generic, TextureFlavor::Normal] {
            let mut files = OutputFiles::new();
            let mut png = PngCache::new();
            let mut cache = HashMap::new();
            let path = export_texture_asset(
                &mut files,
                &fixture.p4k,
                &mut png,
                &mut cache,
                source,
                flavor,
                0,
                None,
                TextureContainer::Dds,
            )
            .unwrap();
            let expected = if flavor == TextureFlavor::Normal {
                crate::pipeline::load_normal_texture(&fixture.p4k, source, 0)
            } else {
                crate::pipeline::load_diffuse_texture(&fixture.p4k, source, 0)
            }
            .unwrap();
            assert!(path.ends_with(".png"), "{source}");
            assert_eq!(files.get(&path), Some(&expected));
        }
    }
}

#[path = "decomposed_export_texture_tests.rs"]
mod export_tests;
