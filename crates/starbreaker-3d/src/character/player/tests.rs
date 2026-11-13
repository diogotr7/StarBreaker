//! Synthetic parser/resolver regressions; no extracted game data is embedded.
use super::*;
use serde_json::json;

#[test]
fn bspace_alias_faces_and_missing_parameters() {
    let doc = xml::parse(br#"<ParaGroup><Dimensions><Param Name="MoveSpeed" Min="0" Max="7" Cells="4"/></Dimensions><ExampleList><Example AName="synthetic" Alias="a" PlaybackScale="2"/><Example AName="other" Alias="b" SetPara0="3"/></ExampleList><Blendable><Face p0="a" p1="b"/></Blendable></ParaGroup>"#).unwrap();
    let b = blend_space::parse(&doc).unwrap();
    assert_eq!(b["examples"][0]["params"], json!([null]));
    assert_eq!(b["examples"][1]["params"], json!([3.0]));
    assert_eq!(b["faces"], json!([[0, 1]]));
    assert_eq!(b["examples"][0]["playback_scale"], 2.0);
}

#[test]
fn bspace_rejects_dangling_alias() {
    let doc = xml::parse(br#"<ParaGroup><Dimensions><Param Name="MoveSpeed" Min="0" Max="7" Cells="4"/></Dimensions><ExampleList><Example AName="synthetic" Alias="a"/></ExampleList><Blendable><Face p0="missing"/></Blendable></ParaGroup>"#).unwrap();
    assert!(blend_space::parse(&doc).is_err());
}

#[test]
fn loadout_ports_keep_flags_offsets_and_gender_helper() {
    let port = json!({"Name":"slot","Flags":"select invis_p1","Types":[{"Type":"Thing","SubTypes":["Small"]}],"Extension":{"SelectTag":"synthetic"},"AttachmentImplementation":{"_Type_":"SItemPortDefAttachmentImplementationBone","Helper":{"Tags":"","Helper":{"Name":"joint","Offset":{"Position":{"x":1,"y":2,"z":3},"Rotation":{"x":0,"y":0,"z":90}},"ItemOffsetHelperName":"anchor"},"SubHelpers":[{"Tags":"Female","Helper":{"Name":"other_joint","Offset":{"Position":{"x":4,"y":5,"z":6},"Rotation":{"x":0,"y":0,"z":0}},"ItemOffsetHelperName":"other_anchor"},"SubHelpers":[]}]}}});
    let p = ports::parse_port(&port, "owner", &["Female".into()]).unwrap();
    assert_eq!(p["attach_bone"], "other_joint");
    assert_eq!(p["flags"], json!(["select", "invis_p1"]));
    assert_eq!(p["item_offset_helper"], "other_anchor");
    assert_eq!(p["select_tag"], "synthetic");
    assert_eq!(p["offset"]["position"], json!([4.0, 5.0, 6.0]));
    assert_eq!(ports::parse_port(&port, "owner", &[]).unwrap()["attach_bone"], "joint");
}

#[test]
fn fragment_resolution_is_ordered_subset_matching() {
    let f = json!([
        {"fragment":"action","tags":["AI"],"frag_tags":[],"guid":"ai"},
        {"fragment":"action","tags":["armed","FP"],"frag_tags":["left"],"guid":"specific"},
        {"fragment":"action","tags":["armed"],"frag_tags":[],"guid":"fallback"}
    ]);
    let fs = f.as_array().unwrap();
    assert_eq!(
        motion::resolve(fs, "action", &["armed".into(), "FP".into()], &["left".into()]).unwrap()["guid"],
        "specific"
    );
    assert_eq!(
        motion::resolve(fs, "action", &["armed".into(), "FP".into()], &[]).unwrap()["guid"],
        "fallback"
    );
    assert!(motion::resolve(fs, "action", &[], &[]).is_none());
}

#[test]
fn root_motion_parameters_use_root_frame_and_playback_scale() {
    let r = json!({"start":{"position":[0,0,0],"rotation":[1,0,0,0]},"end":{"position":[3,4,0],"rotation":[1,0,0,0]}});
    assert!((clips::parameter("MoveSpeed", &r, 2.0, 2.0).unwrap() - 5.0).abs() < 1e-6);
    assert!((clips::parameter("TravelAngle", &r, 2.0, 1.0).unwrap() - (-3.0_f64).atan2(4.0)).abs() < 1e-6);
    assert!(clips::parameter("Unknown", &r, 2.0, 1.0).is_err());
}

#[test]
fn additive_detection_uses_asset_flags_not_name() {
    assert!(clips::asset_flags(1).0);
    assert!(!clips::asset_flags(0).0);
    assert!(!clips::asset_flags(2).0);
}

#[test]
fn combined_spaces_preserve_component_topology() {
    let doc=xml::parse(br#"<CombinedBlendSpace><Dimensions><Param name="MoveSpeed" ChooseBlendSpace="1"/></Dimensions></CombinedBlendSpace>"#).unwrap();
    let make = |id: &str, min: f64| max_space(id, min);
    fn max_space(id: &str, min: f64) -> serde_json::Value {
        json!({"id":id,"dimensions":[{"name":"MoveSpeed","min":min,"max":min+2.0,"cells":3}],"examples":[{"clip":"synthetic","alias":"a","params":[min]}],"faces":[[0]]})
    }
    let b = blend_space::combine(&doc, &[make("a", 0.0), make("b", 2.0)]).unwrap();
    assert_eq!(b["faces"], json!([[0], [1]]));
    assert_eq!(b["dimensions"][0]["max"], 4.0);
    assert_eq!(b["components"], json!(["a", "b"]));
}

#[test]
fn legacy_bspace_faces_use_checked_numeric_indices() {
    let doc=xml::parse(br#"<ParaGroup><Dimensions><Param Name="TurnAngle" Min="-1" Max="1" Cells="3"/></Dimensions><ExampleList><Example AName="synthetic"/><Example AName="other"/></ExampleList><Blendable><Face p0="0" p1="1"/></Blendable></ParaGroup>"#).unwrap();
    assert_eq!(blend_space::parse(&doc).unwrap()["faces"], json!([[0, 1]]));
}

#[test]
fn xml_numbers_allow_authored_whitespace() {
    let node = xml::parse(br#"<Param Min=" 0.00 " Max=" 7 "/>"#).unwrap();
    assert_eq!(node.number("Min", -1.0).unwrap(), 0.0);
    assert_eq!(node.number("Max", -1.0).unwrap(), 7.0);
}

#[test]
fn motion_category_uses_local_schema_not_inherited_motion_tags() {
    let unrelated=xml::parse(br#"<TagDefinition><Imports><Import filename="synthetic-motion.xml"/></Imports><Tags><Group name="LandingStrength"/></Tags></TagDefinition>"#).unwrap();
    let motion =
        xml::parse(br#"<TagDefinition><Tags><Group name="MotionHemisphere"/></Tags></TagDefinition>"#).unwrap();
    assert!(!motion::ground_schema(&unrelated));
    assert!(motion::ground_schema(&motion));
}
#[test]
fn source_stem_handles_extensionless_database_paths() {
    assert_eq!(sources::stem("synthetic/path/Clip"), "clip");
    assert_eq!(sources::stem("synthetic\\path\\Clip.caf"), "clip");
}

#[test]
fn stow_queries_do_not_inherit_carry_motion_base() {
    assert_eq!(motion::query_base(true, Some("synthetic_carry".into())), None);
    assert_eq!(
        motion::query_base(false, Some("synthetic_body".into())),
        Some("synthetic_body".into())
    );
}

#[test]
fn body_motion_excludes_fragments_that_animate_an_external_item() {
    let c=xml::parse(br#"<ControllerDef><FragmentDefs><synthetic_walk scopes="body+sound"/><synthetic_take scopes="body+item"/></FragmentDefs><ScopeDefs><body context="actor" alwaysRootScope="1" numLayers="2"/><sound context="audio" numLayers="0"/><item context="prop" numLayers="3"/></ScopeDefs></ControllerDef>"#).unwrap();
    assert!(motion::body_motion_scope(&c, "synthetic_walk"));
    assert!(!motion::body_motion_scope(&c, "synthetic_take"));
}

#[test]
fn nested_provenance_is_scoped_to_its_package() {
    let mut file = crate::ExportedFile {
        relative_path: "Data/UI/Generated/.export_stamp.json".into(),
        bytes: b"synthetic".to_vec(),
        kind: crate::ExportedFileKind::PackageManifest,
    };
    export::relocate_provenance(&mut file, "Packages/synthetic");
    assert_eq!(file.relative_path, "Packages/synthetic/.export_stamp.json");
    assert_eq!(file.bytes, b"synthetic");
}

#[test]
fn nested_assets_are_rebased_without_changing_source_paths() {
    let mut scene = json!({"mesh_asset":"Data/Objects/synthetic.glb","material_sidecar":"Data/Objects/synthetic.materials.json","geometry_path":"Objects/synthetic.skin"});
    let mut files = vec![
        crate::ExportedFile {
            relative_path: "Data/Objects/synthetic.materials.json".into(),
            bytes: br#"{"texture":"Data/Objects/synthetic.png"}"#.to_vec(),
            kind: crate::ExportedFileKind::MaterialSidecar,
        },
        crate::ExportedFile {
            relative_path: "Data/Objects/synthetic.png".into(),
            bytes: vec![1, 2, 3],
            kind: crate::ExportedFileKind::TextureAsset,
        },
        crate::ExportedFile {
            relative_path: "Data/Objects/synthetic.glb".into(),
            bytes: vec![4, 5, 6],
            kind: crate::ExportedFileKind::MeshAsset,
        },
    ];
    export::rebase_assets(&mut scene, &mut files, "instance").unwrap();
    assert_eq!(scene["mesh_asset"], "Data/PlayerItems/instance/Objects/synthetic.glb");
    assert_eq!(scene["geometry_path"], "Objects/synthetic.skin");
    let mat: serde_json::Value = serde_json::from_slice(&files[0].bytes).unwrap();
    assert_eq!(mat["texture"], "Data/PlayerItems/instance/Objects/synthetic.png");
}

#[test]
fn repeated_item_package_manifests_have_instance_paths() {
    let mut scene = json!({"liveries":"Packages/synthetic/liveries.json"});
    let mut files = vec![crate::ExportedFile {
        relative_path: "Packages/synthetic/liveries.json".into(),
        bytes: b"{}".to_vec(),
        kind: crate::ExportedFileKind::PackageManifest,
    }];
    export::rebase_assets(&mut scene, &mut files, "instance").unwrap();
    assert_eq!(
        files[0].relative_path,
        "Packages/PlayerItems/instance/synthetic/liveries.json"
    );
    assert_eq!(scene["liveries"], files[0].relative_path);
}

#[test]
fn missing_clips_preserve_authored_selection_and_report_context() {
    let mut states = vec![
        json!({"tags":["synthetic_fp"],"fragments":[{"fragment":"synthetic_action","tags":["synthetic"],"frag_tags":["slot"],"layers":[{"clip":"absent"},{"clip":"present"}]}]}),
    ];
    let mut spaces = std::collections::BTreeMap::new();
    let missing = std::collections::BTreeSet::from(["absent".into()]);
    let d = diagnostics::mark_missing(&mut states, &mut spaces, &missing, "female");
    assert_eq!(states[0]["fragments"][0]["layers"][0]["clip"], "absent");
    assert_eq!(states[0]["fragments"][0]["layers"][0]["missing"], true);
    assert!(states[0]["fragments"][0]["layers"][1].get("missing").is_none());
    assert_eq!(
        d[0],
        json!({"kind":"missing_clip","clip":"absent","fragment":"synthetic_action","tags":["synthetic"],"frag_tags":["slot"],"gender":"female"})
    );
}

#[test]
fn chrparams_includes_preserve_authored_shared_source_roots() {
    let doc=xml::parse(br##"<Params><AnimationList><Animation name="$Include" path="synthetic-shared.chrparams"/><Animation name="#filepath" path="Animations/local"/><Animation name="*" path="*/*.caf"/></AnimationList></Params>"##).unwrap();
    let (files,dbs)=sources::search_rules(&doc,&mut |path| {
        assert_eq!(path,"synthetic-shared.chrparams");
        xml::parse(br##"<Params><AnimationList><Animation name="$TracksDatabase" path="Animations/shared/*/*.dba"/><Animation name="#filepath" path="Animations/shared"/><Animation name="*" path="*/*.caf"/></AnimationList></Params>"##)
    },&mut Default::default()).unwrap();
    assert_eq!(dbs, vec!["data/animations/shared/*/*.dba"]);
    assert_eq!(
        files,
        vec![
            ("*".into(), "data/animations/shared/*/*.caf".into()),
            ("*".into(), "data/animations/local/*/*.caf".into())
        ]
    );
}

#[test]
fn wearer_identity_is_not_exported_again_as_loadout_clothing() {
    use super::super::loadout::LoadoutEntry;
    let entries = vec![LoadoutEntry {
        port: "body".into(),
        item: "synthetic_body".into(),
        children: vec![
            LoadoutEntry {
                port: "face".into(),
                item: "synthetic_head".into(),
                children: vec![],
            },
            LoadoutEntry {
                port: "armour".into(),
                item: "synthetic_gear".into(),
                children: vec![],
            },
        ],
    }];
    let result = loadouts::equipment(&entries, &std::collections::BTreeSet::from(["face".into()]));
    assert_eq!(result[0].children.len(), 1);
    assert_eq!(result[0].children[0].item, "synthetic_gear");
}

#[test]
fn weapon_motion_base_is_shared_across_fragment_schemas() {
    use std::collections::BTreeMap;
    let motion = BTreeMap::from([("MotionBase".into(), vec!["synthetic_base".into()])]);
    let mut stow = motion.clone();
    stow.insert("selectTag".into(), vec!["slot".into()]);
    let schemas = BTreeMap::from([
        ("synthetic_idle".into(), motion.clone()),
        ("synthetic_move".into(), motion),
        ("synthetic_stow".into(), stow),
    ]);
    let fragments = vec![
        json!({"fragment":"synthetic_idle","tags":["synthetic_weapon","synthetic_stance"],"frag_tags":["synthetic_base"]}),
        json!({"fragment":"synthetic_move","tags":["synthetic_stance"],"frag_tags":["synthetic_base"]}),
    ];
    let bases = queries::bases(
        &fragments,
        &schemas,
        &["synthetic_weapon".into(), "synthetic_stance".into()],
        &["synthetic_weapon".into()],
    );
    assert_eq!(bases["synthetic_idle"].as_deref(), Some("synthetic_base"));
    assert_eq!(bases["synthetic_move"], bases["synthetic_idle"]);
    assert_eq!(bases["synthetic_stow"], None);
    assert!(
        queries::bases(&fragments, &schemas, &["synthetic_stance".into()], &[])
            .values()
            .all(Option::is_none)
    );
}

#[test]
fn weapon_queries_enumerate_every_authored_speed_and_the_untagged_state() {
    let groups = std::collections::BTreeMap::from([(
        "moveSpeed".into(),
        vec!["synthetic_slow".into(), "synthetic_fast".into()],
    )]);
    assert_eq!(
        queries::speeds(&groups)
            .into_iter()
            .map(|s| s.map(String::as_str))
            .collect::<Vec<_>>(),
        vec![None, Some("synthetic_slow"), Some("synthetic_fast")]
    );
}

#[test]
fn weapon_motion_base_association_can_be_authored_in_another_stance() {
    let tags = vec![
        "synthetic_weapon".into(),
        "synthetic_crouch".into(),
        "synthetic_speed".into(),
    ];
    let context = queries::base_context(&tags, &["synthetic_stand".into(), "synthetic_crouch".into()]);
    assert!(ports::subset(
        &["synthetic_weapon".into(), "synthetic_stand".into()],
        &context
    ));
    assert!(ports::subset(&tags, &context));
    assert!(!context.contains(&"unrequested_speed".into()));
}
