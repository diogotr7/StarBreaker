//! Unit tests for `starbreaker-blend`.
//!
//! These tests verify binary serialisation at the byte level and do NOT require
//! a running Blender instance.  Round-trip integration tests (loading in Blender)
//! live in Phase 67J.

use starbreaker_blend::*;

// ── Header / block serialisation ─────────────────────────────────────────────

#[test]
fn blend_magic_is_17_bytes() {
    assert_eq!(BLEND_MAGIC.len(), 17);
    assert_eq!(BLEND_MAGIC, b"BLENDER17-01v0501");
}

#[test]
fn write_block_header_is_32_bytes() {
    let mut out = Vec::new();
    write_block_header(&mut out, b"OB\0\0", SDNA_IDX_OBJECT, 0x1000, 1288, 1);
    assert_eq!(out.len(), 32);
}

#[test]
fn write_block_header_layout() {
    let mut out = Vec::new();
    write_block_header(&mut out, b"ME\0\0", 42, 0xdeadbeef_cafebabe_u64, 100, 3);
    assert_eq!(&out[0..4], b"ME\0\0");                                    // code
    assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()), 42);    // sdna_idx
    assert_eq!(u64::from_le_bytes(out[8..16].try_into().unwrap()), 0xdeadbeef_cafebabe_u64); // old_ptr
    assert_eq!(u32::from_le_bytes(out[16..20].try_into().unwrap()), 100); // data_len
    assert_eq!(u32::from_le_bytes(out[20..24].try_into().unwrap()), 0);   // zero
    assert_eq!(u32::from_le_bytes(out[24..28].try_into().unwrap()), 3);   // count
    assert_eq!(u32::from_le_bytes(out[28..32].try_into().unwrap()), 0);   // zero2
}

#[test]
fn write_block_appends_data_after_header() {
    let payload = b"ABCD";
    let mut out = Vec::new();
    write_block(&mut out, b"DATA", 0, 0x2000, 1, payload);
    assert_eq!(out.len(), 32 + 4);
    assert_eq!(&out[32..], b"ABCD");
}

#[test]
fn screen_blocks_use_forward_compatible_sn_filecode() {
    let screen = build_screen("TopScreen");
    let mut out = Vec::new();
    write_block(&mut out, b"SN\0\0", SDNA_IDX_SCREEN, 0x1000, 1, &screen);

    assert_eq!(&out[0..4], b"SN\0\0");
    assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()), SDNA_IDX_SCREEN);
    assert_eq!(&out[32 + 40..32 + 42], b"SR");
}

#[test]
fn startup_ui_prefix_uses_50mm_view3d_lenses() {
    let bytes = startup_ui_prefix_bytes();
    let lens_50 = 50.0f32.to_le_bytes();
    let lens_100 = 100.0f32.to_le_bytes();

    let count_50 = bytes
        .windows(lens_50.len())
        .filter(|window| *window == lens_50)
        .count();
    let count_100 = bytes
        .windows(lens_100.len())
        .filter(|window| *window == lens_100)
        .count();

    assert_eq!(count_50, 15);
    assert_eq!(count_100, 0);
}

// ── PtrAlloc ──────────────────────────────────────────────────────────────────

#[test]
fn ptr_alloc_increments_by_0x10() {
    let mut pa = PtrAlloc::new(0x1000);
    assert_eq!(pa.alloc(), 0x1000);
    assert_eq!(pa.alloc(), 0x1010);
    assert_eq!(pa.alloc(), 0x1020);
}

// ── Struct size sanity checks ─────────────────────────────────────────────────

#[test]
fn object_size_is_1288() {
    let ob = build_object("TestOb", 0, 0, 0, 0, 0);
    assert_eq!(ob.len(), OBJECT_SIZE);
    assert_eq!(OBJECT_SIZE, 1288);
}

#[test]
fn mesh_size_is_1960() {
    let me = build_mesh("TestMesh", 4, 0, 1, 4, 0, 0, 0, 0, 0, 0, 0, 5);
    assert_eq!(me.len(), MESH_SIZE);
    assert_eq!(MESH_SIZE, 1960);
}

#[test]
fn lamp_size_is_568() {
    let la = build_lamp("TestLamp", 0, [1.0, 1.0, 1.0], 10.0, 0.1, 1.0, 0.5, 0.1, 6500.0, false);
    assert_eq!(la.len(), LAMP_SIZE);
    assert_eq!(LAMP_SIZE, 568);
}

#[test]
fn material_size_is_584() {
    let ma = build_material("TestMaterial");
    assert_eq!(ma.len(), MATERIAL_SIZE);
    assert_eq!(MATERIAL_SIZE, 584);
}

#[test]
fn bnode_tree_size_is_736() {
    let ntree = build_empty_shader_node_tree(0x2000);
    assert_eq!(ntree.len(), BNODE_TREE_SIZE);
    assert_eq!(BNODE_TREE_SIZE, 736);
}

#[test]
fn empty_object_size_is_1288() {
    let ob = build_empty_object("Empty", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 0);
    assert_eq!(ob.len(), OBJECT_SIZE);
}

#[test]
fn empty_object_sets_id_properties_pointer() {
    let ob = build_empty_object_with_properties(
        "Empty",
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        0,
        0x1234_5678_u64,
    );
    assert_eq!(u64::from_le_bytes(ob[344..352].try_into().unwrap()), 0x1234_5678_u64);
}

#[test]
fn lamp_object_sets_id_properties_pointer() {
    let ob = build_lamp_object_with_properties(
        "Light",
        0x2000,
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        0,
        0x1234_5678_u64,
    );
    assert_eq!(u64::from_le_bytes(ob[344..352].try_into().unwrap()), 0x1234_5678_u64);
}

#[test]
fn idproperty_group_records_child_count_and_subtypes() {
    let (root, children, _) = build_idprop_tree(
        0x1000,
        &[0x2000, 0x3000],
        &[],
        &[
            ("flag".to_string(), IdPropValue::Int(1)),
            ("ratio".to_string(), IdPropValue::Double(2.0)),
        ],
    );

    assert_eq!(root[16], 6);
    assert_eq!(root[17], 6);
    assert_eq!(i32::from_le_bytes(root[128..132].try_into().unwrap()), 2);
    assert_eq!(i32::from_le_bytes(root[132..136].try_into().unwrap()), 2);
    assert_eq!(children[0].1[16], 1);
    assert_eq!(children[0].1[17], 1);
    assert_eq!(children[1].1[16], 8);
    assert_eq!(children[1].1[17], 8);
}

#[test]
fn view_layer_sets_active_collection_pointer() {
    let view_layer = build_view_layer("ViewLayer", 0x2000, 0x3000);

    assert_eq!(u64::from_le_bytes(view_layer[88..96].try_into().unwrap()), 0x2000);
    assert_eq!(u64::from_le_bytes(view_layer[120..128].try_into().unwrap()), 0x3000);
    assert_eq!(u64::from_le_bytes(view_layer[136..144].try_into().unwrap()), 0x3000);
    assert_eq!(i32::from_le_bytes(view_layer[144..148].try_into().unwrap()), 0x7fff);
    assert_eq!(i32::from_le_bytes(view_layer[148..152].try_into().unwrap()), 1);
    assert_eq!(f32::from_le_bytes(view_layer[152..156].try_into().unwrap()), 0.5);
    assert_eq!(i16::from_le_bytes(view_layer[156..158].try_into().unwrap()), 8);
    assert_eq!(i16::from_le_bytes(view_layer[158..160].try_into().unwrap()), 6);
}

