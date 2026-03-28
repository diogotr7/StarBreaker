use serde::{Deserialize, Serialize};
use starbreaker_common::{CigGuid, ColorRgba, NameHash, ParseError, SpanReader, SpanWriter};

use crate::read_helpers::{expect_empty_guid, expect_u32, read_guid, read_name_hash};

// ─── Texture ─────────────────────────────────────────────────────────────────

/// A texture reference: 21 bytes (u32 zero, u8 index, CigGuid).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Texture {
    pub index: u8,
    pub guid: CigGuid,
}

impl Texture {
    pub fn read(reader: &mut SpanReader) -> Result<Self, ParseError> {
        expect_u32(reader, 0)?;
        let index = reader.read_u8()?;
        let guid = read_guid(reader)?;
        Ok(Texture { index, guid })
    }

    pub fn write(&self, writer: &mut SpanWriter) {
        writer.write_u32(0);
        writer.write_u8(self.index);
        writer.write_val(&self.guid);
    }
}

// ─── MaterialParam ───────────────────────────────────────────────────────────

/// A named material parameter: 12 bytes (NameHash name, T value, u32 zero).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialParam<T: ParamValue> {
    pub name: NameHash,
    pub value: T,
}

/// Trait for material parameter value types (f32 and ColorRgba).
pub trait ParamValue: Clone + std::fmt::Debug {
    fn read_value(reader: &mut SpanReader) -> Result<Self, ParseError>;
    fn write_value(&self, writer: &mut SpanWriter);
}

impl ParamValue for f32 {
    fn read_value(reader: &mut SpanReader) -> Result<Self, ParseError> {
        reader.read_f32()
    }

    fn write_value(&self, writer: &mut SpanWriter) {
        writer.write_f32(*self);
    }
}

impl ParamValue for ColorRgba {
    fn read_value(reader: &mut SpanReader) -> Result<Self, ParseError> {
        // ColorRgba is 4 bytes (r, g, b, a) -- read byte-by-byte for alignment safety
        let r = reader.read_u8()?;
        let g = reader.read_u8()?;
        let b = reader.read_u8()?;
        let a = reader.read_u8()?;
        Ok(ColorRgba::new(r, g, b, a))
    }

    fn write_value(&self, writer: &mut SpanWriter) {
        writer.write_val(self);
    }
}

impl<T: ParamValue> MaterialParam<T> {
    pub fn read(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let name = read_name_hash(reader)?;
        let value = T::read_value(reader)?;
        expect_u32(reader, 0)?;
        Ok(MaterialParam { name, value })
    }

    pub fn write(&self, writer: &mut SpanWriter) {
        writer.write_val(&self.name);
        self.value.write_value(writer);
        writer.write_u32(0);
    }
}

// ─── SubMaterial ─────────────────────────────────────────────────────────────

/// A sub-material containing textures, float parameters, and color parameters.
///
/// Game field names:
/// - `"materialNameCRC"` (v≥4): sub-material name hash
/// - `"Textures"` → `"textureGUID"` (v≥6): texture references
/// - `"FloatParams"` → `"nameCRC"` + `"value"`: float parameters
/// - `"ColorParams"` → `"nameCRC"` + R/G/B/A: color parameters (API uses 4×f32, binary stores 4×u8)
///
/// Note: the game API also reads `"intParams"` (nameCRC + u32 value) for v≥3, but this section
/// is NOT present in the binary CHF file format — only in the GRPC network format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubMaterial {
    pub name: NameHash,
    pub textures: Vec<Texture>,
    pub material_params: Vec<MaterialParam<f32>>,
    pub material_colors: Vec<MaterialParam<ColorRgba>>,
}

impl SubMaterial {
    pub fn read(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let name = read_name_hash(reader)?;

        let texture_count = reader.read_u32()? as usize;
        let mut textures = Vec::with_capacity(texture_count);
        for _ in 0..texture_count {
            textures.push(Texture::read(reader)?);
        }

        let param_count = reader.read_u64()? as usize;
        let mut material_params = Vec::with_capacity(param_count);
        for _ in 0..param_count {
            material_params.push(MaterialParam::<f32>::read(reader)?);
        }

        let color_count = reader.read_u64()? as usize;
        let mut material_colors = Vec::with_capacity(color_count);
        for _ in 0..color_count {
            material_colors.push(MaterialParam::<ColorRgba>::read(reader)?);
        }

        Ok(SubMaterial {
            name,
            textures,
            material_params,
            material_colors,
        })
    }

