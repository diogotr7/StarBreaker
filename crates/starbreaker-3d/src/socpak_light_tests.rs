//! Synthetic component-layout regressions for authored light fields and state selection.
use super::*;

#[test]
fn cockpit_projector_deltas_inherit_entity_class_fields() {
    let bytes = crate::test_xml::encode(include_str!(
        "../tests/fixtures/light_class/gladius_projectors.xml"
    ));
    let xml = starbreaker_cryxml::from_bytes(&bytes).unwrap();
    let base: serde_json::Value = serde_json::from_str(include_str!(
        "../tests/fixtures/light_class/light_component.json"
    ))
    .unwrap();
    let base_fields = fixture_base(&base);
    let lights = extract_cryxml_entities(&xml)
        .1
        .into_iter()
        .map(|light| {
            let source = light.source_component.as_ref().unwrap();
            assert_eq!(source.class.as_deref(), Some("Light"));
            build_light_info_from_fields(
                &source.inherit(&base_fields),
                &light.name,
                &light.position,
                &light.rotation,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(lights.len(), 2);
    for light in lights {
        let light = serde_json::to_value(light).unwrap();
        assert_eq!(light["radius_m"], base["sizeParams"]["lightRadius"]);
        assert_eq!(
            light["states"]["defaultState"]["temperature"],
            base["defaultState"]["temperature"]
        );
        assert_eq!(light["affects_this_area_only"], base["affectsThisAreaOnly"]);
        assert_eq!(light["affect_gi"], base["affectsObjects"]);
        assert_eq!(light["importance_name"], base["importance"]);
        assert_eq!(
            light["max_shadow_cast_distance_m"],
            base["shadowParams"]["maxShadowCastDist"]
        );
        assert_eq!(light["plane_softness"], base["miscParams"]["planeSoftness"]);
        assert_eq!(
            light["fade_min_distance_m"],
            base["fadeParams"]["minDistance"]
        );
    }
}

#[test]
fn unknown_gi_and_unresolved_sparse_values_remain_unknown() {
    let light = parse(
        r#"<EntityComponentLight affectGI="FutureMode"><defaultState/></EntityComponentLight>"#,
    );
    assert!(light["affect_gi"].is_null());
    assert_eq!(light["affect_gi_mode"], "FutureMode");
    assert!(light["radius_m"].is_null());
    assert!(light["states"]["defaultState"]["intensity_raw"].is_null());
    assert!(light["states"]["defaultState"]["temperature"].is_null());
    for key in ["use_temperature", "light_style", "preset_tag", "color"] {
        assert!(light["states"]["defaultState"][key].is_null(), "{key}");
    }
    for key in ["light_type", "light_style", "color"] {
        assert!(light[key].is_null(), "{key}");
    }
    let inherited = parse(r#"<EntityComponentLight affectGI="InheritAffectsObjects"/>"#);
    assert!(inherited["affect_gi"].is_null());
}

#[test]
fn enum_diagnostics_warn_once_per_export_and_reset_on_next_export() {
    struct Capture;
    thread_local! {
        static WARNINGS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if record.level() == log::Level::Warn {
                WARNINGS.with(|messages| messages.borrow_mut().push(record.args().to_string()));
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Capture = Capture;
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    // Synthetic empty v8 database: no LightComponentParams struct or enum metadata.
    let mut bytes = [0_u8; 120];
    bytes[4..8].copy_from_slice(&8_u32.to_le_bytes());
    let db = Database::from_bytes(&bytes).unwrap();
    for _ in 0..2 {
        WARNINGS.with(|messages| messages.borrow_mut().clear());
        let mut resolver = light_component::LightResolver::new(&db);
        for mode in ["FutureMode", "AnotherFutureMode", "FutureMode"] {
            let light: LightInfo = serde_json::from_value(parse(&format!(
                r#"<EntityComponentLight affectGI="{mode}" importance="FuturePriority"/>"#
            )))
            .unwrap();
            let resolved = resolver.resolve(light);
            assert_eq!(resolved.affect_gi, None);
            assert_eq!(resolved.affect_gi_mode.as_deref(), Some(mode));
            assert_eq!(resolved.importance, None);
            assert_eq!(resolved.importance_name.as_deref(), Some("FuturePriority"));
        }
        WARNINGS.with(|messages| {
            let messages = messages.borrow();
            assert_eq!(
                messages.iter().filter(|s| s.contains("affectGI")).count(),
                1,
                "{messages:?}"
            );
            assert_eq!(
                messages
                    .iter()
                    .filter(|s| s.contains("importance enum"))
                    .count(),
                1,
                "{messages:?}"
            );
            assert_eq!(messages.len(), 2, "{messages:?}");
        });
    }
}

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
    assert!(light["glow_multiplier"].is_null());
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
        assert!(light[field].is_null(), "{field}");
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

// Decode the extracted JSON into the same Value tree used by a DataCore query.
fn fixture_base(json: &serde_json::Value) -> light_component::LightComponent {
    fn value(json: &serde_json::Value) -> Value<'_> {
        match json {
            serde_json::Value::Object(fields) => Value::Object {
                type_name: "fixture",
                fields: fields
                    .iter()
                    .filter(|(k, _)| !k.starts_with("__"))
                    .map(|(k, v)| (k.as_str(), value(v)))
                    .collect(),
                record_id: None,
            },
            serde_json::Value::Bool(v) => Value::Bool(*v),
            serde_json::Value::Number(v) => Value::Float(v.as_f64().unwrap() as f32),
            serde_json::Value::String(v) => Value::String(v),
            _ => Value::Null,
        }
    }
    light_component::LightComponent::from_datacore(&value(json))
}

#[test]
fn partial_default_inherits_intensity_but_explicit_zero_wins() {
    let base_json: serde_json::Value = serde_json::from_str(include_str!(
        "../tests/fixtures/light_class/light_component.json"
    ))
    .unwrap();
    let mut base = fixture_base(&base_json);
    // Distinct synthetic class values prove these are looked up, not constants.
    base.attributes
        .insert("sizeParams.bulbRadius".into(), "0.375".into());
    base.attributes
        .insert("miscParams.attenuationTweak".into(), "0.625".into());
    for (attributes, expected) in [("", 1.0), ("intensity=\"0\"", 0.0)] {
        let bytes = crate::test_xml::encode(&format!(
            "<EntityComponentLight><defaultState {attributes}/></EntityComponentLight>"
        ));
        let xml = starbreaker_cryxml::from_bytes(&bytes).unwrap();
        let source = light_component::LightComponent::from_xml(&xml, xml.root()).inherit(&base);
        let light =
            build_light_info_from_fields(&source, "synthetic", &[0.0; 3], &[1.0, 0.0, 0.0, 0.0]);
        assert_eq!(light.intensity_raw, Some(expected));
        assert_eq!(light.states["defaultState"].temperature, Some(6600.0));
        assert_eq!(light.bulb_radius_m, 0.375);
        assert_eq!(light.attenuation_tweak, 0.625);
    }
}

#[test]
fn live_class_fixture_and_projector_exports_match_datacore() {
    let Ok(found) = starbreaker_common::discover::find_p4k() else {
        return;
    };
    let p4k = MappedP4k::open(found.path).unwrap();
    let bytes = p4k
        .read_file(r"Data\Game2.dcb")
        .or_else(|_| p4k.read_file(r"Data\Game.dcb"))
        .unwrap();
    let db = Database::from_bytes(&bytes).unwrap();
    let mut resolver = light_component::LightResolver::new(&db);
    let base_json: serde_json::Value = serde_json::from_str(include_str!(
        "../tests/fixtures/light_class/light_component.json"
    ))
    .unwrap();
    let base = resolver.class_component("Light").unwrap().clone();
    assert_eq!(
        base,
        fixture_base(&base_json),
        "refresh extracted fixture when DataCore changes"
    );
    let options = light_fields::importance_options(&db);
    for mode in light_fields::enum_options(&db, "affectGI") {
        assert!(
            light_fields::affect_gi(Some(mode), Some(true)).is_some(),
            "new LIVE affectGI mode: {mode}"
        );
    }
    let identity = glam::Mat4::IDENTITY.to_cols_array_2d();
    let payload = load_interior_from_socpak(
        &p4k,
        "Data/ObjectContainers/Ships/AEGS/Gladius/base_lightgroups.socpak",
        identity,
        identity,
        &[identity],
    )
    .unwrap();
    let fixture_bytes = crate::test_xml::encode(include_str!(
        "../tests/fixtures/light_class/gladius_projectors.xml"
    ));
    let fixture_xml = starbreaker_cryxml::from_bytes(&fixture_bytes).unwrap();
    for fixture in extract_cryxml_entities(&fixture_xml).1 {
        let raw = payload
            .lights
            .iter()
            .find(|l| l.name == fixture.name)
            .unwrap();
        assert_eq!(
            raw.source_component, fixture.source_component,
            "refresh SOC fixture when data changes"
        );
        let light = resolver.resolve(raw.clone());
        assert_eq!(light.radius_m, base.float("sizeParams.lightRadius"));
        assert_eq!(
            light.states["defaultState"].temperature,
            base.float("defaultState.temperature")
        );
        assert_eq!(
            light.affects_this_area_only,
            base.boolean("affectsThisAreaOnly")
        );
        assert_eq!(light.affect_gi, base.boolean("affectsObjects"));
        assert_eq!(
            light.importance,
            options
                .iter()
                .position(|v| Some(*v) == base.attr("importance"))
                .map(|i| i as i32)
        );
        assert_eq!(
            light.shadow_casting,
            base.boolean("shadowParams.shadowCasting")
        );
        assert_eq!(light.plane_softness, base.float("miscParams.planeSoftness"));
        assert_eq!(
            light.max_shadow_cast_distance_m,
            base.float("shadowParams.maxShadowCastDist")
        );
        eprintln!(
            "{}: radius={:?}, temperature={:?}, area={:?}, importance={:?} {:?}",
            light.name,
            light.radius_m,
            light.states["defaultState"].temperature,
            light.affects_this_area_only,
            light.importance_name,
            light.importance
        );
    }
}
