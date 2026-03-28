//! Parser for IncludedObjects chunks (type 0x0010) found in .soc files.
//!
//! These chunks contain pre-baked geometry references (CGF file paths) with
//! 3×4 f64 transform matrices, used for ship interior static meshes.

use starbreaker_common::ParseError;

/// A single geometry placement from an IncludedObjects chunk.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IncludedObject {
    /// Index into the CGF path list.
    pub cgf_index: u16,
    /// Unknown u16 after the id — might be bone index or flags.
    pub unknown2: u16,
    /// 3×4 column-major transform matrix (f64, truncate to f32 for glTF).
    /// Layout: `[[r00,r10,r20], [r01,r11,r21], [r02,r12,r22], [tx,ty,tz]]`
    pub transform: [[f64; 3]; 4],
    /// Two f64×3 vectors before the transform (purpose unknown — bone ref? bbox?).
    pub vector1: [f64; 3],
    pub vector2: [f64; 3],
}

/// Parsed IncludedObjects chunk data.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct IncludedObjects {
    /// CGF geometry file paths (e.g., "objects/spaceships/ships/rsi/zeus/interior/...cgf").
    pub cgf_paths: Vec<String>,
    /// Material file paths.
    pub material_paths: Vec<String>,
    /// Tint palette paths.
    pub tint_palette_paths: Vec<String>,
    /// Placed geometry objects with transforms.
    pub objects: Vec<IncludedObject>,
}

const CHUNK_STR_LEN: usize = 256;

impl IncludedObjects {
    /// Parse an IncludedObjects chunk from raw chunk data.
    pub fn from_bytes(data: &[u8]) -> Result<Self, ParseError> {
        let mut off = 4; // skip 4-byte padding

        // CGF paths
        let num_cgfs = read_u32(data, &mut off)? as usize;
        let mut cgf_paths = Vec::with_capacity(num_cgfs);
        for _ in 0..num_cgfs {
            cgf_paths.push(read_fixed_string(data, &mut off)?);
        }

        // Material + tint palette counts: two u16 packed in 4 bytes
        let num_materials = read_u16(data, &mut off)? as usize;
        let num_palettes = read_u16(data, &mut off)? as usize;

        let mut material_paths = Vec::with_capacity(num_materials);
        for _ in 0..num_materials {
            material_paths.push(read_fixed_string(data, &mut off)?);
        }

        let mut tint_palette_paths = Vec::with_capacity(num_palettes);
        for _ in 0..num_palettes {
            tint_palette_paths.push(read_fixed_string(data, &mut off)?);
        }

        // Skip 28 unknown bytes
        if off + 28 > data.len() {
            return Err(truncated(off, 28, data.len()));
        }
        off += 28;

        // Objects section: byte count, not object count
        let len_objects_bytes = read_u32(data, &mut off)? as usize;
        let objects_end = off + len_objects_bytes;
        let mut objects = Vec::new();

        while off + 4 <= objects_end {
            let obj_type = u32::from_le_bytes(
                data[off..off + 4]
                    .try_into()
                    .map_err(|_| truncated(off, 4, data.len()))?,
            );

            match obj_type {
                0x0000_0001 => {
                    // Type1: variable size, determined by trailing unknown3 field
                    let base_size = 168;
                    if off + base_size > data.len() {
                        break;
                    }

                    // vector1 at +4, vector2 at +28 (each 3×f64 = 24 bytes)
                    let mut vector1 = [0.0f64; 3];
                    let mut vector2 = [0.0f64; 3];
                    for i in 0..3 {
                        vector1[i] = f64::from_le_bytes(
                            data[off + 4 + i * 8..off + 12 + i * 8].try_into().unwrap(),
                        );
                        vector2[i] = f64::from_le_bytes(
                            data[off + 28 + i * 8..off + 36 + i * 8].try_into().unwrap(),
                        );
                    }

                    let id = u16::from_le_bytes(data[off + 60..off + 62].try_into().unwrap());
                    let unknown2 = u16::from_le_bytes(data[off + 62..off + 64].try_into().unwrap());

                    // 3×4 f64 transform at offset +64 (after type+vectors+unknown+id+unknown2)
                    // Data is row-major: [r00,r01,r02,tx, r10,r11,r12,ty, r20,r21,r22,tz]
                    // We store column-major: transform[col][row]
                    let mat_off = off + 64;
                    let mut transform = [[0.0f64; 3]; 4];
                    for row in 0..3 {
                        for (col, col_data) in transform.iter_mut().enumerate() {
                            let idx = mat_off + (row * 4 + col) * 8;
                            col_data[row] =
                                f64::from_le_bytes(data[idx..idx + 8].try_into().unwrap());
                        }
                    }

                    let unknown3 =
                        u64::from_le_bytes(data[off + 160..off + 168].try_into().unwrap());
                    // Scan forward for the next valid object type marker to determine actual size
                    let actual_size = if unknown3 == 0 {
                        base_size + 16
                    } else {
                        base_size
                    };

                    // Skip any trailing zero padding until we hit a valid type or end
                    let mut end = off + actual_size;
                    while end + 4 <= objects_end {
                        let next_type =
                            u32::from_le_bytes(data[end..end + 4].try_into().unwrap_or([0; 4]));
                        if next_type == 0 {
                            end += 4;
                        } else {
                            break;
                        }
                    }

                    // Read the unknown1 field at +52 (u64)
                    let unknown1 = u64::from_le_bytes(data[off + 52..off + 60].try_into().unwrap());

                    // If unknown3==0, there are 16 extra bytes — read them
                    let extra = if unknown3 == 0 && off + 184 <= data.len() {
                        let e1 = u64::from_le_bytes(data[off + 168..off + 176].try_into().unwrap());
                        let e2 = u64::from_le_bytes(data[off + 176..off + 184].try_into().unwrap());
                        format!(", extra=[{e1:#x}, {e2:#x}]")
                    } else {
                        String::new()
                    };

                    log::debug!(
                        "IncludedObjects: Type1 at offset {off}, cgf_index={id}, unknown1={unknown1:#x}, unknown2={unknown2:#x}, unknown3={unknown3:#x}{extra}"
                    );

                    objects.push(IncludedObject {
                        cgf_index: id,
                        unknown2,
                        transform,
                        vector1,
                        vector2,
                    });
                    off = end;
                }
                0x0000_0007 => {
                    // Type7: 152 bytes total (4 type + 148 unknown)
                    log::debug!("IncludedObjects: skipping Type7 object at offset {off}");
                    off += 152;
                }
                0x0000_0010 => {
                    // Type10: 136 bytes total
                    log::debug!("IncludedObjects: skipping Type10 object at offset {off}");
                    off += 136;
                }
                _ => {
                    // Unknown type: skip 4 bytes (brute force, matching scdatatools)
                    log::debug!(
                        "IncludedObjects: unknown type {obj_type:#010x} at offset {off}, skipping 4 bytes"
                    );
                    off += 4;
                }
            }

            if off > objects_end {
                break;
            }
        }

        Ok(IncludedObjects {
            cgf_paths,
            material_paths,
            tint_palette_paths,
            objects,
        })
    }
}