    pub fn write(&self, writer: &mut SpanWriter) {
        writer.write_val(&self.name);

        writer.write_u32(self.textures.len() as u32);
        for tex in &self.textures {
            tex.write(writer);
        }

        writer.write_u64(self.material_params.len() as u64);
        for param in &self.material_params {
            param.write(writer);
        }

        writer.write_u64(self.material_colors.len() as u64);
        for color in &self.material_colors {
            color.write(writer);
        }
    }
}

// ─── MaterialDefinition ──────────────────────────────────────────────────────

/// A top-level material definition containing sub-materials.
///
/// Game field names: `"attachment"` (v≥5: hash, v<5: string→CRC), `"baseMaterialGUID"` (v≥5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialDefinition {
    /// CRC32 hash of the attachment name. Game field: `"attachment"`.
    pub name: NameHash,
    /// Base material GUID. Game field: `"baseMaterialGUID"`.
    pub guid: CigGuid,
    /// Material flags bitmask. Non-uniform bit distribution rules out a hash function.
    /// Varies per-character even for the same material GUID. Stored and round-tripped.
    pub mtl_flags: u32,
    pub sub_materials: Vec<SubMaterial>,
}

impl MaterialDefinition {
    /// Read a MaterialDefinition from the top-level reader.
    ///
    /// This uses the top-level reader (not a sub-reader) so it can check remaining()
    /// to know whether to read trailing 5 sentinels between sub-materials.
    pub fn read(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let name = read_name_hash(reader)?;
        let guid = read_guid(reader)?;
        let mtl_flags = reader.read_u32()?;
        expect_empty_guid(reader)?;
        let sub_material_count = reader.read_u32()? as usize;

        // Group boundary marker (5) — CryEngine binary serialization BeginGroup tag.
        // See chf_data.rs for full documentation.
        expect_u32(reader, 5)?;

        let mut sub_materials = Vec::with_capacity(sub_material_count);
        for i in 0..sub_material_count {
            sub_materials.push(SubMaterial::read(reader)?);

            let is_last_sub = i == sub_material_count - 1;
            if !is_last_sub {
                // Between sub-materials within the same MaterialDefinition, the group
                // boundary marker (5) is mandatory.
                expect_u32(reader, 5)?;
            } else if reader.remaining() >= 4 {
                // After the last sub-material, the marker is present if another
                // MaterialDefinition follows, absent if this is the final material.
                let pos = reader.position();
                let val = reader.read_u32()?;
                if val != 5 {
                    reader.set_position(pos);
                }
            }
        }

        Ok(MaterialDefinition {
            name,
            guid,
            mtl_flags,
            sub_materials,
        })
    }

    /// Write this MaterialDefinition to the writer.
    ///
    /// `is_last_material` controls whether a trailing 5 is emitted after the last
    /// sub-material: it is NOT emitted for the last sub-material of the last material
    /// definition (since there is nothing following it).
    pub fn write(&self, writer: &mut SpanWriter, is_last_material: bool) {
        writer.write_val(&self.name);
        writer.write_val(&self.guid);
        writer.write_u32(self.mtl_flags);
        writer.write_val(&CigGuid::EMPTY);
        writer.write_u32(self.sub_materials.len() as u32);

        // Group boundary marker (5) before sub-materials
        writer.write_u32(5);

        for (i, sub) in self.sub_materials.iter().enumerate() {
            sub.write(writer);

            let is_last_sub = i == self.sub_materials.len() - 1;
            // Write group boundary marker (5) after each sub-material, EXCEPT
            // after the last sub-material of the last material definition.
            if !(is_last_material && is_last_sub) {
                writer.write_u32(5);
            }
        }
    }
}
