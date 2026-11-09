//! Curated emissive and MeshDecal sidecar semantics from authored shader data.
//! Slot references join the submaterial's texture_slots array; no texture-name heuristics.
use crate::mtl::{ShaderFamily, SubMaterial, TextureSemanticRole};
use serde_json::{Value, json};

pub(crate) fn emissive(material: &SubMaterial) -> Value {
    let emittance = material.texture_slots.iter().find(|slot| {
        slot.slot.eq_ignore_ascii_case("Emittance")
            || slot.slot.eq_ignore_ascii_case("Emissive")
            || (material.shader_family() == ShaderFamily::HardSurface
                && slot.slot.eq_ignore_ascii_case("TexSlot14"))
    });
    let factor = material.emissive_factor();
    let color = std::array::from_fn::<_, 3, _>(|i| factor[i] * material.diffuse[i]);
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
    json!({"color": color, "uses_base_texture": emittance.is_none(),
        "emittance_texture": emittance.map(|s| &s.slot),
        "alpha_glow_cutoff": cutoff, "glow": material.glow})
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
    #[test]
    fn curated_emission_retains_authored_scalars_and_map_selection() {
        let m = material(
            r#"<Material Shader="Illum" Emissive="0.5,1,0.25" Glow="0.5" Diffuse="0.5,0.25,1"><PublicParams AlphaGlowCutoff="0.4"/></Material>"#,
        );
        assert_eq!(emissive(&m)["color"], json!([0.125, 0.125, 0.125]));
        assert_eq!(emissive(&m)["uses_base_texture"], true);
        assert_eq!(emissive(&m)["alpha_glow_cutoff"], json!(0.4));
        let m = material(
            r#"<Material Shader="HardSurface"><Textures><Texture Map="TexSlot14" File="synthetic.tif"/></Textures></Material>"#,
        );
        assert_eq!(emissive(&m)["color"], json!([0.0, 0.0, 0.0]));
        assert_eq!(emissive(&m)["emittance_texture"], "TexSlot14");
        assert_eq!(emissive(&m)["uses_base_texture"], false);
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
