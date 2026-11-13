//! Curated emissive and MeshDecal sidecar semantics from authored shader data.
//! Slot references join the submaterial's texture_slots array; no texture-name heuristics.
use crate::mtl::{ShaderFamily, SubMaterial, TextureSemanticRole};
use serde_json::{Value, json};

pub(crate) fn emissive(material: &SubMaterial) -> Value {
    let emittance = material
        .texture_slots
        .iter()
        .filter(|_| material.has_emittance_map())
        .find(|slot| {
            slot.slot.eq_ignore_ascii_case("Emittance")
                || slot.slot.eq_ignore_ascii_case("Emissive")
                || (material.shader_family() == ShaderFamily::HardSurface
                    && slot.slot.eq_ignore_ascii_case("TexSlot14"))
        });
    let color = material.emissive_tint();
    let cutoff = material
        .public_params
        .iter()
        .map(|p| (&p.name, &p.value))
        .chain(
            material
                .authored_attributes
                .iter()
                .map(|a| (&a.name, &a.value)),
        )
        .find(|(name, _)| name.eq_ignore_ascii_case("AlphaGlowCutoff"))
        .map(|(_, value)| super::decomposed::string_value_to_json(value));
    json!({"color": color, "strength": material.emissive_strength(),
        "forward_rendering": material.is_forward_rendered(),
        "uses_base_texture": !material.has_emittance_map(),
        "emittance_texture": emittance.map(|s| &s.slot),
        "alpha_glow_cutoff": cutoff, "glow": material.glow})
}

/// Raw coverage controls; forward cards need soft alpha in the host renderer.
/// This does not replace the bundled glTF alpha approximation in SubMaterial.
pub(crate) fn alpha_config(material: &SubMaterial) -> Value {
    json!({
        "forward_rendering": material.is_forward_rendered(),
        "two_sided": material.authored_two_sided(),
        "alpha_test": material.alpha_test,
        "opacity": material.opacity,
    })
}

pub(crate) fn decal(material: &SubMaterial) -> Option<Value> {
    if material.shader_family() != ShaderFamily::MeshDecal {
        return None;
    }
    let flags = material.decoded_string_gen_mask();
    let slots = material.semantic_texture_slots();
    let has = |slot: &str| slots.iter().any(|s| s.slot.eq_ignore_ascii_case(slot));
    // Same structural exclusions as Blender's _mesh_decal_pom_payload_is_control_only.
    let color_signal = flags.has_decal
        || flags.has_stencil_map
        || flags.tokens.iter().any(|t| t == "DECAL_OPACITY_MAP")
        || slots.iter().any(|s| {
            matches!(
                s.role,
                TextureSemanticRole::DecalSheet
                    | TextureSemanticRole::Opacity
                    | TextureSemanticRole::Stencil
                    | TextureSemanticRole::TintPaletteDecal
                    | TextureSemanticRole::RenderToTexture
            )
        })
        || ["TexSlot5", "TexSlot6", "TexSlot7", "TexSlot8"]
            .iter()
            .any(|s| has(s));
    let palette_stencil = flags.has_stencil_map
        && slots
            .iter()
            .any(|s| s.path.eq_ignore_ascii_case("$TintPaletteDecal"));
    let kind = if palette_stencil {
        "stencil_palette"
    } else if flags.has_parallax_occlusion_mapping && !color_signal {
        "relief_only"
    } else if flags.tokens.iter().any(|t| t == "DIFFUSE_MAP") && has("TexSlot1") {
        "color_sheet"
    } else {
        "unknown"
    };
    let refs = [
        ("coverage", "TexSlot1"),
        ("normal_gloss", "TexSlot3"),
        ("height", "TexSlot4"),
        ("breakup", "TexSlot5"),
        ("tint_mask", "TexSlot6"),
        ("stencil", "TexSlot7"),
        ("grime", "TexSlot8"),
    ]
    .into_iter()
    .filter(|(_, slot)| has(slot))
    .map(|(role, slot)| (role.to_owned(), json!(slot)))
    .chain(
        (kind == "color_sheet" && has("TexSlot1"))
            .then(|| ("base_color".to_owned(), json!("TexSlot1"))),
    )
    .collect::<serde_json::Map<_, _>>();
    let params = material
        .public_params
        .iter()
        .filter(|p| {
            p.name.starts_with("Stencil")
                || [
                    "DecalAlphaMultiplier",
                    "DecalDiffuseOpacity",
                    "DecalFalloff",
                    "PomDisplacement",
                    "PomHeightBias",
                ]
                .contains(&p.name.as_str())
        })
        .map(|p| (p.name.clone(), parameter_value(&p.value)))
        .collect::<serde_json::Map<_, _>>();
    Some(json!({"kind": kind, "slots": refs, "public_params": params}))
}

