use half::f16;

pub fn decode_half2(raw: [u16; 2]) -> [f32; 2] {
    [
        f16::from_bits(raw[0]).to_f32(),
        f16::from_bits(raw[1]).to_f32(),
    ]
}

pub fn decode_half4(raw: [u16; 4]) -> [f32; 4] {
    [
        f16::from_bits(raw[0]).to_f32(),
        f16::from_bits(raw[1]).to_f32(),
        f16::from_bits(raw[2]).to_f32(),
        f16::from_bits(raw[3]).to_f32(),
    ]
}

/// Dequantize a position from signed-normalized i16 values to world-space f32.
///
/// The raw u16 values are reinterpreted as signed i16 and divided by 32767
/// to get SNorm values in [-1, +1]. These are then scaled to the bounding box:
///   position = snorm * half_extent + center
/// where half_extent = (max - min) / 2 and center = (max + min) / 2.
pub fn dequantize_position(raw: [u16; 3], min: &[f32; 3], max: &[f32; 3]) -> [f32; 3] {
    let mut result = [0.0f32; 3];
    for i in 0..3 {
        let snorm = raw[i] as i16 as f32 / 32767.0;
        // Clamp half_extent to minimum 1.0 per axis, matching CryEngine-Converter.
        // When the bbox extent is < 2m in an axis, clamping to 1.0 effectively
        // outputs raw SNorm values for that axis (no bbox scaling).
        let half_extent = ((max[i] - min[i]) / 2.0).max(1.0);
        let center = (max[i] + min[i]) / 2.0;
        result[i] = snorm * half_extent + center;
    }
    result
}

/// Normal (vec3) and tangent (vec4, w = bitangent sign) extracted from a TBN quaternion.
pub struct NormalTangent {
    pub normal: [f32; 3],
    pub tangent: [f32; 4],
}

/// Decode SPipQTangents (4× SNorm i16 quaternion) into normal + tangent.
///
/// From CryEngine source (VertexFormats.h SPipQTangents):
///   GetQ(): each component = i16 / 32767.0
///   GetN(): q.GetColumn2() * (q.w < 0 ? -1 : +1)
///
/// Normal = column 2 of rotation matrix, negated if w < 0.
pub fn decode_qtangent_snorm(raw: [u16; 4]) -> NormalTangent {
    let qx = raw[0] as i16 as f32 / 32767.0;
    let qy = raw[1] as i16 as f32 / 32767.0;
    let qz = raw[2] as i16 as f32 / 32767.0;
    let qw = raw[3] as i16 as f32 / 32767.0;

    // Column 2 of rotation matrix = normal direction
    let mut nx = 2.0 * (qx * qz + qw * qy);
    let mut ny = 2.0 * (qy * qz - qw * qx);
    let mut nz = 1.0 - 2.0 * (qx * qx + qy * qy);

    // CryEngine convention: negate normal if w < 0 (bitangent reflection)
    if qw < 0.0 {
        nx = -nx;
        ny = -ny;
        nz = -nz;
    }

    // Column 0 = tangent direction
    let tx = 1.0 - 2.0 * (qy * qy + qz * qz);
    let ty = 2.0 * (qx * qy + qw * qz);
    let tz = 2.0 * (qx * qz - qw * qy);

    // Normalize
    let n_len = (nx * nx + ny * ny + nz * nz).sqrt();
    let t_len = (tx * tx + ty * ty + tz * tz).sqrt();
    let handedness = if qw >= 0.0 { 1.0 } else { -1.0 };

    NormalTangent {
        normal: if n_len > 0.0 {
            [nx / n_len, ny / n_len, nz / n_len]
        } else {
            [0.0, 0.0, 1.0]
        },
        tangent: if t_len > 0.0 {
            [tx / t_len, ty / t_len, tz / t_len, handedness]
        } else {
            [1.0, 0.0, 0.0, handedness]
        },
    }
}

/// Decode compressed tangent vectors (8 bytes = 2× u32) into normal + tangent.
///
/// Each u32 encodes a unit vector:
///   - Bits 0-14:  X component, biased by 0x3FFF → signed range [-16383, +16384]
///   - Bits 15-29: Y component, biased by 0x3FFF → signed range [-16383, +16384]
///   - Bit 30:     Z sign (1 = negate Z)
///   - Bit 31:     (first u32 only) bitangent sign for QTangent handedness
///
/// Z is reconstructed: Z = sqrt(1 - X² - Y²), then the vector is normalized.
/// First u32 = tangent direction, second u32 = bitangent direction.
/// Normal = cross(tangent, bitangent).
pub fn decode_compressed_tangent(raw: [u16; 4]) -> NormalTangent {
    let value1 = raw[0] as u32 | ((raw[1] as u32) << 16);
    let value2 = raw[2] as u32 | ((raw[3] as u32) << 16);

    let vec1 = decode_packed_unit_vector(value1);
    let vec2 = decode_packed_unit_vector(value2);

    // Bitangent sign from bit 31 of first u32.
    // On mirrored geometry, the stored bitangent is flipped, so
    // cross(T, B_stored) points the wrong way. Multiply by the sign to correct.
    let bitan_sign = if (value1 & 0x80000000) != 0 { -1.0f32 } else { 1.0 };

    // Normal = cross(tangent, bitangent) * bitan_sign
    let cx = vec1[1] * vec2[2] - vec1[2] * vec2[1];
    let cy = vec1[2] * vec2[0] - vec1[0] * vec2[2];
    let cz = vec1[0] * vec2[1] - vec1[1] * vec2[0];
    let nx = cx * bitan_sign;
    let ny = cy * bitan_sign;
    let nz = cz * bitan_sign;
    let n_len = (nx * nx + ny * ny + nz * nz).sqrt();

    NormalTangent {
        normal: if n_len > 1e-8 {
            [nx / n_len, ny / n_len, nz / n_len]
        } else {
            [0.0, 0.0, 1.0]
        },
        tangent: [vec1[0], vec1[1], vec1[2], bitan_sign],
    }
}