fn read_u32(data: &[u8], off: &mut usize) -> Result<u32, ParseError> {
    if *off + 4 > data.len() {
        return Err(truncated(*off, 4, data.len()));
    }
    let val = u32::from_le_bytes(data[*off..*off + 4].try_into().unwrap());
    *off += 4;
    Ok(val)
}

fn read_u16(data: &[u8], off: &mut usize) -> Result<u16, ParseError> {
    if *off + 2 > data.len() {
        return Err(truncated(*off, 2, data.len()));
    }
    let val = u16::from_le_bytes(data[*off..*off + 2].try_into().unwrap());
    *off += 2;
    Ok(val)
}

fn read_fixed_string(data: &[u8], off: &mut usize) -> Result<String, ParseError> {
    if *off + CHUNK_STR_LEN > data.len() {
        return Err(truncated(*off, CHUNK_STR_LEN, data.len()));
    }
    let bytes = &data[*off..*off + CHUNK_STR_LEN];
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(CHUNK_STR_LEN);
    *off += CHUNK_STR_LEN;
    Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

fn truncated(offset: usize, need: usize, have: usize) -> ParseError {
    ParseError::Truncated {
        offset,
        need,
        have: have.saturating_sub(offset),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal IncludedObjects chunk with 1 CGF, 0 materials, 0 palettes, 1 Type1 object.
    fn make_test_chunk(unknown3: u64) -> Vec<u8> {
        let mut buf = Vec::new();

        // 4 bytes padding
        buf.extend_from_slice(&[0u8; 4]);

        // num_cgfs = 1
        buf.extend_from_slice(&1u32.to_le_bytes());

        // CGF path: "test.cgf" padded to 256 bytes
        let mut path = b"test.cgf".to_vec();
        path.resize(256, 0);
        buf.extend_from_slice(&path);

        // num_materials = 0 (u16), num_palettes = 0 (u16)
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());

        // 28 unknown bytes
        buf.extend_from_slice(&[0u8; 28]);

        // Build a Type1 object
        let mut obj = Vec::new();
        obj.extend_from_slice(&1u32.to_le_bytes()); // object_type = 1
        obj.extend_from_slice(&[0u8; 48]); // vector1 + vector2 (6 × f64)
        obj.extend_from_slice(&[0u8; 8]); // unknown1
        obj.extend_from_slice(&0u16.to_le_bytes()); // id = 0
        obj.extend_from_slice(&0u16.to_le_bytes()); // unknown2

        // 3×4 row-major transform with non-identity rotation + translation:
        // Row 0: [0.0, -1.0, 0.0, 5.0]   (90° rotation around Z)
        // Row 1: [1.0,  0.0, 0.0, 10.0]
        // Row 2: [0.0,  0.0, 1.0, 15.0]
        for &val in &[
            0.0, -1.0, 0.0, 5.0, 1.0, 0.0, 0.0, 10.0, 0.0, 0.0, 1.0, 15.0f64,
        ] {
            obj.extend_from_slice(&val.to_le_bytes());
        }

        obj.extend_from_slice(&unknown3.to_le_bytes());
        if unknown3 == 0 {
            obj.extend_from_slice(&[0u8; 16]); // extra 16 bytes
        }

        // Objects section: byte count
        buf.extend_from_slice(&(obj.len() as u32).to_le_bytes());
        buf.extend_from_slice(&obj);

        buf
    }

    #[test]
    fn parse_single_object_184_byte_variant() {
        let data = make_test_chunk(0); // unknown3 == 0 → 184 bytes
        let io = IncludedObjects::from_bytes(&data).unwrap();

        assert_eq!(io.cgf_paths.len(), 1);
        assert_eq!(io.cgf_paths[0], "test.cgf");
        assert_eq!(io.material_paths.len(), 0);
        assert_eq!(io.tint_palette_paths.len(), 0);
        assert_eq!(io.objects.len(), 1);
        assert_eq!(io.objects[0].cgf_index, 0);

        // Verify column-major storage of a 90° Z rotation + translation:
        // Row-major input: [[0,-1,0,5], [1,0,0,10], [0,0,1,15]]
        // Column 0 (first rotation col): [0, 1, 0]
        // Column 3 (translation): [5, 10, 15]
        let t = &io.objects[0].transform;
        assert!(
            (t[0][0] - 0.0).abs() < 1e-10,
            "col0 row0: expected 0, got {}",
            t[0][0]
        );
        assert!(
            (t[0][1] - 1.0).abs() < 1e-10,
            "col0 row1: expected 1, got {}",
            t[0][1]
        );
        assert!(
            (t[0][2] - 0.0).abs() < 1e-10,
            "col0 row2: expected 0, got {}",
            t[0][2]
        );
        assert!(
            (t[1][0] - -1.0).abs() < 1e-10,
            "col1 row0: expected -1, got {}",
            t[1][0]
        );
        assert!((t[3][0] - 5.0).abs() < 1e-10, "tx");
        assert!((t[3][1] - 10.0).abs() < 1e-10, "ty");
        assert!((t[3][2] - 15.0).abs() < 1e-10, "tz");
    }

    #[test]
    fn parse_single_object_168_byte_variant() {
        let data = make_test_chunk(42); // unknown3 != 0 → 168 bytes
        let io = IncludedObjects::from_bytes(&data).unwrap();

        assert_eq!(io.objects.len(), 1);
        assert_eq!(io.objects[0].cgf_index, 0);

        let t = &io.objects[0].transform;
        assert!((t[3][0] - 5.0).abs() < 1e-10);
        // Verify rotation is correct too (not just translation)
        assert!(
            (t[0][1] - 1.0).abs() < 1e-10,
            "col0 row1 should be 1.0 for 90° Z rotation"
        );
    }

    #[test]
    fn parse_empty_chunk() {
        let mut buf = vec![0u8; 4]; // padding
        buf.extend_from_slice(&0u32.to_le_bytes()); // 0 cgfs
        buf.extend_from_slice(&0u16.to_le_bytes()); // 0 materials
        buf.extend_from_slice(&0u16.to_le_bytes()); // 0 palettes
        buf.extend_from_slice(&[0u8; 28]); // unknown
        buf.extend_from_slice(&0u32.to_le_bytes()); // 0 bytes of objects

        let io = IncludedObjects::from_bytes(&buf).unwrap();
        assert!(io.cgf_paths.is_empty());
        assert!(io.objects.is_empty());
    }
}