#[test]
fn scene_has_nonzero_blender_runtime_safe_defaults() {
    let scene = build_scene("Scene", 0x2000, 0x3000, 0x4000);

    assert_eq!(i32::from_le_bytes(scene[1032..1036].try_into().unwrap()), 1);
    assert_eq!(i32::from_le_bytes(scene[1036..1040].try_into().unwrap()), 1);
    assert_eq!(i32::from_le_bytes(scene[1040..1044].try_into().unwrap()), 250);
    assert_eq!(scene[609], 17);
    assert_eq!(scene[610], 2);
    assert_eq!(scene[611], 32);
    assert_eq!(scene[613], 90);
    assert_eq!(scene[614], 15);
    assert_eq!(f32::from_le_bytes(scene[1068..1072].try_into().unwrap()), 1.0);
    assert_eq!(i32::from_le_bytes(scene[1072..1076].try_into().unwrap()), 1);
    assert_eq!(u16::from_le_bytes(scene[1116..1118].try_into().unwrap()), 24);
    assert_eq!(f32::from_le_bytes(scene[1120..1124].try_into().unwrap()), 0.0);
    assert_eq!(f32::from_le_bytes(scene[1124..1128].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(scene[1128..1132].try_into().unwrap()), 0.0);
    assert_eq!(f32::from_le_bytes(scene[1132..1136].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(scene[1172..1176].try_into().unwrap()), 1.0);
    assert_eq!(&scene[3060..3077], b"BLENDER_WORKBENCH");
    assert_eq!(scene[3028], 3);
    assert_eq!(i32::from_le_bytes(scene[3052..3056].try_into().unwrap()), 1);
    assert_eq!(i32::from_le_bytes(scene[5008..5012].try_into().unwrap()), 48000);
    assert_eq!(i32::from_le_bytes(scene[5072..5076].try_into().unwrap()), 1);
    assert_eq!(i32::from_le_bytes(scene[5076..5080].try_into().unwrap()), -1);
    assert_eq!(f32::from_le_bytes(scene[5176..5180].try_into().unwrap()), 1.0);
    assert_eq!(scene[5180], 1);
    assert_eq!(&scene[5320..5324], b"None");
    assert_eq!(&scene[5384..5387], b"AgX");
    assert_eq!(f32::from_le_bytes(scene[5448..5452].try_into().unwrap()), 0.0);
    assert_eq!(f32::from_le_bytes(scene[5452..5456].try_into().unwrap()), 1.0);
    assert_eq!(&scene[5480..5484], b"sRGB");
    assert_eq!(&scene[5552..5556], b"sRGB");
    assert_eq!(u64::from_le_bytes(scene[568..576].try_into().unwrap()), 0x4000);
    assert_eq!(i32::from_le_bytes(scene[5664..5668].try_into().unwrap()), 1);
    assert_eq!(i32::from_le_bytes(scene[5668..5672].try_into().unwrap()), 250);
}

#[test]
fn scene_initializes_cycles_safe_motion_blur_curve() {
    let scene = build_scene_with_motion_blur_curve("Scene", 0x2000, 0x3000, 0x4000, 0x5000);
    let points = build_motion_blur_shutter_curve_points();

    assert_eq!(i32::from_le_bytes(scene[4552..4556].try_into().unwrap()), 17);
    assert_eq!(f32::from_le_bytes(scene[4568..4572].try_into().unwrap()), 0.0);
    assert_eq!(f32::from_le_bytes(scene[4572..4576].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(scene[4584..4588].try_into().unwrap()), 0.0);
    assert_eq!(f32::from_le_bytes(scene[4588..4592].try_into().unwrap()), 1.0);
    assert_eq!(i16::from_le_bytes(scene[4600..4602].try_into().unwrap()), 3);
    assert_eq!(i16::from_le_bytes(scene[4602..4604].try_into().unwrap()), 1);
    assert_eq!(f32::from_le_bytes(scene[4604..4608].try_into().unwrap()), 256.0);
    assert_eq!(f32::from_le_bytes(scene[4612..4616].try_into().unwrap()), 1.0);
    assert_eq!(u64::from_le_bytes(scene[4632..4640].try_into().unwrap()), 0x5000);

    assert_eq!(points.len(), CURVE_MAP_POINT_SIZE * 3);
    let values = (0..3)
        .map(|idx| {
            let off = idx * CURVE_MAP_POINT_SIZE;
            (
                f32::from_le_bytes(points[off..off + 4].try_into().unwrap()),
                f32::from_le_bytes(points[off + 4..off + 8].try_into().unwrap()),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(values, vec![(0.0, 1.0), (0.5, 1.0), (1.0, 1.0)]);
}

#[test]
fn cycles_render_settings_idprops_match_requested_defaults() {
    let child_ptrs = [0x3000, 0x3010, 0x3020, 0x3030, 0x3040, 0x3050];
    let (root, cycles_group, children) =
        build_cycles_render_settings_system_properties(0x1000, 0x2000, &child_ptrs);
    let scene = build_scene_with_motion_blur_curve_and_properties(
        "Scene",
        0,
        0,
        0,
        0,
        0x1000,
        0x6000,
        "CYCLES",
    );

    assert_eq!(u64::from_le_bytes(scene[352..360].try_into().unwrap()), 0x1000);
    assert_eq!(u64::from_le_bytes(scene[424..432].try_into().unwrap()), 0x6000);
    assert_eq!(&scene[3060..3066], b"CYCLES");
    assert_eq!(root[16], IDP_GROUP);
    assert_eq!(u64::from_le_bytes(root[96..104].try_into().unwrap()), 0x2000);
    assert_eq!(&cycles_group[20..26], b"cycles");
    assert_eq!(i32::from_le_bytes(cycles_group[128..132].try_into().unwrap()), 6);

    let values = children
        .iter()
        .map(|(_, block)| {
            (
                String::from_utf8_lossy(&block[20..84])
                    .trim_end_matches('\0')
                    .to_string(),
                i32::from_le_bytes(block[120..124].try_into().unwrap()),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        vec![
            ("sampling_pattern".to_string(), 1),
            ("device".to_string(), 1),
            ("preview_samples".to_string(), 64),
            ("use_preview_denoising".to_string(), 1),
            ("samples".to_string(), 512),
            ("use_denoising".to_string(), 1),
        ]
    );
}

#[test]
fn world_defaults_are_render_visible() {
    let world = build_world("World");

    assert_eq!(world.len(), WORLD_SIZE);
    assert_eq!(&world[40..47], b"WOWorld");
    assert_eq!(f32::from_le_bytes(world[424..428].try_into().unwrap()), 0.050876088);
    assert_eq!(f32::from_le_bytes(world[428..432].try_into().unwrap()), 0.050876088);
    assert_eq!(f32::from_le_bytes(world[432..436].try_into().unwrap()), 0.050876088);
    assert_eq!(i16::from_le_bytes(world[450..452].try_into().unwrap()), 1 << 4);
    assert_eq!(i32::from_le_bytes(world[476..480].try_into().unwrap()), 10);
    let world_with_tree = build_world_with_node_tree("World", 0x7000);
    assert_eq!(u64::from_le_bytes(world_with_tree[512..520].try_into().unwrap()), 0x7000);
}

#[test]
fn world_sky_shader_uses_requested_intensity_defaults() {
    let mut bytes = Vec::new();
    let mut ptrs = PtrAlloc::new(0x8000);
    write_world_with_sky_shader(&mut bytes, "World", 0x7000, 0x7010, &mut ptrs);

    let mut found_background_strength = false;
    let mut found_sky_sun_intensity = false;
    let mut offset = 0usize;
    while offset + 32 <= bytes.len() {
        let sdna = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
        let len = u32::from_le_bytes(bytes[offset + 16..offset + 20].try_into().unwrap()) as usize;
        let data_start = offset + 32;
        let data_end = data_start + len;
        if sdna == SDNA_IDX_BNSV_FLOAT && len >= 16 {
            let value = f32::from_le_bytes(bytes[data_start + 4..data_start + 8].try_into().unwrap());
            if (value - 0.5).abs() < 1e-6 {
                found_background_strength = true;
            }
        }
        if sdna == SDNA_IDX_NODE_TEX_SKY && len > 992 {
            let value = f32::from_le_bytes(bytes[data_start + 988..data_start + 992].try_into().unwrap());
            if (value - 0.1).abs() < 1e-6 {
                found_sky_sun_intensity = true;
            }
        }
        offset = data_end;
    }

    assert!(found_background_strength, "Background Strength should default to 0.5");
    assert!(found_sky_sun_intensity, "Sky Texture sun_intensity should default to 0.1");
}

#[test]
fn material_without_nodes_does_not_reference_empty_shader_tree() {
    let material = build_material_with_node_tree_and_properties("Material", 0, 0);

    assert_eq!(material[472], 0);
    assert_eq!(u64::from_le_bytes(material[480..488].try_into().unwrap()), 0);
}

#[test]
fn scene_display_shading_uses_valid_solid_enum() {
    let scene = build_scene("Scene", 0x2000, 0x3000, 0x4000);

    // Scene.display.shading.type and prev_type are View3DShading.type bytes.
    // Blender 5.1 valid object draw mode values are 2, 3, 4, and 6; zero is invalid.
    assert_eq!(scene[5712], 3);
    assert_eq!(scene[5713], 3);
}

#[test]
fn scene_defaults_do_not_corrupt_listbase_pointers() {
    let scene = build_scene("Scene", 0x2000, 0x3000, 0x4000);

    assert_eq!(u64::from_le_bytes(scene[5040..5048].try_into().unwrap()), 0);
    assert_eq!(u64::from_le_bytes(scene[5048..5056].try_into().unwrap()), 0);
    assert_eq!(u64::from_le_bytes(scene[5160..5168].try_into().unwrap()), 0);
    assert_eq!(u64::from_le_bytes(scene[5168..5176].try_into().unwrap()), 0);
}

#[test]
fn scene_links_view_layer_and_master_collection() {
    let scene = build_scene("Scene", 0x2000, 0x3000, 0x4000);

    assert_eq!(u64::from_le_bytes(scene[5632..5640].try_into().unwrap()), 0x2000);
    assert_eq!(u64::from_le_bytes(scene[5640..5648].try_into().unwrap()), 0x2000);
    assert_eq!(u64::from_le_bytes(scene[5648..5656].try_into().unwrap()), 0x3000);
}

#[test]
fn tool_settings_block_has_blender_51_size() {
    let tool_settings = build_tool_settings();

    assert_eq!(tool_settings.len(), TOOL_SETTINGS_SIZE);
    assert_eq!(TOOL_SETTINGS_SIZE, 1256);
}

#[test]
fn idproperty_size_is_144() {
    let b = build_idproperty(IDP_INT, "x", 0, 0, 0, 0, 0, 42, 0.0, 0, 0);
    assert_eq!(b.len(), IDPROPERTY_SIZE);
    assert_eq!(IDPROPERTY_SIZE, 144);
}

// ── Object field layout ───────────────────────────────────────────────────────

#[test]
fn object_type_at_416() {
    // OB_MESH = 1
    let ob = build_object("TestOb", 0x2000, 0, 0, 0, 0);
    assert_eq!(i16::from_le_bytes(ob[416..418].try_into().unwrap()), 1);
}

#[test]
fn object_mesh_ptr_at_552() {
    let ob = build_object("TestOb", 0xdead_0000_u64, 0, 0, 0, 0);
    assert_eq!(u64::from_le_bytes(ob[552..560].try_into().unwrap()), 0xdead_0000_u64);
}

#[test]
fn object_id_properties_ptr_at_344() {
    let ob = build_object("TestOb", 0, 0, 0, 0, 0xbeef_1234_u64);
    assert_eq!(u64::from_le_bytes(ob[344..352].try_into().unwrap()), 0xbeef_1234_u64);
}

#[test]
fn object_mat_ptr_at_712() {
    let ob = build_object("TestOb", 0, 0xaaaa_u64, 0xbbbb_u64, 2, 0);
    assert_eq!(u64::from_le_bytes(ob[712..720].try_into().unwrap()), 0xaaaa_u64);
    assert_eq!(u64::from_le_bytes(ob[720..728].try_into().unwrap()), 0xbbbb_u64);
    assert_eq!(i32::from_le_bytes(ob[728..732].try_into().unwrap()), 2);
}

#[test]
fn material_name_uses_ma_id_prefix() {
    let ma = build_material("Hull");
    let name = ma[40..80].split(|&c| c == 0).next().unwrap();
    assert_eq!(name, b"MAHull");
}

#[test]
fn material_uses_visible_opaque_defaults() {
    let ma = build_material("Hull");

    assert_eq!(f32::from_le_bytes(ma[420..424].try_into().unwrap()), 0.8);
    assert_eq!(f32::from_le_bytes(ma[424..428].try_into().unwrap()), 0.8);
    assert_eq!(f32::from_le_bytes(ma[428..432].try_into().unwrap()), 0.8);
    assert_eq!(f32::from_le_bytes(ma[432..436].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(ma[464..468].try_into().unwrap()), 0.4);
    assert_eq!(ma[472], 0);
    assert_eq!(ma[473], 1);
    assert_eq!(f32::from_le_bytes(ma[524..528].try_into().unwrap()), 0.5);
    assert_eq!(ma[532], 0); // blend_method = MA_BM_SOLID
    assert_eq!(ma[533], 1); // blend_shadow = MA_BS_SOLID
    assert_eq!(ma[534], 1 << 6); // blend_flag = MA_BL_TRANSPARENT_SHADOW
}

#[test]
fn material_with_node_tree_sets_nodetree_pointer() {
    let ma = build_material_with_node_tree("Hull", 0xfeed_cafe_u64);

    assert_eq!(ma[472], 1); // use_nodes compatibility byte
    assert_eq!(u64::from_le_bytes(ma[480..488].try_into().unwrap()), 0xfeed_cafe_u64);
}

#[test]
fn material_with_node_tree_sets_id_properties_pointer() {
    let ma = build_material_with_node_tree_and_properties("Hull", 0xfeed_cafe_u64, 0x1234_5678_u64);

    assert_eq!(u64::from_le_bytes(ma[344..352].try_into().unwrap()), 0x1234_5678_u64);
    assert_eq!(u64::from_le_bytes(ma[480..488].try_into().unwrap()), 0xfeed_cafe_u64);
}

#[test]
fn empty_shader_node_tree_has_embedded_shader_identity() {
    let ntree = build_empty_shader_node_tree(0xaaaa_bbbb_u64);

    let name = ntree[40..100].split(|&c| c == 0).next().unwrap();
    assert_eq!(name, b"NTShader Nodetree");
    assert_eq!(i16::from_le_bytes(ntree[298..300].try_into().unwrap()), 0x0400);
    assert_eq!(u64::from_le_bytes(ntree[416..424].try_into().unwrap()), 0xaaaa_bbbb_u64);
    let idname = ntree[432..496].split(|&c| c == 0).next().unwrap();
    assert_eq!(idname, b"ShaderNodeTree");
    assert_eq!(i32::from_le_bytes(ntree[552..556].try_into().unwrap()), 0); // NTREE_SHADER
    assert_eq!(u64::from_le_bytes(ntree[520..528].try_into().unwrap()), 0); // nodes.first
    assert_eq!(u64::from_le_bytes(ntree[528..536].try_into().unwrap()), 0); // nodes.last
    assert_eq!(u64::from_le_bytes(ntree[536..544].try_into().unwrap()), 0); // links.first
    assert_eq!(u64::from_le_bytes(ntree[544..552].try_into().unwrap()), 0); // links.last
}

#[test]
fn material_node_tree_blocks_use_ma_then_embedded_data_order() {
    let ma_ptr = 0x1000_u64;
    let ntree_ptr = 0x1010_u64;
    let ma = build_material_with_node_tree("Hull", ntree_ptr);
    let ntree = build_empty_shader_node_tree(ma_ptr);
    let mut out = Vec::new();

    write_block(&mut out, b"MA\0\0", SDNA_IDX_MATERIAL, ma_ptr, 1, &ma);
    write_block(&mut out, b"DATA", SDNA_IDX_BNODE_TREE, ntree_ptr, 1, &ntree);

    assert_eq!(&out[0..4], b"MA\0\0");
    assert_eq!(u64::from_le_bytes(out[8..16].try_into().unwrap()), ma_ptr);
    assert_eq!(&out[32 + MATERIAL_SIZE..32 + MATERIAL_SIZE + 4], b"DATA");
    assert_eq!(
        u32::from_le_bytes(out[32 + MATERIAL_SIZE + 4..32 + MATERIAL_SIZE + 8].try_into().unwrap()),
        SDNA_IDX_BNODE_TREE
    );
    assert_eq!(
        u64::from_le_bytes(out[32 + MATERIAL_SIZE + 8..32 + MATERIAL_SIZE + 16].try_into().unwrap()),
        ntree_ptr
    );
}

#[test]
fn material_pointer_array_serializes_old_pointers() {
    let array = build_mat_ptr_array_from_ptrs(&[0x1111, 0x2222]);
    assert_eq!(array.len(), 16);
    assert_eq!(u64::from_le_bytes(array[0..8].try_into().unwrap()), 0x1111);
    assert_eq!(u64::from_le_bytes(array[8..16].try_into().unwrap()), 0x2222);
}

#[test]
fn object_transform_defaults_are_visible_identity() {
    let ob = build_object("TestOb", 0x2000, 0, 0, 0, 0);

    assert_eq!(f32::from_le_bytes(ob[760..764].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(ob[764..768].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(ob[768..772].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(ob[820..824].try_into().unwrap()), 1.0);
    assert_eq!(i16::from_le_bytes(ob[1040..1042].try_into().unwrap()), 1);
    assert_eq!(i16::from_le_bytes(ob[1042..1044].try_into().unwrap()), -1);
    assert_eq!(ob[1046], 5);
    assert_eq!(f32::from_le_bytes(ob[1048..1052].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(ob[1064..1068].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(ob[1068..1072].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(ob[1072..1076].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(ob[1076..1080].try_into().unwrap()), 1.0);
}

fn assert_identity_matrix(bytes: &[u8], offset: usize, label: &str) {
    let expected: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    for (i, &v) in expected.iter().enumerate() {
        let got = f32::from_le_bytes(bytes[offset + i * 4..offset + i * 4 + 4].try_into().unwrap());
        assert_eq!(got, v, "{label}[{i}] mismatch");
    }
}

#[test]
fn object_parent_ptr_at_496() {
    let ob = build_empty_object("Child", [0.0; 3], [1.0, 0.0, 0.0, 0.0], [1.0; 3], 0xcafe_u64);
    assert_eq!(u64::from_le_bytes(ob[496..504].try_into().unwrap()), 0xcafe_u64);
}

#[test]
fn empty_object_parentinv_is_identity_when_parented() {
    let ob = build_empty_object("Child", [0.0; 3], [1.0, 0.0, 0.0, 0.0], [1.0; 3], 0x1000_u64);
    assert_identity_matrix(&ob, 884, "parentinv");
}

#[test]
fn empty_object_inverse_matrices_are_identity_when_unparented() {
    let ob = build_empty_object("Root", [0.0; 3], [1.0, 0.0, 0.0, 0.0], [1.0; 3], 0);
    assert_identity_matrix(&ob, 884, "parentinv");
    assert_identity_matrix(&ob, 948, "constinv");
}

#[test]
fn linked_instance_transform_defaults_preserve_nonzero_matrix() {
    let ob = build_linked_instance_object(
        "Linked",
        0x2000,
        [0.0; 3],
        [1.0, 0.0, 0.0, 0.0],
        [1.0; 3],
        0,
    );

    assert_eq!(f32::from_le_bytes(ob[760..764].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(ob[784..788].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(ob[820..824].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(ob[836..840].try_into().unwrap()), 1.0);
    assert_eq!(i16::from_le_bytes(ob[1040..1042].try_into().unwrap()), 0);
    assert_eq!(ob[1046], 5);
    assert_eq!(f32::from_le_bytes(ob[1076..1080].try_into().unwrap()), 1.0);
}

#[test]
fn lamp_object_type_at_416() {
    let ob = build_lamp_object("L", 0x3000, [0.0; 3], [1.0, 0.0, 0.0, 0.0], [1.0; 3], 0);
    assert_eq!(i16::from_le_bytes(ob[416..418].try_into().unwrap()), 10);
}

// ── Mesh field layout ─────────────────────────────────────────────────────────

#[test]
fn mesh_totvert_at_432() {
    let me = build_mesh("M", 7, 5, 2, 8, 0, 0, 0, 0, 0, 0, 0, 0);
    assert_eq!(u32::from_le_bytes(me[432..436].try_into().unwrap()), 7);
    assert_eq!(u32::from_le_bytes(me[436..440].try_into().unwrap()), 5);
}

#[test]
fn mesh_attributes_ptr_at_456() {
    let me = build_mesh("M", 4, 0, 1, 4, 0, 0xcccc_u64, 0, 0, 0, 0, 0, 2);
    assert_eq!(u64::from_le_bytes(me[456..464].try_into().unwrap()), 0xcccc_u64);
}

#[test]
fn triangle_edge_topology_builds_edge_and_corner_edge_layers() {
    let (edges, corner_edges) = triangle_edge_topology(&[0, 1, 2, 2, 1, 3]);

    assert_eq!(edges, vec![[0, 1], [1, 2], [0, 2], [1, 3], [2, 3]]);
    assert_eq!(corner_edges, vec![0, 1, 2, 1, 3, 4]);
    assert_eq!(ints2_data(&edges).len(), edges.len() * 8);
}

#[test]
fn triangle_edge_topology_ignores_incomplete_trailing_indices() {
    let (edges, corner_edges) = triangle_edge_topology(&[0, 1, 2, 99]);

    assert_eq!(edges, vec![[0, 1], [1, 2], [0, 2]]);
    assert_eq!(corner_edges, vec![0, 1, 2]);
}

// ── Lamp field layout ─────────────────────────────────────────────────────────

#[test]
fn lamp_type_point_at_416() {
    let la = build_lamp("L", 0, [1.0, 0.5, 0.0], 100.0, 0.25, 1.0, 0.0, 0.0, 6500.0, false);
    assert_eq!(i16::from_le_bytes(la[416..418].try_into().unwrap()), 0);
}

#[test]
fn lamp_energy_at_440() {
    let la = build_lamp("L", 0, [1.0, 1.0, 1.0], 77.5, 0.0, 1.0, 0.0, 0.0, 6500.0, false);
    assert_eq!(f32::from_le_bytes(la[440..444].try_into().unwrap()), 77.5_f32);
}

#[test]
fn lamp_cutoff_distance_at_528() {
    let la = build_lamp("L", 0, [1.0, 1.0, 1.0], 77.5, 0.02, 1.23, 0.0, 0.0, 6500.0, false);
    assert_eq!(f32::from_le_bytes(la[528..532].try_into().unwrap()), 1.23_f32);
}

#[test]
fn lamp_color_fields() {
    let la = build_lamp("L", 0, [0.1, 0.2, 0.3], 1.0, 0.0, 1.0, 0.0, 0.0, 6500.0, false);
    assert!((f32::from_le_bytes(la[424..428].try_into().unwrap()) - 0.1_f32).abs() < 1e-6);
    assert!((f32::from_le_bytes(la[428..432].try_into().unwrap()) - 0.2_f32).abs() < 1e-6);
    assert!((f32::from_le_bytes(la[432..436].try_into().unwrap()) - 0.3_f32).abs() < 1e-6);
}

#[test]
fn lamp_with_node_tree_sets_nodetree_and_use_nodes() {
    let la = build_lamp_with_node_tree(
        "L",
        0,
        [1.0, 1.0, 1.0],
        10.0,
        0.02,
        1.0,
        0.0,
        0.0,
        5000.0,
        false,
        0x1234,
    );
    assert_eq!(i16::from_le_bytes(la[486..488].try_into().unwrap()), 1);
    assert_eq!(u64::from_le_bytes(la[552..560].try_into().unwrap()), 0x1234);
}

#[test]
fn lamp_object_can_hide_camera_visibility() {
    let ob = build_lamp_object_with_properties_and_visibility(
        "L",
        0x1000,
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        0,
        0,
        true,
    );
    assert_eq!(i16::from_le_bytes(ob[1082..1084].try_into().unwrap()), 0x0008);
}

#[test]
fn light_gobo_node_tree_writes_embedded_data_before_image_id() {
    let mut out = Vec::new();
    let mut ptrs = PtrAlloc::new(0x2000);
    write_light_gobo_node_tree(
        &mut out,
        0x1000,
        0x1010,
        0x1020,
        "spot_075.dds",
        "//../../Data/Textures/lights/generic/spot_075.dds",
        true,
        &mut ptrs,
    );
    assert_eq!(&out[0..4], b"DATA");
    assert!(out.windows("ShaderNodeTexImage".len()).any(|window| window == b"ShaderNodeTexImage"));
    assert!(out.windows("IMspot_075.dds".len()).any(|window| window == b"IMspot_075.dds"));
    assert!(out
        .windows("//../../Data/Textures/lights/generic/spot_075.dds".len())
        .any(|window| window == b"//../../Data/Textures/lights/generic/spot_075.dds"));
}

#[test]
fn light_gobo_node_tree_image_tex_node_type_is_143() {
    // SH_NODE_TEX_IMAGE must be 143, not 108.  Written as little-endian i16 at
    // BNode+0x0C0.  Locate "ShaderNodeTexImage" in the output and verify that
    // the two bytes at relative offset -(0x78 - 0xC0) = +0x48 contain 143/0x8F.
    let mut out = Vec::new();
    let mut ptrs = PtrAlloc::new(0x2000);
    write_light_gobo_node_tree(
        &mut out,
        0x1000,
        0x1010,
        0x1020,
        "spot.dds",
        "//spot.dds",
        true,
        &mut ptrs,
    );
    let idname = b"ShaderNodeTexImage";
    let pos = out
        .windows(idname.len())
        .position(|w| w == idname)
        .expect("ShaderNodeTexImage not found in output");
    // idname is at BNode+0x78; node_type is at BNode+0xC0 → offset delta = 0x48
    let type_offset = pos + 0x48;
    let node_type = i16::from_le_bytes(out[type_offset..type_offset + 2].try_into().unwrap());
    assert_eq!(node_type, 143, "SH_NODE_TEX_IMAGE should be 143, got {node_type}");
}

#[test]
fn light_gobo_node_tree_output_node_has_do_output_flag() {
    // NODE_DO_OUTPUT = 1 << 6 = 0x40.  The Light Output BNode flags field is at
    // BNode+0x74; idname "ShaderNodeOutputLight" is at BNode+0x78 (delta = +4 bytes).
    let mut out = Vec::new();
    let mut ptrs = PtrAlloc::new(0x2000);
    write_light_gobo_node_tree(
        &mut out,
        0x1000,
        0x1010,
        0x1020,
        "spot.dds",
        "//spot.dds",
        true,
        &mut ptrs,
    );
    let idname = b"ShaderNodeOutputLight";
    let pos = out
        .windows(idname.len())
        .position(|w| w == idname)
        .expect("ShaderNodeOutputLight not found in output");
    // flags is 4 bytes before idname (BNode+0x74 vs BNode+0x78)
    let flags_offset = pos - 4;
    let flags = u32::from_le_bytes(out[flags_offset..flags_offset + 4].try_into().unwrap());
    assert!(flags & 0x40 != 0, "NODE_DO_OUTPUT (0x40) must be set on Light Output node, flags={flags:#010x}");
}

#[test]
fn light_gobo_node_tree_has_no_texcoord_or_mapping_nodes() {
    // Reference gobo.blend uses only 3 nodes: Image Texture → Emission → Light Output.
    // No Texture Coordinate or Mapping nodes should be present.
    let mut out = Vec::new();
    let mut ptrs = PtrAlloc::new(0x2000);
    write_light_gobo_node_tree(&mut out, 0x1000, 0x1010, 0x1020, "spot.dds", "//spot.dds", true, &mut ptrs);
    assert!(
        !out.windows(b"ShaderNodeTexCoord".len()).any(|w| w == b"ShaderNodeTexCoord"),
        "ShaderNodeTexCoord must NOT be present in gobo node tree"
    );
    assert!(
        !out.windows(b"ShaderNodeMapping".len()).any(|w| w == b"ShaderNodeMapping"),
        "ShaderNodeMapping must NOT be present in gobo node tree"
    );
}

#[test]
fn light_gobo_node_tree_has_two_links() {
    // Simple 3-node chain has 2 links: ImageTex.Color→Emission.Color, Emission→LightOutput
    let mut out = Vec::new();
    let mut ptrs = PtrAlloc::new(0x2000);
    write_light_gobo_node_tree(&mut out, 0x1000, 0x1010, 0x1020, "spot.dds", "//spot.dds", true, &mut ptrs);
    // Block header layout: [code:4][sdna_idx:4][old_ptr:8][data_len:4][pad:4][count:4][pad:4] = 32 bytes
    // SDNA index is at bytes 4-7 as u32 le; SDNA_IDX_BNODE_LINK = 470
    const HDR: usize = 32;
    let link_count = out
        .windows(HDR)
        .filter(|h| {
            &h[0..4] == b"DATA"
            && u32::from_le_bytes([h[4], h[5], h[6], h[7]]) == 470
        })
        .count();
    assert_eq!(link_count, 2, "Expected 2 node links (SDNA 470), got {link_count}");
}

#[test]
fn light_gobo_node_tree_image_tex_texmapping_scale_is_one() {
    // TexMapping.size (exposed as Python `.scale`) must be (1,1,1).
    // NodeTexBase.tex_mapping (TexMapping) is at offset 0 of NodeTexImage storage.
    // TexMapping layout: loc[3] (0..12) + rot[3] (12..24) + size[3] (24..36).
    // Zeroed storage → size=(0,0,0) → all UV lookups collapse to edge pixel → black gobo.
    let mut out = Vec::new();
    let mut ptrs = PtrAlloc::new(0x2000);
    write_light_gobo_node_tree(&mut out, 0x1000, 0x1010, 0x1020, "spot.png", "//spot.png", true, &mut ptrs);

    const HDR: usize = 32;
    let storage_pos = out
        .windows(HDR)
        .position(|h| {
            &h[0..4] == b"DATA"
            && u32::from_le_bytes([h[4], h[5], h[6], h[7]]) == 531
        })
        .expect("NodeTexImage DATA block (SDNA 531) not found");
    let data_start = storage_pos + HDR;

    let sx = f32::from_le_bytes(out[data_start + 24..data_start + 28].try_into().unwrap());
    let sy = f32::from_le_bytes(out[data_start + 28..data_start + 32].try_into().unwrap());
    let sz = f32::from_le_bytes(out[data_start + 32..data_start + 36].try_into().unwrap());
    assert_eq!(sx, 1.0, "TexMapping.size.x must be 1.0, got {sx}");
    assert_eq!(sy, 1.0, "TexMapping.size.y must be 1.0, got {sy}");
    assert_eq!(sz, 1.0, "TexMapping.size.z must be 1.0, got {sz}");

    // projx/projy/projz (chars at offsets 40/41/42) must be X/Y/Z (1/2/3).
    // With projx=NONE(0) Blender maps every ray to UV(0,0) → single corner pixel → dark gobo.
    let projx = out[data_start + 40];
    let projy = out[data_start + 41];
    let projz = out[data_start + 42];
    assert_eq!(projx, 1, "TexMapping.projx must be TEXMAP_PROJ_X=1, got {projx}");
    assert_eq!(projy, 2, "TexMapping.projy must be TEXMAP_PROJ_Y=2, got {projy}");
    assert_eq!(projz, 3, "TexMapping.projz must be TEXMAP_PROJ_Z=3, got {projz}");

    // mat[4][4] identity diagonal: offsets 48, 68, 88, 108 must be 1.0.
    for (i, diag_off) in [48usize, 68, 88, 108].iter().enumerate() {
        let v = f32::from_le_bytes(out[data_start + diag_off..data_start + diag_off + 4].try_into().unwrap());
        assert_eq!(v, 1.0, "TexMapping.mat[{i}][{i}] must be 1.0 (identity), got {v}");
    }

    // max[3]=(1,1,1) at offsets 124-135 — BKE_texture_mapping_init default.
    let mx = f32::from_le_bytes(out[data_start + 124..data_start + 128].try_into().unwrap());
    let my = f32::from_le_bytes(out[data_start + 128..data_start + 132].try_into().unwrap());
    let mz = f32::from_le_bytes(out[data_start + 132..data_start + 136].try_into().unwrap());
    assert_eq!(mx, 1.0, "TexMapping.max.x must be 1.0, got {mx}");
    assert_eq!(my, 1.0, "TexMapping.max.y must be 1.0, got {my}");
    assert_eq!(mz, 1.0, "TexMapping.max.z must be 1.0, got {mz}");
}

#[test]
fn light_gobo_node_tree_image_tex_iuser_frames_and_sfra() {
    // NodeTexImage.iuser must have frames=100 and sfra=1 (matching BKE_imageuser_default).
    // Without these, iuser.frames=0 causes Blender's GPU evaluator to skip the image,
    // producing a black gobo light.
    // NodeTexImage layout: NodeTexBase(960 bytes) + ImageUser(40 bytes) + ...
    // ImageUser: scene*(8) + framenr(4) + frames(4) + offset(4) + sfra(4) + ...
    // iuser.frames at offset 960+12=972; iuser.sfra at offset 960+20=980.
    let mut out = Vec::new();
    let mut ptrs = PtrAlloc::new(0x2000);
    write_light_gobo_node_tree(&mut out, 0x1000, 0x1010, 0x1020, "spot.dds", "//spot.dds", true, &mut ptrs);

    // Find the NodeTexImage storage DATA block (SDNA_IDX_NODE_TEX_IMAGE = 531)
    const HDR: usize = 32;
    let storage_pos = out
        .windows(HDR)
        .position(|h| {
            &h[0..4] == b"DATA"
            && u32::from_le_bytes([h[4], h[5], h[6], h[7]]) == 531
        })
        .expect("NodeTexImage DATA block (SDNA 531) not found");
    let data_start = storage_pos + HDR;

    let frames = u32::from_le_bytes(out[data_start + 972..data_start + 976].try_into().unwrap());
    let sfra   = u32::from_le_bytes(out[data_start + 980..data_start + 984].try_into().unwrap());
    assert_eq!(frames, 100, "iuser.frames must be 100 (BKE_imageuser_default), got {frames}");
    assert_eq!(sfra, 1, "iuser.sfra must be 1 (BKE_imageuser_default), got {sfra}");
}

#[test]
fn light_gobo_node_tree_image_has_tile_block() {
    // Blender 5.x requires at least one ImageTile (UDIM 1001) in Image.tiles for
    // IMA_SRC_FILE images.  Without a tile, Image.tiles ListBase is null, has_data
    // stays false, and the gobo renders as magenta.
    const SDNA_IMAGE_TILE: u32 = 241;
    const HDR: usize = 32;
    let mut out = Vec::new();
    let mut ptrs = PtrAlloc::new(0x2000);
    write_light_gobo_node_tree(&mut out, 0x1000, 0x1010, 0x1020, "gobo.png", "//gobo.png", true, &mut ptrs);

    let tile_block_pos = out.windows(HDR).position(|h| {
        &h[0..4] == b"DATA"
            && u32::from_le_bytes([h[4], h[5], h[6], h[7]]) == SDNA_IMAGE_TILE
    });
    assert!(tile_block_pos.is_some(), "ImageTile DATA block (SDNA 241) not found");

    let tile_start = tile_block_pos.unwrap() + HDR;
    let tile_number = u32::from_le_bytes(out[tile_start + 40..tile_start + 44].try_into().unwrap());
    assert_eq!(tile_number, 1001, "ImageTile.tile_number must be 1001, got {tile_number}");
}

#[test]
fn light_gobo_node_tree_image_tile_ptr_in_image_block() {
    // The Image datablock's tiles ListBase (first/last at offsets 1648/1656) must
    // point at the ImageTile block so Blender can walk the tile linked list.
    // Also verifies the colorspace is Linear Rec.709, matching the reference gobo.blend.
    const SDNA_IMAGE: u32 = 242;
    const HDR: usize = 32;
    let mut out = Vec::new();
    let mut ptrs = PtrAlloc::new(0x2000);
    write_light_gobo_node_tree(&mut out, 0x1000, 0x1010, 0x1020, "gobo.png", "//gobo.png", true, &mut ptrs);

    let image_pos = out.windows(HDR).position(|h| {
        &h[0..4] == b"IM\0\0"
            && u32::from_le_bytes([h[4], h[5], h[6], h[7]]) == SDNA_IMAGE
    }).expect("IM Image block not found");
    let image_data = image_pos + HDR;

    let tiles_first = u64::from_le_bytes(out[image_data + 1648..image_data + 1656].try_into().unwrap());
    let tiles_last  = u64::from_le_bytes(out[image_data + 1656..image_data + 1664].try_into().unwrap());
    assert_ne!(tiles_first, 0, "Image.tiles.first must be nonzero");
    assert_eq!(tiles_first, tiles_last, "Image.tiles.first must equal tiles.last for a single tile");

    // colorspace_settings.name at offset 1576 (64 bytes) — must match reference gobo.blend
    let cs_raw = &out[image_data + 1576..image_data + 1640];
    let cs = cs_raw.split(|&c| c == 0).next().unwrap();
    assert_eq!(cs, b"Linear Rec.709", "gobo image colorspace must be Linear Rec.709, got {:?}", std::str::from_utf8(cs));
}

// ── IDProperty field layout ───────────────────────────────────────────────────

#[test]
fn idproperty_type_at_16() {
    let b = build_idproperty(IDP_STRING, "foo", 0, 0, 0, 0, 0, 0, 0.0, 5, 5);
    assert_eq!(b[16], IDP_STRING);
}

#[test]
fn idproperty_name_at_20() {
    let b = build_idproperty(IDP_INT, "MY_PROP", 0, 0, 0, 0, 0, 0, 0.0, 0, 0);
    let name = b[20..84].split(|&c| c == 0).next().unwrap();
    assert_eq!(name, b"MY_PROP");
}

#[test]
fn idproperty_int_val_at_120() {
    let b = build_idproperty(IDP_INT, "n", 0, 0, 0, 0, 0, -7, 0.0, 0, 0);
    assert_eq!(i32::from_le_bytes(b[120..124].try_into().unwrap()), -7);
}

#[test]
fn idproperty_string_data_ptr_at_88() {
    let b = build_idproperty(IDP_STRING, "s", 0, 0, 0xdead_u64, 0, 0, 0, 0.0, 4, 4);
    assert_eq!(u64::from_le_bytes(b[88..96].try_into().unwrap()), 0xdead_u64);
}

#[test]
fn idproperty_group_first_at_96_last_at_104() {
    let b = build_idproperty(IDP_GROUP, "user_properties", 0, 0, 0, 0x1111_u64, 0x2222_u64, 0, 0.0, 0, 0);
    assert_eq!(u64::from_le_bytes(b[96..104].try_into().unwrap()), 0x1111_u64);
    assert_eq!(u64::from_le_bytes(b[104..112].try_into().unwrap()), 0x2222_u64);
}

#[test]
fn idproperty_string_len_at_128() {
    let b = build_idproperty(IDP_STRING, "s", 0, 0, 0, 0, 0, 0, 0.0, 31, 31);
    assert_eq!(i32::from_le_bytes(b[128..132].try_into().unwrap()), 31);
    assert_eq!(i32::from_le_bytes(b[132..136].try_into().unwrap()), 31);
}

// ── build_idprop_tree ─────────────────────────────────────────────────────────

#[test]
fn idprop_tree_single_int() {
    let mut pa = PtrAlloc::new(0x1000);
    let root_ptr = pa.alloc();
    let child_ptr = pa.alloc();
    let props = vec![("N".to_string(), IdPropValue::Int(42))];
    let (root, children, strings) = build_idprop_tree(root_ptr, &[child_ptr], &[], &props);
    assert_eq!(root.len(), IDPROPERTY_SIZE);
    assert_eq!(root[16], IDP_GROUP);
    // group.first @96 = child_ptr
    assert_eq!(u64::from_le_bytes(root[96..104].try_into().unwrap()), child_ptr);
    assert_eq!(children.len(), 1);
    assert_eq!(strings.len(), 0);
    let (_, cblock) = &children[0];
    assert_eq!(i32::from_le_bytes(cblock[120..124].try_into().unwrap()), 42);
}

#[test]
fn idprop_tree_string_produces_data_block() {
    let mut pa = PtrAlloc::new(0x1000);
    let root_ptr = pa.alloc();
    let child_ptr = pa.alloc();
    let str_data_ptr = pa.alloc();
    let props = vec![("TEMPLATE".to_string(), IdPropValue::String("foo".to_string()))];
    let (_, children, strings) = build_idprop_tree(root_ptr, &[child_ptr], &[str_data_ptr], &props);
    assert_eq!(children.len(), 1);
    assert_eq!(strings.len(), 1);
    let (_, bytes) = &strings[0];
    assert_eq!(bytes, b"foo\0");
    // IDP_STRING: len = 4 (strlen+1)
    let (_, cblock) = &children[0];
    assert_eq!(i32::from_le_bytes(cblock[128..132].try_into().unwrap()), 4);
}

#[test]
fn idprop_tree_linked_list_chain() {
    let mut pa = PtrAlloc::new(0x1000);
    let root_ptr = pa.alloc();
    let ptrs: Vec<u64> = (0..3).map(|_| pa.alloc()).collect();
    let props = vec![
        ("A".to_string(), IdPropValue::Int(1)),
        ("B".to_string(), IdPropValue::Int(2)),
        ("C".to_string(), IdPropValue::Int(3)),
    ];
    let (_, children, _) = build_idprop_tree(root_ptr, &ptrs, &[], &props);
    // A.next = ptrs[1], A.prev = 0
    let (_, a) = &children[0];
    assert_eq!(u64::from_le_bytes(a[0..8].try_into().unwrap()), ptrs[1]);
    assert_eq!(u64::from_le_bytes(a[8..16].try_into().unwrap()), 0);
    // B.next = ptrs[2], B.prev = ptrs[0]
    let (_, b) = &children[1];
    assert_eq!(u64::from_le_bytes(b[0..8].try_into().unwrap()), ptrs[2]);
    assert_eq!(u64::from_le_bytes(b[8..16].try_into().unwrap()), ptrs[0]);
    // C.next = 0, C.prev = ptrs[1]
    let (_, c) = &children[2];
    assert_eq!(u64::from_le_bytes(c[0..8].try_into().unwrap()), 0);
    assert_eq!(u64::from_le_bytes(c[8..16].try_into().unwrap()), ptrs[1]);
}

// ── Vertex group helpers ───────────────────────────────────────────────────────

#[test]
fn bdeformgroup_name_at_16() {
    let b = build_bdeformgroup("GroupA", 0, 0);
    assert_eq!(b.len(), BDEFORMGROUP_SIZE);
    let name = b[16..80].split(|&c| c == 0).next().unwrap();
    assert_eq!(name, b"GroupA");
}

#[test]
fn mdeformweight_layout() {
    let b = build_mdeformweight_array(&[(2, 0.75)]);
    assert_eq!(b.len(), MDEFORMWEIGHT_SIZE);
    assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 2);
    assert!((f32::from_le_bytes(b[4..8].try_into().unwrap()) - 0.75_f32).abs() < 1e-6);
}

#[test]
fn mdeformvert_layout() {
    let b = build_mdeformvert_array(&[(0xabcd_u64, 2)]);
    assert_eq!(b.len(), MDEFORMVERT_SIZE);
    assert_eq!(u64::from_le_bytes(b[0..8].try_into().unwrap()), 0xabcd_u64);
    assert_eq!(i32::from_le_bytes(b[8..12].try_into().unwrap()), 2);
}

// ── Attribute helpers ─────────────────────────────────────────────────────────

#[test]
fn attribute_size_is_24() {
    let a = build_attribute(0x1000, ATTR_TYPE_FLOAT3, ATTR_DOMAIN_POINT, 0x2000);
    assert_eq!(a.len(), ATTRIBUTE_SIZE);
}

#[test]
fn attribute_array_size_is_32() {
    let a = build_attribute_array(0x3000, 4);
    assert_eq!(a.len(), ATTRIBUTE_ARRAY_SIZE);
}

#[test]
fn attribute_type_and_domain() {
    let a = build_attribute(0, ATTR_TYPE_FLOAT2, ATTR_DOMAIN_CORNER, 0);
    assert_eq!(i16::from_le_bytes(a[8..10].try_into().unwrap()), ATTR_TYPE_FLOAT2);
    assert_eq!(a[10], ATTR_DOMAIN_CORNER);
    assert_eq!(a[11], ATTR_STORAGE_ARRAY);
}

// ── Mat/matbits helpers ───────────────────────────────────────────────────────

#[test]
fn mat_ptr_array_is_all_zeroes() {
    let b = build_mat_ptr_array(3);
    assert_eq!(b.len(), 24);
    assert!(b.iter().all(|&x| x == 0));
}

#[test]
fn matbits_all_zeroes() {
    let b = build_matbits(2);
    assert_eq!(b, vec![0, 0]);
}

// ── Data serialisers ──────────────────────────────────────────────────────────

#[test]
fn floats3_data_correct() {
    let d = floats3_data(&[[1.0, 2.0, 3.0]]);
    assert_eq!(d.len(), 12);
    assert_eq!(f32::from_le_bytes(d[0..4].try_into().unwrap()), 1.0_f32);
    assert_eq!(f32::from_le_bytes(d[4..8].try_into().unwrap()), 2.0_f32);
    assert_eq!(f32::from_le_bytes(d[8..12].try_into().unwrap()), 3.0_f32);
}

#[test]
fn ints_data_correct() {
    let d = ints_data(&[0, 1, 2, 3]);
    assert_eq!(d.len(), 16);
    assert_eq!(i32::from_le_bytes(d[4..8].try_into().unwrap()), 1);
}

#[test]
fn bytes4_data_correct() {
    let d = bytes4_data(&[[10, 20, 30, 40]]);
    assert_eq!(d, vec![10, 20, 30, 40]);
}

// ── DNA1 sanity ───────────────────────────────────────────────────────────────

#[test]
fn dna1_starts_with_sdna() {
    assert_eq!(&DNA1_BYTES[0..4], b"SDNA");
}

// ── Collection Object Linking (Phase 5A) ─────────────────────────────────────

#[test]
fn collection_object_size_is_32_bytes() {
    let co = build_collection_object(0x1000);
    assert_eq!(co.len(), COLLECTION_OBJECT_SIZE);
    assert_eq!(COLLECTION_OBJECT_SIZE, 32);
}

#[test]
fn collection_object_layout_no_links() {
    let co = build_collection_object(0xdeadbeef);
    // Offset 16: object pointer
    assert_eq!(u64::from_le_bytes(co[16..24].try_into().unwrap()), 0xdeadbeef);
    // Offset 0-8: prev pointer should be zero (not set)
    assert_eq!(u64::from_le_bytes(co[0..8].try_into().unwrap()), 0);
    // Offset 8-16: next pointer should be zero (not set)
    assert_eq!(u64::from_le_bytes(co[8..16].try_into().unwrap()), 0);
}

#[test]
fn collection_object_linked_builds_doubly_linked_node() {
    // DNA: CollectionObject.next at offset 0, CollectionObject.prev at offset 8.
    // BLO_read_struct_list follows offset 0 (next) to traverse the chain.
    let co = build_collection_object_linked(0xaaaabbbb, 0x1234, 0x5678);
    assert_eq!(co.len(), COLLECTION_OBJECT_SIZE);
    // Offset 0-8: next pointer (Blender traverses via offset 0)
    assert_eq!(u64::from_le_bytes(co[0..8].try_into().unwrap()), 0x5678);
    // Offset 8-16: prev pointer
    assert_eq!(u64::from_le_bytes(co[8..16].try_into().unwrap()), 0x1234);
    // Offset 16-24: object pointer
    assert_eq!(u64::from_le_bytes(co[16..24].try_into().unwrap()), 0xaaaabbbb);
}

#[test]
fn collection_object_linked_single_element_chain() {
    let co = build_collection_object_linked(0x1000, 0, 0);  // No prev, no next
    // Next should be null (offset 0)
    assert_eq!(u64::from_le_bytes(co[0..8].try_into().unwrap()), 0);
    // Prev should be null (offset 8)
    assert_eq!(u64::from_le_bytes(co[8..16].try_into().unwrap()), 0);
    // Object pointer should be set
    assert_eq!(u64::from_le_bytes(co[16..24].try_into().unwrap()), 0x1000);
}

#[test]
fn collection_object_linked_multiple_elements() {
    // DNA: next at offset 0, prev at offset 8
    let co1 = build_collection_object_linked(0x1111, 0, 0x2000);  // First element, no prev, next is second
    let co2 = build_collection_object_linked(0x2222, 0x1000, 0x3000);  // Middle element
    let co3 = build_collection_object_linked(0x3333, 0x2000, 0);  // Last element, next is null

    // Verify chain: co1 -> co2 -> co3 via offset 0 (next)
    assert_eq!(u64::from_le_bytes(co1[0..8].try_into().unwrap()), 0x2000);  // co1 next
    assert_eq!(u64::from_le_bytes(co1[8..16].try_into().unwrap()), 0);      // co1 prev
    
    assert_eq!(u64::from_le_bytes(co2[0..8].try_into().unwrap()), 0x3000);  // co2 next
    assert_eq!(u64::from_le_bytes(co2[8..16].try_into().unwrap()), 0x1000); // co2 prev
    
    assert_eq!(u64::from_le_bytes(co3[0..8].try_into().unwrap()), 0);       // co3 next (null)
    assert_eq!(u64::from_le_bytes(co3[8..16].try_into().unwrap()), 0x2000); // co3 prev
}

#[test]
fn collection_size_is_520_bytes() {
    let c = build_collection("TestColl", 0, 0, 0, 0);
    assert_eq!(c.len(), COLLECTION_SIZE);
    assert_eq!(COLLECTION_SIZE, 520);
}

#[test]
fn collection_gobject_offsets_416_424() {
    // Collection writes object pointers at correct offsets for ListBase.gobject:
    // Offset 416-423: ListBase.first (head of linked list) - AFTER 408-byte ID struct
    // Offset 424-431: ListBase.last (tail of linked list)
    let head_ptr = 0xdeadbeef_u64;
    let tail_ptr = 0xcafebabe_u64;
    let c = build_collection("TestColl", head_ptr, tail_ptr, 0, 0);
    
    // Verify head pointer at offset 416
    assert_eq!(u64::from_le_bytes(c[416..424].try_into().unwrap()), head_ptr);
    // Verify tail pointer at offset 424
    assert_eq!(u64::from_le_bytes(c[424..432].try_into().unwrap()), tail_ptr);
}