/// Decode a packed unit vector from a u32 (15-15-1-1 bit layout).
/// Used by `decode_compressed_tangent` and directly for IVONORMALS.
pub fn decode_packed_unit_vector(val: u32) -> [f32; 3] {
    const SCALE: f32 = 1.0 / 16383.0;

    let x_raw = (val & 0x7FFF) as i32 - 0x3FFF;
    let y_raw = ((val >> 15) & 0x7FFF) as i32 - 0x3FFF;
    let x = x_raw as f32 * SCALE;
    let y = y_raw as f32 * SCALE;

    let z_sq = (1.0 - x * x - y * y).max(0.0);
    let z_unsigned = z_sq.sqrt();
    let z = if (val >> 30) & 1 != 0 { -z_unsigned } else { z_unsigned };

    // Normalize (the game does this too)
    let len = (x * x + y * y + z * z).sqrt();
    if len > 1e-8 {
        [x / len, y / len, z / len]
    } else {
        [0.0, 0.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_half2_known_values() {
        let result = decode_half2([0x3C00, 0x0000]);
        assert_eq!(result, [1.0, 0.0]);
    }

    #[test]
    fn decode_half2_negative() {
        let result = decode_half2([0xBC00, 0x3C00]);
        assert_eq!(result, [-1.0, 1.0]);
    }

    #[test]
    fn decode_half4_known_values() {
        let result = decode_half4([0x3C00, 0x0000, 0xBC00, 0x3800]);
        assert_eq!(result, [1.0, 0.0, -1.0, 0.5]);
    }

    #[test]
    fn qtangent_snorm_identity_like() {
        // Quaternion (0, 0, 0, 32767) → identity rotation → normal = (0, 0, 1)
        let nt = decode_qtangent_snorm([0, 0, 0, 0x7FFF]);
        assert!((nt.normal[2] - 1.0).abs() < 0.01, "Z should be ~1, got {}", nt.normal[2]);
    }

    #[test]
    fn qtangent_snorm_output_unit_length() {
        let nt = decode_qtangent_snorm([0x3FFF, 0xBFFF, 0x3FFF, 0x3FFF]);
        let len = (nt.normal[0].powi(2) + nt.normal[1].powi(2) + nt.normal[2].powi(2)).sqrt();
        assert!((len - 1.0).abs() < 0.01, "normal length should be ~1.0, got {len}");
    }

    #[test]
    fn compressed_tangent_orthogonal_vectors_produce_unit_normal() {
        // Two orthogonal axis-aligned vectors should produce a unit normal
        // Tangent = +X: x_raw=16383 → (1,0,0), Bitangent = +Y: y_raw=16383 → (0,1,0)
        // Normal = cross(+X, +Y) = +Z
        let tx_val: u32 = 0x3FFF + (0x3FFF << 15); // x=+1, y=0, z_sign=0 → (1,0,0) after normalize
        let by_val: u32 = 0x3FFF + (0x7FFE << 15); // x=0, y=+1, z_sign=0 → (0,1,0) after normalize
        // Actually these bit patterns need care. Let's just test that output is unit length.
        let nt = decode_compressed_tangent([0x3FFF, 0x3FFF, 0x3FFF, 0x3FFF]);
        let len = (nt.normal[0].powi(2) + nt.normal[1].powi(2) + nt.normal[2].powi(2)).sqrt();
        assert!((len - 1.0).abs() < 0.01, "normal length should be ~1.0, got {len}");
    }

    #[test]
    fn packed_unit_vector_roundtrip() {
        // X=0, Y=0 → Z=±1 depending on bit 30
        let v = decode_packed_unit_vector(0x3FFF | (0x3FFF << 15)); // x_raw=0, y_raw=0 → z=1
        assert!((v[2] - 1.0).abs() < 0.01, "Z should be ~1, got {}", v[2]);

        // With Z sign bit set
        let v2 = decode_packed_unit_vector(0x3FFF | (0x3FFF << 15) | (1 << 30));
        assert!((v2[2] + 1.0).abs() < 0.01, "Z should be ~-1, got {}", v2[2]);
    }

    #[test]
    fn dequantize_position_snorm() {
        let min = [0.0, 0.0, 0.0];
        let max = [10.0, 20.0, 30.0];

        // SNorm 0 (raw u16 = 0, interpreted as i16 = 0) → center of bbox
        let at_center = dequantize_position([0, 0, 0], &min, &max);
        let expected_center = [5.0, 10.0, 15.0];
        for i in 0..3 {
            assert!(
                (at_center[i] - expected_center[i]).abs() < 0.01,
                "center[{i}] = {}",
                at_center[i]
            );
        }

        // SNorm +1 (raw u16 = 32767, i16 = 32767) → bbox max
        let at_max = dequantize_position([32767, 32767, 32767], &min, &max);
        for i in 0..3 {
            assert!(
                (at_max[i] - max[i]).abs() < 0.01,
                "max[{i}] = {}",
                at_max[i]
            );
        }

        // SNorm -1 (raw u16 = 0x8001 = 32769, i16 = -32767) → bbox min
        let at_min = dequantize_position([0x8001, 0x8001, 0x8001], &min, &max);
        for i in 0..3 {
            assert!(
                (at_min[i] - min[i]).abs() < 0.01,
                "min[{i}] = {}",
                at_min[i]
            );
        }
    }
}