fn parameter_value(value: &str) -> Value {
    let parts = value.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() == 3 {
        if let Ok(rgb) = parts
            .iter()
            .map(|p| p.parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
        {
            if rgb.iter().all(|v| v.is_finite()) {
                return json!(rgb);
            }
        }
    }
    super::decomposed::string_value_to_json(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn material(xml: &str) -> SubMaterial {
        crate::mtl::parse_mtl(&crate::test_xml::encode(xml))
            .unwrap()
            .materials
            .remove(0)
    }
    // Synthetic colours/scalars; these reproduce the four authored categories.
    fn check_emission(xml: &str, tint: [f32; 3], strength: f32) {
        let m = material(xml);
        let e = emissive(&m);
        assert_eq!(e["color"], json!(tint));
        assert_eq!(e["strength"], json!(strength));
        assert_eq!(m.emissive_factor(), tint.map(|v| v * strength));
    }
    #[test]
    fn black_emissive_with_glow_is_untinted_surface_emission() {
        check_emission(
            r#"<Material Shader="Illum" Emissive="0,0,0" Glow="0.25" Diffuse="1,0.5,0"/>"#,
            [1.0; 3],
            0.25,
        );
    }
    #[test]
    fn white_emissive_with_glow_keeps_surface_colour() {
        check_emission(
            r#"<Material Shader="Illum" Emissive="1,1,1" Glow="0.5" Diffuse="0.25,0.5,1"/>"#,
            [1.0; 3],
            0.5,
        );
    }
    #[test]
    fn illum_neutral_emissive_without_signal_stays_off() {
        check_emission(
            r#"<Material Shader="Illum" Emissive="1,1,1"/>"#,
            [1.0; 3],
            0.0,
        );
    }
    #[test]
    fn layer_neutral_emissive_without_signal_stays_off() {
        check_emission(
            r#"<Material Shader="LayerBlend_V2" Emissive="1,1,1"/>"#,
            [1.0; 3],
            0.0,
        );
    }
    #[test]
    fn authored_alpha_controls_are_independent_of_decal_inference() {
        for flags in ["32770", "0x8002"] {
            let m = material(&format!(
                r#"<Material Shader="Illum" MtlFlags="{flags}" AlphaTest="0.125" Opacity="0.625"/>"#
            ));
            assert_eq!(
                alpha_config(&m),
                json!({"forward_rendering":true,
                "two_sided":true, "alpha_test":0.125, "opacity":0.625})
            );
        }
        let m = material(r#"<Material Shader="MeshDecal" MtlFlags="0"/>"#);
        assert!(m.is_double_sided()); // Existing glTF policy is deliberately separate.
        assert_eq!(
            alpha_config(&m),
            json!({"forward_rendering":false,
            "two_sided":false, "alpha_test":0.0, "opacity":1.0})
        );
    }
    #[test]
    fn signed_material_flags_preserve_the_authored_bits() {
        for (flags, bits) in [
            ("-1", u64::MAX),
            ("-9223372036854743038", (i64::MIN + 32770) as u64),
        ] {
            let m = material(&format!(r#"<Material Shader="Illum" MtlFlags="{flags}"/>"#));
            assert_eq!(m.material_flags(), bits);
            assert!(m.is_forward_rendered());
            assert!(m.authored_two_sided());
        }
    }
    #[test]
    fn forward_illum_cards_emit_without_glow_but_other_families_do_not() {
        // Synthetic authored categories; names are deliberately irrelevant.
        for shader in ["Illum", "HardSurface"] {
            check_emission(
                &format!(r#"<Material Shader="{shader}" MtlFlags="32768" Emissive="1,1,1"/>"#),
                [1.0; 3],
                1.0,
            );
            check_emission(
                &format!(
                    r#"<Material Shader="{shader}" MtlFlags="32768" Emissive="0.25,0.5,0.75"/>"#
                ),
                [0.25, 0.5, 0.75],
                1.0,
            );
            check_emission(
                &format!(r#"<Material Shader="{shader}" MtlFlags="32768" Emissive="0,0,0"/>"#),
                [1.0; 3],
                0.0,
            );
            check_emission(
                &format!(
                    r#"<Material Shader="{shader}" MtlFlags="32768" Emissive="0.25,0.5,0.75" Glow="0.125"/>"#
                ),
                [0.25, 0.5, 0.75],
                0.125,
            );
            check_emission(
                &format!(
                    r#"<Material Shader="{shader}" MtlFlags="32768" Emissive="0.25,0.5,0.75" StringGenMask="%EMITTANCE_MAP"/>"#
                ),
                [0.25, 0.5, 0.75],
                1.0,
            );
        }
        for flags in ["0", "32768"] {
            check_emission(
                &format!(
                    r#"<Material Shader="LayerBlend_V2" MtlFlags="{flags}" Emissive="1,1,1"/>"#
                ),
                [1.0; 3],
                0.0,
            );
        }
        let m = material(r#"<Material Shader="Illum" MtlFlags="32768" Emissive="1,1,1"/>"#);
        assert_eq!(emissive(&m)["forward_rendering"], true);
    }
    #[test]
    fn emittance_token_is_a_signal_without_a_named_slot_or_glow() {
        let m = material(
            r#"<Material Shader="Illum" Emissive="0.5,1,0.25" StringGenMask="%EMITTANCE_MAP"><PublicParams AlphaGlowCutoff="0.4"/></Material>"#,
        );
        assert_eq!(emissive(&m)["strength"], json!(1.0));
        assert_eq!(emissive(&m)["color"], json!([0.5, 1.0, 0.25]));
        assert_eq!(emissive(&m)["uses_base_texture"], false);
        assert_eq!(emissive(&m)["alpha_glow_cutoff"], json!(0.4));
        assert_eq!(m.emissive_factor(), [0.5, 1.0, 0.25]);
    }
    #[test]
    fn macro_emittance_uses_shader_token_and_slot_binding() {
        let m = material(
            r#"<Material Shader="HardSurface" Emissive="0,0,0" StringGenMask="%MACRO_EMISSIVE_MAP"><Textures><Texture Map="TexSlot14" File="arbitrary.tif"/></Textures></Material>"#,
        );
        assert_eq!(emissive(&m)["strength"], json!(1.0));
        assert_eq!(emissive(&m)["emittance_texture"], "TexSlot14");
        assert_eq!(m.emissive_factor(), [1.0; 3]);
        let m = material(
            r#"<Material Shader="HardSurface"><Textures><Texture Map="TexSlot14" File="emissive_name.tif"/></Textures></Material>"#,
        );
        assert_eq!(m.emissive_strength(), 0.0);
    }

    #[test]
    fn decal_classification_uses_structure_not_asset_names() {
        for (flags, extra, expected) in [
            ("%DIFFUSE_MAP%PARALLAX_OCCLUSION_MAPPING", "", "relief_only"),
            ("%DIFFUSE_MAP", "", "color_sheet"),
            (
                "%STENCIL_MAP",
                r#"<Texture Map="TexSlot7" File="$TintPaletteDecal"/>"#,
                "stencil_palette",
            ),
            ("", "", "unknown"),
            (
                "%DIFFUSE_MAP%PARALLAX_OCCLUSION_MAPPING",
                r#"<Texture Map="TexSlot6" File="synthetic.tif"/>"#,
                "color_sheet",
            ),
        ] {
            let m = material(&format!(
                r#"<Material Shader="MeshDecal" StringGenMask="{flags}"><Textures><Texture Map="TexSlot1" File="arbitrary.tif"/>{extra}</Textures><PublicParams PomHeightBias="0.25" StencilOpacity="0.75" StencilDiffuseColor="0.25,0.5,0.75"/></Material>"#
            ));
            let d = decal(&m).unwrap();
            assert_eq!(d["kind"], expected);
            assert_eq!(d["slots"]["coverage"], "TexSlot1");
            assert_eq!(d["public_params"]["PomHeightBias"], json!(0.25));
            assert_eq!(
                d["public_params"]["StencilDiffuseColor"],
                json!([0.25, 0.5, 0.75])
            );
        }
        assert!(decal(&material(r#"<Material Shader="Illum"/>"#)).is_none());
    }
}
