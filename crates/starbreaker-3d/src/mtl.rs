use crate::error::Error;
use crate::ivo::material::MaterialName;
use starbreaker_chunks::ChunkFile;

/// Default paint palette colors for a ship entity.
/// Queried from DataCore TintPaletteTree via the entity's TintPaletteRef.
#[derive(Debug, Clone, Default)]
pub struct TintPaletteFinishEntry {
    pub specular: Option<[f32; 3]>,
    pub glossiness: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct TintPaletteFinish {
    pub primary: TintPaletteFinishEntry,
    pub secondary: TintPaletteFinishEntry,
    pub tertiary: TintPaletteFinishEntry,
    pub glass: TintPaletteFinishEntry,
}

#[derive(Debug, Clone, Default)]
pub struct TintPalette {
    pub source_name: Option<String>,
    pub primary: [f32; 3],   // entryA — main hull
    pub secondary: [f32; 3], // entryB — secondary panels
    pub tertiary: [f32; 3],  // entryC — accent
    pub glass: [f32; 3],     // glass tint
    pub finish: TintPaletteFinish,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaintOverrideInfo {
    pub paint_item_name: String,
    pub subgeometry_tag: String,
    pub subgeometry_index: usize,
    pub material_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredAttribute {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredBlock {
    pub tag: String,
    pub attributes: Vec<AuthoredAttribute>,
    pub children: Vec<AuthoredBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredTexture {
    pub slot: String,
    pub path: String,
    pub is_virtual: bool,
    pub attributes: Vec<AuthoredAttribute>,
    pub child_blocks: Vec<AuthoredBlock>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterialSetAuthoredData {
    pub attributes: Vec<AuthoredAttribute>,
    pub public_params: Vec<PublicParam>,
    pub child_blocks: Vec<AuthoredBlock>,
}

#[derive(Debug, Clone)]
pub struct MtlFile {
    pub materials: Vec<SubMaterial>,
    /// P4k source path of this .mtl file (e.g. `Data\Objects\Ships\RSI\aurora_mk2\rsi_aurora_mk2_int.mtl`).
    /// Used for CGF-Converter compatible material naming.
    pub source_path: Option<String>,
    /// Resolved paint override selector metadata when this material set came from a SubGeometry match.
    pub paint_override: Option<PaintOverrideInfo>,
    /// Raw authored material-set metadata from the root `<Material>` node when this file contains submaterials.
    pub material_set: MaterialSetAuthoredData,
}

#[derive(Debug, Clone)]
pub struct SubMaterial {
    pub name: String,
    pub shader: String,
    pub diffuse: [f32; 3],
    pub opacity: f32,
    pub alpha_test: f32,
    pub string_gen_mask: String,
    pub is_nodraw: bool,
    /// CryEngine specular color (RGB). Determines reflectance/metalness in PBR:
    /// - Dielectrics: specular ≈ [0.04, 0.04, 0.04] (low reflectance)
    /// - Metals: specular = metal color (high reflectance, colored)
    /// Maps to `PM_MatSpecularColor` shader parameter.
    pub specular: [f32; 3],
    /// CryEngine shininess (0-255). Maps directly to smoothness.
    /// GBuffer stores "Gloss/smoothness", EFTT_SMOOTHNESS
    /// uses `_ddna` suffix (alpha channel of normal map for per-pixel smoothness).
    pub shininess: f32,
    /// CryEngine emissive color (RGB). Default [0,0,0] = no glow.
    /// Maps to `PM_MatEmissiveColor` shader parameter.
    pub emissive: [f32; 3],
    /// CryEngine glow factor (0-1). Multiplied with emissive for intensity.
    pub glow: f32,
    pub surface_type: String,
    pub diffuse_tex: Option<String>,
    /// Normal map texture path (TexSlot2, typically `_ddna.dds`).
    /// The alpha channel contains per-pixel smoothness (EFTT_SMOOTHNESS).
    pub normal_tex: Option<String>,
    /// Parsed MatLayers (LayerBlend/HardSurface shaders). Each layer references
    /// an external .mtl with its own textures, tint color, and palette channel.
    pub layers: Vec<MatLayer>,
    /// Palette tint channel from first layer with PaletteTint > 0: 1=entryA, 2=entryB, 3=entryC, 0=none.
    pub palette_tint: u8,
    /// All parsed texture bindings, including non-exported slots and virtual inputs.
    pub texture_slots: Vec<TextureSlotBinding>,
    /// PublicParams preserved as authored name/value pairs.
    pub public_params: Vec<PublicParam>,
    /// Raw authored submaterial attributes preserved as exact name/value pairs.
    pub authored_attributes: Vec<AuthoredAttribute>,
    /// Raw authored texture nodes, including nested `TexMod` blocks.
    pub authored_textures: Vec<AuthoredTexture>,
    /// Raw authored non-texture child blocks such as `VertexDeform`.
    pub authored_child_blocks: Vec<AuthoredBlock>,
}

/// A single layer from a LayerBlend/HardSurface material's MatLayers section.
#[derive(Debug, Clone, PartialEq)]
pub struct MatLayerSnapshot {
    pub shader: String,
    pub diffuse: [f32; 3],
    pub specular: [f32; 3],
    pub shininess: f32,
    pub wear_specular_color: Option<[f32; 3]>,
    pub wear_glossiness: Option<f32>,
    pub surface_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLayerMaterial {
    pub name: String,
    pub shader: String,
    pub shader_family: String,
    pub authored_attributes: Vec<AuthoredAttribute>,
    pub public_params: Vec<PublicParam>,
    pub authored_child_blocks: Vec<AuthoredBlock>,
}

#[derive(Debug, Clone)]
pub struct MatLayer {
    /// Authored layer name such as `Primary` or `Wear`.
    pub name: String,
    /// Path to the layer's .mtl file (e.g., `libs/materials/metal/steel_bare_01.mtl`).
    pub path: String,
    /// Authored submaterial selector inside the layer file, when present.
    pub sub_material: String,
    /// TintColor applied to this layer's diffuse texture. Default [1,1,1].
    pub tint_color: [f32; 3],
    /// WearTint authored on the layer entry. Default [1,1,1].
    pub wear_tint: [f32; 3],
    /// Palette channel: 0=none, 1=primary, 2=secondary, 3=tertiary.
    pub palette_tint: u8,
    /// Gloss multiplier authored on the layer entry. Default 1.0.
    pub gloss_mult: f32,
    /// Wear gloss multiplier authored on the layer entry. Default 1.0.
    pub wear_gloss: f32,
    /// UV tiling factor for the layer's textures. Default 1.0.
    pub uv_tiling: f32,
    /// Authored height bias for layer blending. Default 0.0.
    pub height_bias: f32,
    /// Authored height scale for layer blending. Default 1.0.
    pub height_scale: f32,
    /// Raw authored layer attributes preserved exactly as they appear in the XML.
    pub authored_attributes: Vec<AuthoredAttribute>,
    /// Raw authored non-scalar child blocks preserved for future reconstruction.
    pub authored_child_blocks: Vec<AuthoredBlock>,
    /// Snapshot of the referenced layer material's scalar response data.
    pub snapshot: Option<MatLayerSnapshot>,
    /// Resolved authored metadata from the selected layer-file submaterial.
    pub resolved_material: Option<ResolvedLayerMaterial>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureSlotBinding {
    pub slot: String,
    pub path: String,
    pub is_virtual: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicParam {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderFamily {
    HardSurface,
    Layer,
    LayerBlendV2,
    Illum,
    MeshDecal,
    GlassPbr,
    DisplayScreen,
    Monitor,
    UiPlane,
    HumanSkinV2,
    Eye,
    HairPbr,
    Organic,
    Hologram,
    HologramCig,
    ShieldHolo,
    UiMesh,
    NoDraw,
    Unknown,
}

impl ShaderFamily {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShaderFamily::HardSurface => "HardSurface",
            ShaderFamily::Layer => "Layer",
            ShaderFamily::LayerBlendV2 => "LayerBlend_V2",
            ShaderFamily::Illum => "Illum",
            ShaderFamily::MeshDecal => "MeshDecal",
            ShaderFamily::GlassPbr => "GlassPBR",
            ShaderFamily::DisplayScreen => "DisplayScreen",
            ShaderFamily::Monitor => "Monitor",
            ShaderFamily::UiPlane => "UIPlane",
            ShaderFamily::HumanSkinV2 => "HumanSkin_V2",
            ShaderFamily::Eye => "Eye",
            ShaderFamily::HairPbr => "HairPBR",
            ShaderFamily::Organic => "Organic",
            ShaderFamily::Hologram => "Hologram",
            ShaderFamily::HologramCig => "HologramCIG",
            ShaderFamily::ShieldHolo => "Shield_Holo",
            ShaderFamily::UiMesh => "UIMesh",
            ShaderFamily::NoDraw => "NoDraw",
            ShaderFamily::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedStringGenMask {
    pub tokens: Vec<String>,
    pub has_decal: bool,
    pub has_parallax_occlusion_mapping: bool,
    pub has_stencil_map: bool,
    pub has_iridescence: bool,
    pub has_vertex_colors: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureSemanticRole {
    BaseColor,
    AlternateBaseColor,
    NormalGloss,
    SpecularSupport,
    Stencil,
    Breakup,
    Height,
    Iridescence,
    BlendMask,
    WearMask,
    HalControl,
    ScreenMask,
    ScreenPixelLayout,
    Dirt,
    WearGloss,
    Opacity,
    Subsurface,
    PatternMask,
    DecalSheet,
    TintPaletteDecal,
    RenderToTexture,
    Unknown,
}

impl TextureSemanticRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            TextureSemanticRole::BaseColor => "base_color",
            TextureSemanticRole::AlternateBaseColor => "alternate_base_color",
            TextureSemanticRole::NormalGloss => "normal_gloss",
            TextureSemanticRole::SpecularSupport => "specular_support",
            TextureSemanticRole::Stencil => "stencil",
            TextureSemanticRole::Breakup => "breakup",
            TextureSemanticRole::Height => "height",
            TextureSemanticRole::Iridescence => "iridescence",
            TextureSemanticRole::BlendMask => "blend_mask",
            TextureSemanticRole::WearMask => "wear_mask",
            TextureSemanticRole::HalControl => "hal_control",
            TextureSemanticRole::ScreenMask => "screen_mask",
            TextureSemanticRole::ScreenPixelLayout => "screen_pixel_layout",
            TextureSemanticRole::Dirt => "dirt",
            TextureSemanticRole::WearGloss => "wear_gloss",
            TextureSemanticRole::Opacity => "opacity",
            TextureSemanticRole::Subsurface => "subsurface",
            TextureSemanticRole::PatternMask => "pattern_mask",
            TextureSemanticRole::DecalSheet => "decal_sheet",
            TextureSemanticRole::TintPaletteDecal => "tint_palette_decal",
            TextureSemanticRole::RenderToTexture => "render_to_texture",
            TextureSemanticRole::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticTextureBinding {
    pub slot: String,
    pub path: String,
    pub is_virtual: bool,
    pub role: TextureSemanticRole,
    pub authored_attributes: Vec<AuthoredAttribute>,
    pub authored_child_blocks: Vec<AuthoredBlock>,
}

/// How a material's alpha should be handled in glTF.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaConfig {
    Opaque,
    Mask(f32),
    Blend,
}

impl SubMaterial {
    pub fn resolved_layer_material(&self) -> ResolvedLayerMaterial {
        ResolvedLayerMaterial {
            name: self.name.clone(),
            shader: self.shader.clone(),
            shader_family: self.shader_family().as_str().to_string(),
            authored_attributes: self.authored_attributes.clone(),
            public_params: self.public_params.clone(),
            authored_child_blocks: self.authored_child_blocks.clone(),
        }
    }

    pub fn public_param(&self, name: &str) -> Option<&str> {
        self.public_params
            .iter()
            .find(|param| param.name.eq_ignore_ascii_case(name))
            .map(|param| param.value.as_str())
    }

    pub fn public_param_f32(&self, names: &[&str]) -> Option<f32> {
        names.iter().find_map(|name| {
            self.public_param(name)
                .and_then(|value| value.parse::<f32>().ok())
        })
    }

    pub fn public_param_rgb(&self, names: &[&str]) -> Option<[f32; 3]> {
        names.iter().find_map(|name| self.public_param(name).map(parse_rgb))
    }

    pub fn first_texture_path_for_role(&self, role: TextureSemanticRole) -> Option<String> {
        self.semantic_texture_slots()
            .into_iter()
            .find(|binding| binding.role == role && !binding.is_virtual)
            .map(|binding| binding.path)
    }

    pub fn has_virtual_input(&self, input: &str) -> bool {
        self.texture_slots.iter().any(|binding| {
            binding.is_virtual && binding.path.eq_ignore_ascii_case(input)
        })
    }

    pub fn primary_uv_tiling(&self) -> Option<f32> {
        self.layers.first().and_then(|layer| {
            if (layer.uv_tiling - 1.0).abs() > f32::EPSILON {
                Some(layer.uv_tiling)
            } else {
                None
            }
        })
    }

    pub fn resolved_palette_color(
        &self,
        palette: Option<&TintPalette>,
    ) -> Option<[f32; 3]> {
        let palette = palette?;
        match self.palette_tint {
            1 => Some(palette.primary),
            2 => Some(palette.secondary),
            3 => Some(palette.tertiary),
            _ if self.is_glass() => Some(palette.glass),
            _ => None,
        }
    }

    pub fn resolved_layer_color(
        &self,
        layer: &MatLayer,
        palette: Option<&TintPalette>,
    ) -> [f32; 3] {
        if let Some(palette) = palette {
            match layer.palette_tint {
                1 => return palette.primary,
                2 => return palette.secondary,
                3 => return palette.tertiary,
                _ => {}
            }
        }
        layer.tint_color
    }

    pub fn shader_family(&self) -> ShaderFamily {
        match self.shader.as_str() {
            "HardSurface" => ShaderFamily::HardSurface,
            "Layer" => ShaderFamily::Layer,
            "LayerBlend_V2" => ShaderFamily::LayerBlendV2,
            "Illum" => ShaderFamily::Illum,
            "MeshDecal" => ShaderFamily::MeshDecal,
            "GlassPBR" => ShaderFamily::GlassPbr,
            "DisplayScreen" => ShaderFamily::DisplayScreen,
            "Monitor" => ShaderFamily::Monitor,
            "UIPlane" => ShaderFamily::UiPlane,
            "HumanSkin_V2" => ShaderFamily::HumanSkinV2,
            "Eye" => ShaderFamily::Eye,
            "HairPBR" => ShaderFamily::HairPbr,
            "Organic" => ShaderFamily::Organic,
            "Hologram" => ShaderFamily::Hologram,
            "HologramCIG" => ShaderFamily::HologramCig,
            "Shield_Holo" => ShaderFamily::ShieldHolo,
            "UIMesh" => ShaderFamily::UiMesh,
            "NoDraw" if self.is_nodraw => ShaderFamily::NoDraw,
            "NoDraw" => ShaderFamily::NoDraw,
            _ if self.is_nodraw => ShaderFamily::NoDraw,
            _ => ShaderFamily::Unknown,
        }
    }

    pub fn decoded_string_gen_mask(&self) -> DecodedStringGenMask {
        let tokens: Vec<String> = self
            .string_gen_mask
            .split('%')
            .filter(|token| !token.is_empty())
            .map(|token| token.to_string())
            .collect();
        let has_token = |needle: &str| tokens.iter().any(|token| token.eq_ignore_ascii_case(needle));

        DecodedStringGenMask {
            has_decal: has_token("DECAL"),
            has_parallax_occlusion_mapping: has_token("PARALLAX_OCCLUSION_MAPPING"),
            has_stencil_map: has_token("STENCIL_MAP"),
            has_iridescence: has_token("IRIDESCENCE"),
            has_vertex_colors: has_token("VERTCOLORS"),
            tokens,
        }
    }

    pub fn semantic_texture_slots(&self) -> Vec<SemanticTextureBinding> {
        let flags = self.decoded_string_gen_mask();

        self.texture_slots
            .iter()
            .map(|binding| {
                let authored = self.authored_texture(binding);
                SemanticTextureBinding {
                    slot: binding.slot.clone(),
                    path: binding.path.clone(),
                    is_virtual: binding.is_virtual,
                    role: classify_texture_role(self, &flags, binding),
                    authored_attributes: authored
                        .map(|texture| texture.attributes.clone())
                        .unwrap_or_default(),
                    authored_child_blocks: authored
                        .map(|texture| texture.child_blocks.clone())
                        .unwrap_or_default(),
                }
            })
            .collect()
    }

    fn authored_texture(&self, binding: &TextureSlotBinding) -> Option<&AuthoredTexture> {
        self.authored_textures.iter().find(|texture| {
            texture.slot == binding.slot
                && texture.path == binding.path
                && texture.is_virtual == binding.is_virtual
        })
    }

    /// Determine alpha config from parsed .mtl attributes.
    /// Priority: NoDraw → StringGenMask decal flags → AlphaTest → Opacity → Opaque.
    pub fn alpha_config(&self) -> AlphaConfig {
        if self.is_nodraw {
            return AlphaConfig::Blend;
        }
        let mask = &self.string_gen_mask;
        // Decals use alpha blending for layering on top of hull geometry.
        // MeshDecal shader is always a decal overlay, even without %DECAL flag.
        // Note: STENCIL_MAP on non-MeshDecal shaders (e.g. HardSurface camo paints)
        // is for the stencil pattern system, NOT alpha blending — don't treat as decal.
        if mask.contains("%DECAL") || self.shader == "MeshDecal" {
            return AlphaConfig::Blend;
        }
        if self.alpha_test > 0.0 {
            return AlphaConfig::Mask(self.alpha_test);
        }
        if self.opacity < 1.0 {
            return AlphaConfig::Blend;
        }
        AlphaConfig::Opaque
    }
    /// Whether this material should be hidden.
    pub fn should_hide(&self) -> bool {
        self.is_nodraw || self.opacity <= f32::EPSILON
    }

    /// Whether this material is a decal overlay (needs alpha texture to look correct).
    pub fn is_decal(&self) -> bool {
        let mask = &self.string_gen_mask;
        mask.contains("%DECAL") || self.shader == "MeshDecal"
    }

    /// Whether this material uses a glass shader (GlassPBR).
    pub fn is_glass(&self) -> bool {
        self.shader == "GlassPBR"
    }

    /// glTF roughness factor derived from CryEngine Shininess.
    ///
    /// CryEngine Shininess (0-255) = smoothness. The GBuffer stores
    /// "Gloss/smoothness" and EFTT_SMOOTHNESS maps to the `_ddna` alpha channel.
    /// Roughness = 1.0 - smoothness, where smoothness = shininess / 255.
    ///
    /// HardSurface, LayerBlend, and Illum shaders typically set Shininess=255 as a
    /// placeholder because they derive per-pixel smoothness from the `_ddna` alpha
    /// channel. Without per-pixel smoothness texture data, the literal 0.0 roughness
    /// creates an unrealistic mirror finish. Use a default of 0.5 for these shaders.
    pub fn roughness(&self) -> f32 {
        let smoothness = (self.shininess / 255.0).clamp(0.0, 1.0);
        let roughness = 1.0 - smoothness;
        // Shaders that use per-pixel smoothness set Shininess=255 as a placeholder.
        // Without the _ddna texture, this produces roughness=0 (mirror). Use a
        // reasonable default instead.
        if roughness == 0.0 && self.uses_per_pixel_smoothness() {
            0.5
        } else {
            roughness
        }
    }

    /// Whether this material's shader derives smoothness from a texture rather than
    /// the scalar Shininess value.
    fn uses_per_pixel_smoothness(&self) -> bool {
        let s = self.shader.to_lowercase();
        s.contains("hardsurface")
            || s.contains("layerblend")
            || s == "illum"
            || s == "glasspbr"
    }

    /// glTF metallic factor derived from material name.
    ///
    /// CryEngine uses a specular workflow where the Specular color parameter serves
    /// multiple purposes depending on the shader: it can be actual F0 reflectance
    /// (Illum), a blend/multiply factor (HardSurface, LayerBlend_V2), or a default
    /// value of [1,1,1]. There is no reliable way to convert specular color to the
    /// PBR metallic/dielectric distinction without the full shader graph.
    ///
    /// Conservative heuristic: only classify as metallic when the material name
    /// unambiguously indicates bare/unpainted metal. Everything else is dielectric.
    pub fn metallic(&self) -> f32 {
        let name = self.name.to_lowercase();
        if name.contains("chrome")
            || name.contains("raw_metal")
            || name.contains("raw_iron")
            || name == "tile_metal_bare"
        {
            1.0
        } else {
            0.0
        }
    }

    /// For metallic materials, the base color should come from the specular color
    /// (which IS the metal color in CryEngine's specular workflow).
    pub fn metal_base_color(&self) -> Option<[f32; 3]> {
        if self.metallic() > 0.5 {
            Some(self.specular)
        } else {
            None
        }
    }

    /// Emissive factor for glTF.
    ///
    /// CryEngine: EmissiveColor is the emission color, GlowAmount is the intensity
    /// multiplier. If EmissiveColor is non-zero, the material intends to emit light.
    /// GlowAmount defaults to 0 when absent from the MTL, but materials that set
    /// EmissiveColor without explicit GlowAmount still expect to glow (at full intensity).
    pub fn emissive_factor(&self) -> [f32; 3] {
        let e = &self.emissive;
        if e[0] > 0.0 || e[1] > 0.0 || e[2] > 0.0 {
            let g = if self.glow > 0.0 { self.glow } else { 1.0 };
            [e[0] * g, e[1] * g, e[2] * g]
        } else {
            [0.0, 0.0, 0.0]
        }
    }

    pub fn is_double_sided(&self) -> bool {
        let mask = &self.string_gen_mask;
        mask.contains("%DECAL") || self.shader == "MeshDecal"
    }
}

pub fn resolve_layer_submaterial<'a>(
    materials: &'a MtlFile,
    selector: &str,
) -> Option<&'a SubMaterial> {
    let selector = selector.trim();
    if selector.is_empty() {
        return materials.materials.first();
    }

    if let Some(material) = materials
        .materials
        .iter()
        .find(|material| material.name.eq_ignore_ascii_case(selector))
    {
        return Some(material);
    }

    if let Ok(index) = selector.parse::<usize>() {
        if let Some(material) = materials.materials.get(index) {
            return Some(material);
        }
    }

    materials.materials.first()
}

pub fn parse_mtl(data: &[u8]) -> Result<MtlFile, Error> {
    let xml = starbreaker_cryxml::from_bytes(data)?;
    let root = xml.root();

    let sub_materials_node = xml
        .node_children(root)
        .find(|child| xml.node_tag(child) == "SubMaterials");

    let material_set = if sub_materials_node.is_some() {
        let public_params = xml
            .node_children(root)
            .filter(|child| xml.node_tag(child) == "PublicParams")
            .flat_map(|node| parse_public_params(&xml, node))
            .collect::<Vec<_>>();
        let child_blocks = xml
            .node_children(root)
            .filter(|child| {
                let tag = xml.node_tag(child);
                tag != "SubMaterials" && tag != "PublicParams"
            })
            .map(|child| parse_authored_block(&xml, child))
            .collect::<Vec<_>>();
        MaterialSetAuthoredData {
            attributes: collect_authored_attributes(&xml, root),
            public_params,
            child_blocks,
        }
    } else {
        MaterialSetAuthoredData::default()
    };

    let materials = if let Some(sub_node) = sub_materials_node {
        xml.node_children(sub_node)
            .filter(|child| xml.node_tag(child) == "Material")
            .map(|mat_node| parse_sub_material(&xml, mat_node))
            .collect()
    } else {
        vec![parse_sub_material(&xml, root)]
    };

    Ok(MtlFile {
        materials,
        source_path: None,
        paint_override: None,
        material_set,
    })
}

fn collect_authored_attributes(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
) -> Vec<AuthoredAttribute> {
    xml.node_attributes(node)
        .map(|(name, value)| AuthoredAttribute {
            name: name.to_string(),
            value: value.to_string(),
        })
        .collect()
}

fn parse_public_params(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
) -> Vec<PublicParam> {
    xml.node_attributes(node)
        .map(|(name, value)| PublicParam {
            name: name.to_string(),
            value: value.to_string(),
        })
        .collect()
}

fn parse_authored_block(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
) -> AuthoredBlock {
    AuthoredBlock {
        tag: xml.node_tag(node).to_string(),
        attributes: collect_authored_attributes(xml, node),
        children: xml
            .node_children(node)
            .map(|child| parse_authored_block(xml, child))
            .collect(),
    }
}

fn parse_sub_material(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
) -> SubMaterial {
    let authored_attributes = collect_authored_attributes(xml, node);
    let mut name = String::new();
    let mut shader = String::new();
    let mut diffuse = [1.0f32; 3];
    let mut specular = [0.04f32; 3];
    let mut shininess = 128.0f32;
    let mut surface_type = String::new();
    let mut opacity = 1.0f32;
    let mut alpha_test = 0.0f32;
    let mut emissive = [0.0f32; 3];
    let mut glow = 0.0f32;
    let mut string_gen_mask = String::new();
    let mut is_nodraw = false;

    for (key, val) in xml.node_attributes(node) {
        match key {
            "Name" => name = val.to_string(),
            "Shader" => {
                shader = val.to_string();
                if val == "NoDraw" {
                    is_nodraw = true;
                }
            }
            "Diffuse" => diffuse = parse_rgb(val),
            "Specular" => specular = parse_rgb(val),
            "Shininess" => shininess = val.parse().unwrap_or(128.0),
            "Emissive" => emissive = parse_rgb(val),
            "Glow" | "GlowAmount" => glow = val.parse().unwrap_or(0.0),
            "SurfaceType" => surface_type = val.to_string(),
            "Opacity" => opacity = val.parse().unwrap_or(1.0),
            "AlphaTest" => alpha_test = val.parse().unwrap_or(0.0),
            "StringGenMask" => string_gen_mask = val.to_string(),
            _ => {}
        }
    }

    let mut diffuse_tex = None;
    let mut normal_tex = None;
    let mut layers = Vec::new();
    let mut palette_tint: u8 = 0;
    let mut texture_slots = Vec::new();
    let mut public_params = Vec::new();
    let mut authored_textures = Vec::new();
    let mut authored_child_blocks = Vec::new();

    for child in xml.node_children(node) {
        match xml.node_tag(child) {
            "Textures" => {
                for tex in xml.node_children(child) {
                    if xml.node_tag(tex) != "Texture" {
                        continue;
                    }
                    let attributes = collect_authored_attributes(xml, tex);
                    let child_blocks = xml
                        .node_children(tex)
                        .map(|child| parse_authored_block(xml, child))
                        .collect::<Vec<_>>();
                    let mut slot = String::new();
                    let mut file_path = String::new();
                    for attr in &attributes {
                        match attr.name.as_str() {
                            "Map" => slot = attr.value.clone(),
                            "File" => file_path = attr.value.clone(),
                            _ => {}
                        }
                    }
                    let is_virtual = file_path.starts_with('$');
                    if !slot.is_empty() || !file_path.is_empty() {
                        authored_textures.push(AuthoredTexture {
                            slot: slot.clone(),
                            path: file_path.clone(),
                            is_virtual,
                            attributes,
                            child_blocks,
                        });
                    }
                    if !file_path.is_empty() {
                        texture_slots.push(TextureSlotBinding {
                            slot: slot.clone(),
                            path: file_path.clone(),
                            is_virtual,
                        });
                        if is_virtual {
                            continue;
                        }
                        match slot.as_str() {
                            "TexSlot1" => diffuse_tex = Some(file_path),
                            "TexSlot2" => normal_tex = Some(file_path),
                            _ => {}
                        }
                    }
                }
            }
            "MatLayers" => {
                for layer in xml.node_children(child) {
                    let authored_attributes = collect_authored_attributes(xml, layer);
                    let authored_child_blocks = xml
                        .node_children(layer)
                        .map(|child| parse_authored_block(xml, child))
                        .collect::<Vec<_>>();
                    let mut lname = String::new();
                    let mut lpath = String::new();
                    let mut lsub_material = String::new();
                    let mut ltint = [1.0f32; 3];
                    let mut lwear_tint = [1.0f32; 3];
                    let mut lpalette: u8 = 0;
                    let mut lgloss_mult: f32 = 1.0;
                    let mut lwear_gloss: f32 = 1.0;
                    let mut luv_tiling: f32 = 1.0;
                    let mut lheight_bias: f32 = 0.0;
                    let mut lheight_scale: f32 = 1.0;
                    for (key, val) in xml.node_attributes(layer) {
                        match key {
                            "Name" => lname = val.to_string(),
                            "Path" if !val.is_empty() => lpath = val.to_string(),
                            "Submtl" => lsub_material = val.to_string(),
                            "TintColor" => ltint = parse_rgb(val),
                            "WearTint" => lwear_tint = parse_rgb(val),
                            "PaletteTint" => lpalette = val.parse().unwrap_or(0),
                            "GlossMult" => lgloss_mult = val.parse().unwrap_or(1.0),
                            "WearGloss" => lwear_gloss = val.parse().unwrap_or(1.0),
                            "UVTiling" => luv_tiling = val.parse().unwrap_or(1.0),
                            "HeightBias" => lheight_bias = val.parse().unwrap_or(0.0),
                            "HeightScale" => lheight_scale = val.parse().unwrap_or(1.0),
                            _ => {}
                        }
                    }
                    if !lpath.is_empty() {
                        layers.push(MatLayer {
                            name: lname,
                            path: lpath,
                            sub_material: lsub_material,
                            tint_color: ltint,
                            wear_tint: lwear_tint,
                            palette_tint: lpalette,
                            gloss_mult: lgloss_mult,
                            wear_gloss: lwear_gloss,
                            uv_tiling: luv_tiling,
                            height_bias: lheight_bias,
                            height_scale: lheight_scale,
                            authored_attributes,
                            authored_child_blocks,
                            snapshot: None,
                            resolved_material: None,
                        });
                    }
                    // First layer with PaletteTint > 0 determines the palette channel
                    if lpalette > 0 && palette_tint == 0 {
                        palette_tint = lpalette;
                    }
                }
            }
            "PublicParams" => {
                public_params.extend(parse_public_params(xml, child));
            }
            _ => authored_child_blocks.push(parse_authored_block(xml, child)),
        }
    }

    SubMaterial {
        name,
        shader,
        diffuse,
        specular,
        shininess,
        emissive,
        glow,
        surface_type,
        opacity,
        alpha_test,
        string_gen_mask,
        is_nodraw,
        diffuse_tex,
        normal_tex,
        layers,
        palette_tint,
        texture_slots,
        public_params,
        authored_attributes,
        authored_textures,
        authored_child_blocks,
    }
}

fn classify_texture_role(
    material: &SubMaterial,
    flags: &DecodedStringGenMask,
    binding: &TextureSlotBinding,
) -> TextureSemanticRole {
    let family = material.shader_family();
    if binding.is_virtual {
        if binding.path.eq_ignore_ascii_case("$TintPaletteDecal") {
            return TextureSemanticRole::TintPaletteDecal;
        }
        if binding.path.eq_ignore_ascii_case("$RenderToTexture") {
            return TextureSemanticRole::RenderToTexture;
        }
        return TextureSemanticRole::Unknown;
    }

    let path_lower = binding.path.to_lowercase();
    let has_name = |needles: &[&str]| needles.iter().any(|needle| path_lower.contains(needle));
    let has_public_param = |needles: &[&str]| {
        material.public_params.iter().any(|param| {
            let name = param.name.to_lowercase();
            needles.iter().any(|needle| name.contains(needle))
        })
    };
    let has_slot = |slot: &str| {
        material
            .texture_slots
            .iter()
            .any(|other| !other.is_virtual && other.slot.eq_ignore_ascii_case(slot))
    };
    let has_stencil_semantics = flags.has_stencil_map || has_public_param(&["stencil"]);
    let has_height_semantics = flags.has_parallax_occlusion_mapping
        || has_public_param(&["pom", "height", "displacement"]);
    let is_normal_like = has_name(&["ddn", "ddna", "normal", "bump"]);
    let is_breakup_like = has_name(&["breakup", "grime", "rust", "bleed", "dirt"]);
    let is_height_like = has_name(&["disp", "height", "pom"]);
    let is_iridescence_like = has_name(&["irid", "irides"]);
    let is_mask_like = has_name(&["mask", "opacity", "alpha"]);
    let is_decal_like = has_name(&["decal", "sticker", "emblem", "atlas"]);
    let is_pixel_layout_like = has_name(&["pixel", "crt", "grid"]);

    match family {
        ShaderFamily::Layer | ShaderFamily::LayerBlendV2 => match binding.slot.as_str() {
            "TexSlot1" => TextureSemanticRole::BaseColor,
            "TexSlot2" if !has_slot("TexSlot3") || is_normal_like => TextureSemanticRole::NormalGloss,
            "TexSlot3" if is_normal_like => TextureSemanticRole::NormalGloss,
            "TexSlot3" | "TexSlot9" if flags.has_decal || is_decal_like => TextureSemanticRole::DecalSheet,
            "TexSlot3" | "TexSlot9" => TextureSemanticRole::AlternateBaseColor,
            "TexSlot4" => TextureSemanticRole::SpecularSupport,
            "TexSlot5" if flags.has_iridescence || is_iridescence_like => TextureSemanticRole::Iridescence,
            "TexSlot8" if has_height_semantics || is_height_like => TextureSemanticRole::Height,
            "TexSlot11" => TextureSemanticRole::WearMask,
            "TexSlot12" => TextureSemanticRole::BlendMask,
            "TexSlot13" => TextureSemanticRole::HalControl,
            _ => TextureSemanticRole::Unknown,
        },
        ShaderFamily::MeshDecal => match binding.slot.as_str() {
            "TexSlot1" => TextureSemanticRole::BaseColor,
            "TexSlot3" if is_normal_like => TextureSemanticRole::NormalGloss,
            "TexSlot4" if has_height_semantics || is_height_like => TextureSemanticRole::Height,
            "TexSlot4" => TextureSemanticRole::SpecularSupport,
            "TexSlot5" if is_breakup_like => TextureSemanticRole::Breakup,
            "TexSlot7" if has_stencil_semantics || has_name(&["stencil"]) => TextureSemanticRole::Stencil,
            "TexSlot8" | "TexSlot9" if is_breakup_like => TextureSemanticRole::Breakup,
            "TexSlot8" if has_height_semantics || is_height_like => TextureSemanticRole::Height,
            "TexSlot9" if flags.has_decal || is_decal_like => TextureSemanticRole::DecalSheet,
            _ => TextureSemanticRole::Unknown,
        },
        ShaderFamily::DisplayScreen | ShaderFamily::UiPlane => match binding.slot.as_str() {
            "TexSlot1" => TextureSemanticRole::BaseColor,
            "TexSlot2" | "TexSlot3" if is_normal_like => TextureSemanticRole::NormalGloss,
            "TexSlot6" if is_mask_like || has_public_param(&["screen", "monitor", "wipe", "mask"]) => {
                TextureSemanticRole::ScreenMask
            }
            "TexSlot10" | "TexSlot16" if is_mask_like || has_name(&["crack", "wipe", "pattern"]) => {
                TextureSemanticRole::PatternMask
            }
            "TexSlot11" if is_breakup_like => TextureSemanticRole::Dirt,
            "TexSlot17" if is_pixel_layout_like => TextureSemanticRole::ScreenPixelLayout,
            _ => TextureSemanticRole::Unknown,
        },
        ShaderFamily::Monitor => match binding.slot.as_str() {
            "TexSlot1" => TextureSemanticRole::BaseColor,
            _ => TextureSemanticRole::Unknown,
        },
        ShaderFamily::Organic => match binding.slot.as_str() {
            "TexSlot1" => TextureSemanticRole::BlendMask,
            "TexSlot2" if is_normal_like || !has_slot("TexSlot3") => TextureSemanticRole::NormalGloss,
            "TexSlot3" => TextureSemanticRole::BaseColor,
            "TexSlot8" if has_height_semantics || is_height_like => TextureSemanticRole::Height,
            "TexSlot17" if is_mask_like => TextureSemanticRole::Opacity,
            "TexSlot17" => TextureSemanticRole::Subsurface,
            _ => TextureSemanticRole::Unknown,
        },
        ShaderFamily::GlassPbr => match binding.slot.as_str() {
            "TexSlot2" if is_normal_like || !is_mask_like => TextureSemanticRole::NormalGloss,
            "TexSlot6" => TextureSemanticRole::WearGloss,
            "TexSlot10" | "TexSlot13" | "TexSlot16" if is_mask_like || has_name(&["crack", "pattern"]) => {
                TextureSemanticRole::PatternMask
            }
            "TexSlot11" if is_breakup_like => TextureSemanticRole::Dirt,
            _ => TextureSemanticRole::Unknown,
        },
        ShaderFamily::HumanSkinV2 => match binding.slot.as_str() {
            "TexSlot1" => TextureSemanticRole::BaseColor,
            "TexSlot2" if is_normal_like || !is_mask_like => TextureSemanticRole::NormalGloss,
            "TexSlot4" => TextureSemanticRole::SpecularSupport,
            "TexSlot12" => TextureSemanticRole::Opacity,
            "TexSlot17" => TextureSemanticRole::Subsurface,
            _ => TextureSemanticRole::Unknown,
        },
        ShaderFamily::HairPbr => match binding.slot.as_str() {
            "TexSlot1" => TextureSemanticRole::Opacity,
            "TexSlot4" => TextureSemanticRole::PatternMask,
            _ => TextureSemanticRole::Unknown,
        },
        ShaderFamily::Eye => match binding.slot.as_str() {
            "TexSlot1" => TextureSemanticRole::BaseColor,
            "TexSlot2" if is_normal_like || !is_mask_like => TextureSemanticRole::NormalGloss,
            _ => TextureSemanticRole::Unknown,
        },
        _ => match binding.slot.as_str() {
            "TexSlot1" => TextureSemanticRole::BaseColor,
            "TexSlot2" if is_normal_like || !is_mask_like => TextureSemanticRole::NormalGloss,
            "TexSlot3" if is_normal_like => TextureSemanticRole::NormalGloss,
            "TexSlot3" | "TexSlot9" if flags.has_decal || is_decal_like => TextureSemanticRole::DecalSheet,
            "TexSlot3" | "TexSlot9" => TextureSemanticRole::AlternateBaseColor,
            "TexSlot4" => TextureSemanticRole::SpecularSupport,
            "TexSlot7" if has_stencil_semantics || has_name(&["stencil"]) => TextureSemanticRole::Stencil,
            "TexSlot8" if has_stencil_semantics && is_breakup_like => TextureSemanticRole::Breakup,
            "TexSlot8" if has_height_semantics || is_height_like => TextureSemanticRole::Height,
            "TexSlot10" if flags.has_iridescence || is_iridescence_like => TextureSemanticRole::Iridescence,
            "TexSlot11" if is_breakup_like => TextureSemanticRole::Dirt,
            "TexSlot12" if is_mask_like => TextureSemanticRole::BlendMask,
            "TexSlot17" if is_mask_like => TextureSemanticRole::Opacity,
            _ => TextureSemanticRole::Unknown,
        },
    }
}

fn parse_rgb(s: &str) -> [f32; 3] {
    let parts: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    if parts.len() >= 3 {
        [parts[0], parts[1], parts[2]]
    } else {
        [1.0, 1.0, 1.0]
    }
}

/// Extract the MtlName string from a .cgf/.skin metadata IVO file.
pub fn extract_mtl_name(data: &[u8]) -> Option<String> {
    let chunk_file = ChunkFile::from_bytes(data).ok()?;
    let ivo = match &chunk_file {
        ChunkFile::Ivo(ivo) => ivo,
        ChunkFile::CrCh(_) => return None,
    };

    ivo.chunks()
        .iter()
        .find(|c| c.chunk_type == starbreaker_chunks::known_types::ivo::MTL_NAME_IVO320)
        .and_then(|entry| MaterialName::read(ivo.chunk_data(entry)).ok())
        .map(|m| m.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_submaterial(shader: &str, string_gen_mask: &str) -> SubMaterial {
        SubMaterial {
            name: String::new(),
            shader: shader.to_string(),
            diffuse: [1.0, 1.0, 1.0],
            opacity: 1.0,
            alpha_test: 0.0,
            string_gen_mask: string_gen_mask.to_string(),
            is_nodraw: shader == "NoDraw",
            specular: [0.04, 0.04, 0.04],
            shininess: 128.0,
            emissive: [0.0, 0.0, 0.0],
            glow: 0.0,
            surface_type: String::new(),
            diffuse_tex: None,
            normal_tex: None,
            layers: Vec::new(),
            palette_tint: 0,
            texture_slots: Vec::new(),
            public_params: Vec::new(),
            authored_attributes: Vec::new(),
            authored_textures: Vec::new(),
            authored_child_blocks: Vec::new(),
        }
    }

    #[test]
    fn parse_rgb_valid_floats() {
        let result = parse_rgb("0.5,0.25,0.75");
        assert_eq!(result, [0.5, 0.25, 0.75]);
    }

    #[test]
    fn parse_rgb_integers() {
        let result = parse_rgb("1,0,1");
        assert_eq!(result, [1.0, 0.0, 1.0]);
    }

    #[test]
    fn parse_rgb_invalid_fallback() {
        let result = parse_rgb("not,valid,data");
        assert_eq!(result, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn parse_rgb_too_few_components() {
        let result = parse_rgb("0.5,0.25");
        assert_eq!(result, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn parse_rgb_extra_components_ignored() {
        let result = parse_rgb("0.1,0.2,0.3,0.4");
        assert_eq!(result, [0.1, 0.2, 0.3]);
    }

    #[test]
    fn parse_rgb_with_spaces() {
        let result = parse_rgb("0.1, 0.2, 0.3");
        assert_eq!(result, [0.1, 0.2, 0.3]);
    }

    #[test]
    fn shader_family_classifies_known_families() {
        assert_eq!(dummy_submaterial("HardSurface", "").shader_family(), ShaderFamily::HardSurface);
        assert_eq!(dummy_submaterial("HologramCIG", "").shader_family(), ShaderFamily::HologramCig);
        assert_eq!(dummy_submaterial("Monitor", "").shader_family(), ShaderFamily::Monitor);
        assert_eq!(dummy_submaterial("NoDraw", "").shader_family(), ShaderFamily::NoDraw);
        assert_eq!(dummy_submaterial("SomethingElse", "").shader_family(), ShaderFamily::Unknown);
    }

    #[test]
    fn resolve_layer_submaterial_prefers_named_selector() {
        let first = dummy_submaterial("Layer", "");
        let mut second = dummy_submaterial("Layer", "");
        second.name = "paint".into();

        let materials = MtlFile {
            materials: vec![first, second],
            source_path: None,
            paint_override: None,
            material_set: Default::default(),
        };

        let resolved = resolve_layer_submaterial(&materials, "paint").expect("layer material");
        assert_eq!(resolved.name, "paint");
    }

    #[test]
    fn resolve_layer_submaterial_falls_back_to_first_material() {
        let mut first = dummy_submaterial("Layer", "");
        first.name = "primary".into();
        let mut second = dummy_submaterial("Layer", "");
        second.name = "wear".into();

        let materials = MtlFile {
            materials: vec![first, second],
            source_path: None,
            paint_override: None,
            material_set: Default::default(),
        };

        let resolved = resolve_layer_submaterial(&materials, "missing").expect("fallback material");
        assert_eq!(resolved.name, "primary");
    }

    #[test]
    fn resolve_layer_submaterial_accepts_numeric_selector() {
        let mut first = dummy_submaterial("Layer", "");
        first.name = "primary".into();
        let mut second = dummy_submaterial("Layer", "");
        second.name = "paint".into();

        let materials = MtlFile {
            materials: vec![first, second],
            source_path: None,
            paint_override: None,
            material_set: Default::default(),
        };

        let resolved = resolve_layer_submaterial(&materials, "1").expect("numeric layer material");
        assert_eq!(resolved.name, "paint");
    }

    #[test]
    fn recursive_authored_blocks_preserve_nested_children() {
        let material_set_block = AuthoredBlock {
            tag: "VertexDeform".into(),
            attributes: vec![AuthoredAttribute {
                name: "DividerY".into(),
                value: "0.25".into(),
            }],
            children: vec![AuthoredBlock {
                tag: "WaveY".into(),
                attributes: vec![AuthoredAttribute {
                    name: "Amp".into(),
                    value: "0.5".into(),
                }],
                children: Vec::new(),
            }],
        };
        let texture_block = AuthoredBlock {
            tag: "TexMod".into(),
            attributes: vec![AuthoredAttribute {
                name: "TileU".into(),
                value: "2".into(),
            }],
            children: vec![AuthoredBlock {
                tag: "Oscillator".into(),
                attributes: vec![AuthoredAttribute {
                    name: "Rate".into(),
                    value: "0.1".into(),
                }],
                children: Vec::new(),
            }],
        };
        let submaterial_block = AuthoredBlock {
            tag: "VertexDeform".into(),
            attributes: vec![AuthoredAttribute {
                name: "DividerX".into(),
                value: "0.5".into(),
            }],
            children: vec![AuthoredBlock {
                tag: "WaveX".into(),
                attributes: vec![AuthoredAttribute {
                    name: "Amp".into(),
                    value: "0.25".into(),
                }],
                children: Vec::new(),
            }],
        };

        assert_eq!(material_set_block.children[0].tag, "WaveY");
        assert_eq!(submaterial_block.children[0].tag, "WaveX");
        assert_eq!(texture_block.children[0].tag, "Oscillator");
        assert_eq!(texture_block.children[0].attributes[0].name, "Rate");
    }

    #[test]
    fn resolved_layer_material_summary_preserves_authored_fields() {
        let mut material = dummy_submaterial("Layer", "");
        material.name = "paint".into();
        material.authored_attributes = vec![AuthoredAttribute {
            name: "MatTemplate".into(),
            value: "layer_shell".into(),
        }];
        material.public_params = vec![PublicParam {
            name: "WearGlossiness".into(),
            value: "0.91".into(),
        }];
        material.authored_child_blocks = vec![AuthoredBlock {
            tag: "VertexDeform".into(),
            attributes: vec![AuthoredAttribute {
                name: "DividerX".into(),
                value: "0.5".into(),
            }],
            children: Vec::new(),
        }];

        let resolved = material.resolved_layer_material();
        assert_eq!(resolved.name, "paint");
        assert_eq!(resolved.shader_family, "Layer");
        assert_eq!(resolved.authored_attributes[0].name, "MatTemplate");
        assert_eq!(resolved.public_params[0].name, "WearGlossiness");
        assert_eq!(resolved.authored_child_blocks[0].tag, "VertexDeform");
    }

    #[test]
    fn decoded_string_gen_mask_splits_tokens() {
        let decoded = dummy_submaterial(
            "HardSurface",
            "%VERTCOLORS%PARALLAX_OCCLUSION_MAPPING%IRIDESCENCE%STENCIL_MAP",
        )
        .decoded_string_gen_mask();

        assert_eq!(decoded.tokens.len(), 4);
        assert!(decoded.has_vertex_colors);
        assert!(decoded.has_parallax_occlusion_mapping);
        assert!(decoded.has_iridescence);
        assert!(decoded.has_stencil_map);
        assert!(!decoded.has_decal);
    }

    #[test]
    fn semantic_texture_slots_assign_roles() {
        let mut material = dummy_submaterial(
            "HardSurface",
            "%PARALLAX_OCCLUSION_MAPPING%IRIDESCENCE%STENCIL_MAP",
        );
        material.texture_slots = vec![
            TextureSlotBinding {
                slot: "TexSlot1".into(),
                path: "textures/base.tif".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot7".into(),
                path: "textures/stencil_mask.tif".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot8".into(),
                path: "textures/stencil_breakup.tif".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot10".into(),
                path: "textures/iridescence_ramp.tif".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot9".into(),
                path: "$RenderToTexture".into(),
                is_virtual: true,
            },
        ];

        let roles: Vec<TextureSemanticRole> = material
            .semantic_texture_slots()
            .into_iter()
            .map(|binding| binding.role)
            .collect();

        assert_eq!(roles[0], TextureSemanticRole::BaseColor);
        assert_eq!(roles[1], TextureSemanticRole::Stencil);
        assert_eq!(roles[2], TextureSemanticRole::Breakup);
        assert_eq!(roles[3], TextureSemanticRole::Iridescence);
        assert_eq!(roles[4], TextureSemanticRole::RenderToTexture);
    }

    #[test]
    fn layer_blend_semantic_slots_follow_family_patterns() {
        let mut material = dummy_submaterial("LayerBlend_V2", "%DECAL%IRIDESCENCE");
        material.texture_slots = vec![
            TextureSlotBinding {
                slot: "TexSlot3".into(),
                path: "textures/fabric_ddna.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot5".into(),
                path: "textures/fabric_iridescence_ramp.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot9".into(),
                path: "textures/helmet_decal_sheet.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot11".into(),
                path: "textures/fabric_wear_mask.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot12".into(),
                path: "textures/fabric_blend_mask.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot13".into(),
                path: "textures/fabric_hal_control.dds".into(),
                is_virtual: false,
            },
        ];

        let roles: Vec<TextureSemanticRole> = material
            .semantic_texture_slots()
            .into_iter()
            .map(|binding| binding.role)
            .collect();

        assert_eq!(roles[0], TextureSemanticRole::NormalGloss);
        assert_eq!(roles[1], TextureSemanticRole::Iridescence);
        assert_eq!(roles[2], TextureSemanticRole::DecalSheet);
        assert_eq!(roles[3], TextureSemanticRole::WearMask);
        assert_eq!(roles[4], TextureSemanticRole::BlendMask);
        assert_eq!(roles[5], TextureSemanticRole::HalControl);
    }

    #[test]
    fn mesh_decal_semantic_slots_capture_stencil_and_height_contracts() {
        let mut material = dummy_submaterial("MeshDecal", "%STENCIL_MAP%STENCIL_AS_STICKER%PARALLAX_OCCLUSION_MAPPING");
        material.texture_slots = vec![
            TextureSlotBinding {
                slot: "TexSlot4".into(),
                path: "textures/decal_height.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot7".into(),
                path: "$TintPaletteDecal".into(),
                is_virtual: true,
            },
            TextureSlotBinding {
                slot: "TexSlot8".into(),
                path: "textures/decal_grime_breakup.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot9".into(),
                path: "textures/decal_rust_bleed.dds".into(),
                is_virtual: false,
            },
        ];

        let roles: Vec<TextureSemanticRole> = material
            .semantic_texture_slots()
            .into_iter()
            .map(|binding| binding.role)
            .collect();

        assert_eq!(roles[0], TextureSemanticRole::Height);
        assert_eq!(roles[1], TextureSemanticRole::TintPaletteDecal);
        assert_eq!(roles[2], TextureSemanticRole::Breakup);
        assert_eq!(roles[3], TextureSemanticRole::Breakup);
    }

    #[test]
    fn display_screen_semantic_slots_capture_rtt_support_maps() {
        let mut material = dummy_submaterial("DisplayScreen", "");
        material.public_params = vec![PublicParam {
            name: "MonitorMaskStrength".into(),
            value: "1.0".into(),
        }];
        material.texture_slots = vec![
            TextureSlotBinding {
                slot: "TexSlot2".into(),
                path: "textures/glass_scratched_a_ddna.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot6".into(),
                path: "textures/glass_screen_mask.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot9".into(),
                path: "$RenderToTexture".into(),
                is_virtual: true,
            },
            TextureSlotBinding {
                slot: "TexSlot17".into(),
                path: "textures/pixel_layout_crt.dds".into(),
                is_virtual: false,
            },
        ];

        let roles: Vec<TextureSemanticRole> = material
            .semantic_texture_slots()
            .into_iter()
            .map(|binding| binding.role)
            .collect();

        assert_eq!(roles[0], TextureSemanticRole::NormalGloss);
        assert_eq!(roles[1], TextureSemanticRole::ScreenMask);
        assert_eq!(roles[2], TextureSemanticRole::RenderToTexture);
        assert_eq!(roles[3], TextureSemanticRole::ScreenPixelLayout);
    }

    #[test]
    fn monitor_semantic_slots_preserve_base_color_contract() {
        let mut material = dummy_submaterial("Monitor", "");
        material.texture_slots = vec![TextureSlotBinding {
            slot: "TexSlot1".into(),
            path: "textures/temp_displays_diff.tif".into(),
            is_virtual: false,
        }];

        let roles: Vec<TextureSemanticRole> = material
            .semantic_texture_slots()
            .into_iter()
            .map(|binding| binding.role)
            .collect();

        assert_eq!(roles, vec![TextureSemanticRole::BaseColor]);
    }

    #[test]
    fn organic_semantic_slots_do_not_assume_texslot1_is_base_color() {
        let mut material = dummy_submaterial("Organic", "%HEIGHT_BLEND");
        material.texture_slots = vec![
            TextureSlotBinding {
                slot: "TexSlot1".into(),
                path: "textures/ast_blend_mask.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot2".into(),
                path: "textures/ast_surface_ddna.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot3".into(),
                path: "textures/ast_visible_diffuse.dds".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot17".into(),
                path: "textures/meat_subsurface_helper.dds".into(),
                is_virtual: false,
            },
        ];

        let roles: Vec<TextureSemanticRole> = material
            .semantic_texture_slots()
            .into_iter()
            .map(|binding| binding.role)
            .collect();

        assert_eq!(roles[0], TextureSemanticRole::BlendMask);
        assert_eq!(roles[1], TextureSemanticRole::NormalGloss);
        assert_eq!(roles[2], TextureSemanticRole::BaseColor);
        assert_eq!(roles[3], TextureSemanticRole::Subsurface);
    }

    #[test]
    fn semantic_preservation_fields_are_available() {
        let mut material = dummy_submaterial("DisplayScreen", "%IRIDESCENCE%VERTCOLORS");
        material.diffuse_tex = Some("textures/base.tif".into());
        material.texture_slots = vec![
            TextureSlotBinding {
                slot: "TexSlot1".into(),
                path: "textures/base.tif".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot10".into(),
                path: "textures/iridescence_ramp.tif".into(),
                is_virtual: false,
            },
            TextureSlotBinding {
                slot: "TexSlot9".into(),
                path: "$RenderToTexture".into(),
                is_virtual: true,
            },
        ];
        material.public_params = vec![
            PublicParam {
                name: "DisplayScale".into(),
                value: "2.5".into(),
            },
            PublicParam {
                name: "ScreenOn".into(),
                value: "1".into(),
            },
        ];

        assert_eq!(material.texture_slots.len(), 3);
        assert_eq!(material.public_params.len(), 2);
        assert_eq!(material.diffuse_tex.as_deref(), Some("textures/base.tif"));
        assert!(material
            .texture_slots
            .iter()
            .any(|slot| slot.is_virtual && slot.path == "$RenderToTexture"));
    }

    #[test]
    fn semantic_texture_slots_preserve_authored_texture_metadata() {
        let mut material = dummy_submaterial("Illum", "");
        material.texture_slots = vec![TextureSlotBinding {
            slot: "TexSlot1".into(),
            path: "textures/base.tif".into(),
            is_virtual: false,
        }];
        material.authored_textures = vec![AuthoredTexture {
            slot: "TexSlot1".into(),
            path: "textures/base.tif".into(),
            is_virtual: false,
            attributes: vec![
                AuthoredAttribute {
                    name: "Map".into(),
                    value: "TexSlot1".into(),
                },
                AuthoredAttribute {
                    name: "Used".into(),
                    value: "1".into(),
                },
            ],
            child_blocks: vec![AuthoredBlock {
                tag: "TexMod".into(),
                attributes: vec![AuthoredAttribute {
                    name: "TileU".into(),
                    value: "2".into(),
                }],
                children: Vec::new(),
            }],
        }];

        let slots = material.semantic_texture_slots();

        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].authored_attributes[1].name, "Used");
        assert_eq!(slots[0].authored_attributes[1].value, "1");
        assert_eq!(slots[0].authored_child_blocks[0].tag, "TexMod");
        assert_eq!(slots[0].authored_child_blocks[0].attributes[0].name, "TileU");
        assert_eq!(slots[0].authored_child_blocks[0].attributes[0].value, "2");
    }
}
