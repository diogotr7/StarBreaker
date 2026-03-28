use crate::error::Error;
use crate::ivo::material::MaterialName;
use starbreaker_chunks::ChunkFile;

/// Default paint palette colors for a ship entity.
/// Queried from DataCore TintPaletteTree via the entity's TintPaletteRef.
#[derive(Debug, Clone, Default)]
pub struct TintPalette {
    pub primary: [f32; 3],   // entryA — main hull
    pub secondary: [f32; 3], // entryB — secondary panels
    pub tertiary: [f32; 3],  // entryC — accent
    pub glass: [f32; 3],     // glass tint
}

#[derive(Debug, Clone)]
pub struct MtlFile {
    pub materials: Vec<SubMaterial>,
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
}

/// A single layer from a LayerBlend/HardSurface material's MatLayers section.
#[derive(Debug, Clone)]
pub struct MatLayer {
    /// Path to the layer's .mtl file (e.g., `libs/materials/metal/steel_bare_01.mtl`).
    pub path: String,
    /// TintColor applied to this layer's diffuse texture. Default [1,1,1].
    pub tint_color: [f32; 3],
    /// Palette channel: 0=none, 1=primary, 2=secondary, 3=tertiary.
    pub palette_tint: u8,
    /// UV tiling factor for the layer's textures. Default 1.0.
    pub uv_tiling: f32,
}

/// How a material's alpha should be handled in glTF.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaConfig {
    Opaque,
    Mask(f32),
    Blend,
}

impl SubMaterial {
    /// Determine alpha config from parsed .mtl attributes.
    /// Priority: NoDraw → StringGenMask decal flags → AlphaTest → Opacity → Opaque.
    pub fn alpha_config(&self) -> AlphaConfig {
        if self.is_nodraw {
            return AlphaConfig::Blend;
        }
        let mask = &self.string_gen_mask;
        // Decals use alpha blending for layering on top of hull geometry.
        // MeshDecal shader is always a decal overlay, even without %DECAL flag.
        if mask.contains("%DECAL") || mask.contains("STENCIL_MAP") || self.shader == "MeshDecal" {
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
        if self.is_nodraw {
            return true;
        }
        let mask = &self.string_gen_mask;
        // POM decals modify surface normals in CryEngine — in glTF they render as
        // washed-out pink overlays. Hide any decal with POM.
        if mask.contains("%PARALLAX_OCCLUSION_MAPPING")
            && (self.is_decal() || mask.contains("%DECAL"))
        {
            return true;
        }
        // Decals without a diffuse texture AND without vertex colors have no alpha source —
        // they'd render as solid colored rectangles covering the hull.
        if self.is_decal() && self.diffuse_tex.is_none() && !mask.contains("%VERTCOLORS") {
            return true;
        }
        false
    }

    /// Whether this material is a decal overlay (needs alpha texture to look correct).
    pub fn is_decal(&self) -> bool {
        let mask = &self.string_gen_mask;
        mask.contains("%DECAL") || mask.contains("STENCIL_MAP") || self.shader == "MeshDecal"
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
    pub fn roughness(&self) -> f32 {
        let smoothness = (self.shininess / 255.0).clamp(0.0, 1.0);
        1.0 - smoothness
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
        mask.contains("%DECAL") || mask.contains("STENCIL_MAP") || self.shader == "MeshDecal"
    }
}

pub fn parse_mtl(data: &[u8]) -> Result<MtlFile, Error> {
    let xml = starbreaker_cryxml::from_bytes(data)?;
    let root = xml.root();

    let sub_materials_node = xml
        .node_children(root)
        .find(|child| xml.node_tag(child) == "SubMaterials");

    let materials = if let Some(sub_node) = sub_materials_node {
        xml.node_children(sub_node)
            .filter(|child| xml.node_tag(child) == "Material")
            .map(|mat_node| parse_sub_material(&xml, mat_node))
            .collect()
    } else {
        vec![parse_sub_material(&xml, root)]
    };

    Ok(MtlFile { materials })
}

fn parse_sub_material(
    xml: &starbreaker_cryxml::CryXml,
    node: &starbreaker_cryxml::CryXmlNode,
) -> SubMaterial {
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
            "GlowAmount" => glow = val.parse().unwrap_or(0.0),
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

    for child in xml.node_children(node) {
        match xml.node_tag(child) {
            "Textures" => {
                for tex in xml.node_children(child) {
                    if xml.node_tag(tex) != "Texture" {
                        continue;
                    }
                    let mut slot = "";
                    let mut file_path = None;
                    for (key, val) in xml.node_attributes(tex) {
                        match key {
                            "Map" => slot = val,
                            "File" => file_path = Some(val.to_string()),
                            _ => {}
                        }
                    }
                    if let Some(path) = file_path {
                        if path.starts_with('$') {
                            continue;
                        }
                        match slot {
                            "TexSlot1" => diffuse_tex = Some(path),
                            "TexSlot2" => normal_tex = Some(path),
                            _ => {}
                        }
                    }
                }
            }
            "MatLayers" => {
                for layer in xml.node_children(child) {
                    let mut lpath = String::new();
                    let mut ltint = [1.0f32; 3];
                    let mut lpalette: u8 = 0;
                    let mut luv_tiling: f32 = 1.0;
                    for (key, val) in xml.node_attributes(layer) {
                        match key {
                            "Path" if !val.is_empty() => lpath = val.to_string(),
                            "TintColor" => ltint = parse_rgb(val),
                            "PaletteTint" => lpalette = val.parse().unwrap_or(0),
                            "UVTiling" => luv_tiling = val.parse().unwrap_or(1.0),
                            _ => {}
                        }
                    }
                    if !lpath.is_empty() {
                        layers.push(MatLayer {
                            path: lpath,
                            tint_color: ltint,
                            palette_tint: lpalette,
                            uv_tiling: luv_tiling,
                        });
                    }
                    // First layer with PaletteTint > 0 determines the palette channel
                    if lpalette > 0 && palette_tint == 0 {
                        palette_tint = lpalette;
                    }
                }
            }
            _ => {}
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
}
