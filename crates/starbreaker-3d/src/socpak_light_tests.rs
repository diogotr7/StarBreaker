//! Synthetic component-layout regressions for authored light fields and state selection.
use super::*;

fn parse(text: &str) -> serde_json::Value {
    let bytes = crate::test_xml::encode(text);
    let xml = starbreaker_cryxml::from_bytes(&bytes).unwrap();
    serde_json::to_value(
        build_light_info_from_component(
            &xml,
            xml.root(),
            "synthetic",
            &[0.0; 3],
            &[1.0, 0.0, 0.0, 0.0],
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn authored_fields_and_zero_default_survive() {
    // Real component attribute layout; every numeric fixture value is synthetic.
    let light = parse(
        r#"<EntityComponentLight lightType="Planar" importance="-7"
        affectsThisAreaOnly="1" affectGI="true" useTemperature="false">
        <sizeParams lightRadius="19" bulbRadius="0" planeWidth="2.25" planeHeight="3.75"/>
        <shadowParams shadowCasting="1" maxShadowCastDist="37"/>
        <fadeParams minDistance="0" maxDistance="83"/>
        <miscParams glowMultiplier="2.5" attenuationTweak="0.125" planeSoftness="0.375"/>
        <offState intensity="0"/>
        <defaultState intensity="0" temperature="4567" lightStyle="9"><color r="0.25" g="0.5" b="0.75"/></defaultState>
        <auxiliaryState intensity="3"/>
        <emergencyState intensity="4"/>
        <cinematicState intensity="5"/>
    </EntityComponentLight>"#,
    );
    let expected = serde_json::json!({
        "bulb_radius_m":0.0, "plane_width_m":2.25, "plane_height_m":3.75,
        "shadow_casting":true, "max_shadow_cast_distance_m":37.0,
        "affects_this_area_only":true, "affect_gi":true, "attenuation_tweak":0.125,
        "plane_softness":0.375, "fade_min_distance_m":0.0, "fade_max_distance_m":83.0,
        "importance":-7, "glow_multiplier":2.5, "radius_m":19.0,
        "active_state":"defaultState", "intensity_raw":0.0, "intensity_candela_proxy":0.0,
        "intensity_candela_proxy_model":"raw_x1500", "color":[0.25,0.5,0.75], "light_style":9
    });
    for (key, value) in expected.as_object().unwrap() {
        assert_eq!(&light[key], value, "{key}");
    }
    assert_eq!(light["states"].as_object().unwrap().len(), 5);
    assert_eq!(light["states"]["auxiliaryState"]["intensity_cd"], 4500.0);
    assert_eq!(light["states"]["defaultState"]["use_temperature"], false);
    assert_eq!(light["states"]["defaultState"]["temperature"], 4567.0);
    assert_eq!(
        light["states"]["auxiliaryState"]["intensity_candela_proxy_model"],
        "raw_x1500"
    );
}

#[test]
fn default_state_wins_even_when_off() {
    let light = parse(
        r#"<EntityComponentLight><defaultState intensity="0"><color r="0.25" g="0.5" b="0.75"/></defaultState><auxiliaryState intensity="3"/></EntityComponentLight>"#,
    );
    assert_eq!(light["active_state"], "defaultState");
    assert_eq!(light["intensity_raw"], 0.0);
    assert_eq!(light["color"], serde_json::json!([0.25, 0.5, 0.75]));
}

#[test]
fn enum_attributes_preserve_names_and_resolve_inherited_gi() {
    // Enum discriminators are component schema, not asset-specific data values.
    for affects_objects in [false, true] {
        let light = parse(&format!(
            r#"<EntityComponentLight importance="SyntheticPriority"
            affectGI="InheritAffectsObjects" affectsObjects="{affects_objects}"/>"#
        ));
        assert_eq!(light["affect_gi"], affects_objects);
        assert_eq!(light["importance_name"], "SyntheticPriority");
        assert!(
            light["importance"].is_null(),
            "ordinal requires the DataCore schema"
        );
    }
    for (mode, enabled) in [("On", true), ("Off", false)] {
        let light = parse(&format!(r#"<EntityComponentLight affectGI="{mode}"/>"#));
        assert_eq!(light["affect_gi"], enabled);
        assert_eq!(light["affect_gi_mode"], mode);
    }
}

#[test]
fn importance_ordinal_follows_schema_order_and_preserves_unknown_names() {
    let source = parse(r#"<EntityComponentLight importance="SyntheticPriority"/>"#);
    for (options, expected) in [
        (vec!["SyntheticPriority", "Other"], Some(0)),
        (vec!["Other", "SyntheticPriority"], Some(1)),
        (vec!["Other"], None),
    ] {
        let mut light: LightInfo = serde_json::from_value(source.clone()).unwrap();
        light_fields::resolve_importance(&mut light, &options);
        assert_eq!(light.importance, expected);
        assert_eq!(light.importance_name.as_deref(), Some("SyntheticPriority"));
    }
    let mut numeric: LightInfo =
        serde_json::from_value(parse(r#"<EntityComponentLight importance="-7"/>"#)).unwrap();
    light_fields::resolve_importance(&mut numeric, &[]);
    assert_eq!(numeric.importance, Some(-7));
}

#[test]
fn live_importance_options_come_from_component_property_schema() {
    let Ok(found) = starbreaker_common::discover::find_p4k() else {
        return;
    };
    let p4k = MappedP4k::open(found.path).unwrap();
    let bytes = p4k
        .read_file(r"Data\Game2.dcb")
        .or_else(|_| p4k.read_file(r"Data\Game.dcb"))
        .unwrap();
    let db = Database::from_bytes(&bytes).unwrap();
    let options = light_fields::importance_options(&db);
    assert!(!options.is_empty());
    for (index, option) in options.iter().enumerate() {
        let mut light: LightInfo = serde_json::from_value(parse(&format!(
            r#"<EntityComponentLight importance="{option}"/>"#
        )))
        .unwrap();
        light_fields::resolve_importance(&mut light, &options);
        assert_eq!(light.importance, Some(index as i32));
    }
    eprintln!("Live importance options: {options:?}");
}

#[test]
fn absent_default_uses_existing_lit_priority() {
    let light = parse(
        r#"<EntityComponentLight useTemperature="1"><offState intensity="0"/><auxiliaryState intensity="0"/><emergencyState intensity="4"/><cinematicState intensity="5"/></EntityComponentLight>"#,
    );
    assert_eq!(light["active_state"], "emergencyState");
    assert_eq!(light["intensity_raw"], 4.0);
    assert_eq!(light["intensity_candela_proxy"], 6000.0);
    assert_eq!(light["states"]["emergencyState"]["use_temperature"], true);
}

#[test]
fn sparse_component_keeps_optional_fields_unknown() {
    let light = parse("<EntityComponentLight/>");
    assert!((light["bulb_radius_m"].as_f64().unwrap() - 0.05).abs() < 1e-7);
    assert_eq!(light["attenuation_tweak"], 1.0);
    assert_eq!(light["glow_multiplier"], 1.0);
    for field in [
        "plane_width_m",
        "plane_height_m",
        "max_shadow_cast_distance_m",
        "plane_softness",
        "fade_min_distance_m",
        "fade_max_distance_m",
        "importance",
    ] {
        assert!(light.as_object().unwrap().contains_key(field), "{field}");
        assert!(light[field].is_null(), "{field}");
    }
    for field in ["shadow_casting", "affects_this_area_only", "affect_gi"] {
        assert_eq!(light[field], false, "{field}");
    }
    assert_eq!(light["active_state"], "");
}

#[test]
fn bundled_glb_extras_preserve_the_entire_light_record() {
    let value = parse(
        r#"<EntityComponentLight><sizeParams bulbRadius="0.25"/><defaultState intensity="2"/></EntityComponentLight>"#,
    );
    let light = serde_json::from_value(value.clone()).unwrap();
    let metadata = crate::gltf::GlbMetadata {
        entity_name: None,
        geometry_path: None,
        material_path: None,
        export_options: crate::gltf::ExportOptionsMetadata {
            kind: "Bundled".into(),
            material_mode: "All".into(),
            format: "glb".into(),
            lod_level: 0,
            texture_mip: 0,
            include_attachments: false,
            include_interior: true,
        },
    };
    let bytes = crate::gltf::GlbBuilder::new()
        .finalize(vec![], vec![light], &metadata)
        .unwrap();
    let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let doc: serde_json::Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
    let record = &doc["extensions"]["KHR_lights_punctual"]["lights"][0];
    assert_eq!(record["extras"], value);
    assert_eq!(record["intensity"], 3000.0);
}
