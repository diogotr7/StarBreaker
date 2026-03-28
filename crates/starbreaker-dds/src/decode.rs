use crate::error::DdsError;

/// Block-compressed format variants for decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockFormat {
    BC1,
    BC3,
    BC4,
    BC5,
    BC6H,
    BC7,
}

/// Decode block-compressed texture data into RGBA8 pixels.
///
/// Returns a `Vec<u8>` with 4 bytes per pixel (R, G, B, A) in row-major order.
/// All formats decoded via `bcdec_rs`.
pub fn decode_block_compressed(
    data: &[u8],
    width: u32,
    height: u32,
    format: BlockFormat,
    is_snorm: bool,
) -> Result<Vec<u8>, DdsError> {
    match format {
        BlockFormat::BC1 => decode_4bpp(data, width, height, bcdec_rs::bc1),
        BlockFormat::BC3 => decode_4bpp(data, width, height, bcdec_rs::bc3),
        BlockFormat::BC4 => decode_bc4(data, width, height, is_snorm),
        BlockFormat::BC5 => Ok(decode_bc5(data, width, height, is_snorm)),
        BlockFormat::BC6H => Err(DdsError::UnsupportedFormat(
            "BC6H decoding is not yet supported (HDR format)".to_string(),
        )),
        BlockFormat::BC7 => decode_4bpp(data, width, height, bcdec_rs::bc7),
    }
}

/// Decode a format that outputs 4 bytes per pixel (RGBA) with 4×4 blocks.
/// Used for BC1 (8 bytes/block), BC3 (16 bytes/block), BC7 (16 bytes/block).
fn decode_4bpp(
    data: &[u8],
    width: u32,
    height: u32,
    decode_block: fn(&[u8], &mut [u8], usize),
) -> Result<Vec<u8>, DdsError> {
    let w = width as usize;
    let h = height as usize;
    let bw = (w + 3) / 4;
    let bh = (h + 3) / 4;
    let block_size = if data.len() / (bw * bh) == 8 { 8 } else { 16 };
    let pitch = 4 * 4; // 4 pixels × 4 bytes (RGBA)
    let mut out = vec![0u8; w * h * 4];
    let mut block_buf = [0u8; 4 * 4 * 4]; // 4×4 × RGBA

    for by in 0..bh {
        for bx in 0..bw {
            let offset = (by * bw + bx) * block_size;
            if offset + block_size > data.len() {
                break;
            }

            decode_block(&data[offset..], &mut block_buf, pitch);

            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x >= w || y >= h {
                        continue;
                    }
                    let src = (py * 4 + px) * 4;
                    let dst = (y * w + x) * 4;
                    out[dst..dst + 4].copy_from_slice(&block_buf[src..src + 4]);
                }
            }
        }
    }
    Ok(out)
}

/// Decode BC4 (single channel). Expand to RGBA: (R, R, R, 255).
fn decode_bc4(data: &[u8], width: u32, height: u32, is_snorm: bool) -> Result<Vec<u8>, DdsError> {
    let w = width as usize;
    let h = height as usize;
    let bw = (w + 3) / 4;
    let bh = (h + 3) / 4;
    let mut out = vec![0u8; w * h * 4];
    let mut block_buf = [0u8; 4 * 4]; // 4×4 × 1 byte
    let pitch = 4; // 4 pixels × 1 byte

    for by in 0..bh {
        for bx in 0..bw {
            let offset = (by * bw + bx) * 8;
            if offset + 8 > data.len() {
                break;
            }

            bcdec_rs::bc4(&data[offset..], &mut block_buf, pitch, is_snorm);

            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x >= w || y >= h {
                        continue;
                    }
                    let r = block_buf[py * 4 + px];
                    let dst = (y * w + x) * 4;
                    out[dst] = r;
                    out[dst + 1] = r;
                    out[dst + 2] = r;
                    out[dst + 3] = 255;
                }
            }
        }
    }
    Ok(out)
}

/// Decode BC5 (two-channel normal map) with Z reconstruction.
/// Output: RGBA8 where R,G = normal XY in [0..255] (128=zero), B = reconstructed Z, A = 255.
fn decode_bc5(data: &[u8], width: u32, height: u32, is_snorm: bool) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let bw = (w + 3) / 4;
    let bh = (h + 3) / 4;
    let mut out = vec![0u8; w * h * 4];
    let mut block_buf = [0u8; 4 * 4 * 2]; // 4×4 × 2 bytes (RG)
    let pitch = 4 * 2;

    for by in 0..bh {
        for bx in 0..bw {
            let offset = (by * bw + bx) * 16;
            if offset + 16 > data.len() {
                break;
            }

            bcdec_rs::bc5(&data[offset..], &mut block_buf, pitch, is_snorm);

            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x >= w || y >= h {
                        continue;
                    }

                    let bi = (py * 4 + px) * 2;
                    let raw_r = block_buf[bi];
                    let raw_g = block_buf[bi + 1];

                    // Convert decoded byte to [-1..1] normal component.
                    let (nr, ng) = if is_snorm {
                        let sr = raw_r as i8;
                        let sg = raw_g as i8;
                        (sr.max(-127) as f32 / 127.0, sg.max(-127) as f32 / 127.0)
                    } else {
                        (
                            (raw_r as f32 / 255.0) * 2.0 - 1.0,
                            (raw_g as f32 / 255.0) * 2.0 - 1.0,
                        )
                    };

                    // Reconstruct Z from unit normal constraint
                    let nz_sq = 1.0 - nr * nr - ng * ng;
                    let nz = if nz_sq > 0.0 { nz_sq.sqrt() } else { 0.0 };

                    // Convert to unsigned [0..255] where 128 = zero (glTF normal map format)
                    let dst = (y * w + x) * 4;
                    out[dst] = ((nr * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
                    out[dst + 1] = ((ng * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
                    out[dst + 2] = ((nz * 0.5 + 0.5) * 255.0).clamp(0.0, 255.0) as u8;
                    out[dst + 3] = 255;
                }
            }
        }
    }
    out
}
