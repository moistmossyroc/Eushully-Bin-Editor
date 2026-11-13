//! Eushully AGF images (SYS3/SYS4 title art, banners, splashes).
//!
//! Layout (type 2, RE'd from retail files — see docs/engine/age-sys.md):
//! ```text
//! "ACGF" u32 type(=2) u32 0
//! u32 unpacked, u32 unpacked, u32 packed + LZSS data   block 0: BMP header (1080 bytes)
//! u32 unpacked, u32 unpacked, u32 packed + LZSS data   block 1: colour pixels
//! "ACIF" u32 2, u32 0, u32 alpha_size, u32 width, u32 height,
//!        u32 unpacked, u32 unpacked, u32 packed + LZSS data   8-bit alpha plane
//! ```
//! Block 0 = BITMAPFILEHEADER (14) + 2 pad + BITMAPINFOHEADER (40) at 16 + a
//! 1024-byte palette area. Colour is 8 bpp paletted or 24 bpp, BMP row order
//! (bottom-up, rows padded to 4 bytes); the ACIF alpha plane is TOP-DOWN and
//! unpadded. Decoder verified pixel-exact (colour + alpha) against AGF2BMP2AGF
//! on Tenmei SO004 (24 bpp) and SO005A/B (8 bpp). LZSS = the engine's Okumura variant
//! (`alf::lzss_*`); "packed == unpacked" means the block is stored raw.

use crate::alf::{lzss_decompress, lzss_store};

const MAGIC: &[u8; 4] = b"ACGF";
const ALPHA_MAGIC: &[u8; 4] = b"ACIF";
const HEADER_BLOCK_LEN: usize = 1080;
const INFO_HEADER_OFFSET: usize = 16;
const PALETTE_OFFSET: usize = INFO_HEADER_OFFSET + 40;

/// A decoded AGF: straight (non-premultiplied) BGRA, rows top-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgfImage {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, String> {
    data.get(offset..offset + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| format!("AGF truncated at {offset:#x}"))
}

/// Read one `(unpacked, unpacked, packed)` block at `offset`; returns (bytes, next offset).
fn read_block(data: &[u8], offset: usize) -> Result<(Vec<u8>, usize), String> {
    let unpacked = read_u32(data, offset)? as usize;
    let packed = read_u32(data, offset + 8)? as usize;
    let start = offset + 12;
    let body = data.get(start..start + packed).ok_or("AGF block exceeds file")?;
    let bytes = if packed == unpacked { body.to_vec() } else { lzss_decompress(body, unpacked) };
    if bytes.len() != unpacked {
        return Err(format!("AGF block at {offset:#x}: expected {unpacked} bytes, got {}", bytes.len()));
    }
    Ok((bytes, start + packed))
}

fn row_stride(width: u32, bits_per_pixel: u32) -> usize {
    ((width as usize * bits_per_pixel as usize + 31) / 32) * 4
}

/// Decode an AGF (8 bpp paletted or 24 bpp, with or without an ACIF alpha plane).
pub fn decode_agf(data: &[u8]) -> Result<AgfImage, String> {
    // Older builds (e.g. Atelial, SYS4 438) leave the magic blank: accept
    // `ACGF` or four zero bytes, with type 1 (no alpha) or 2 (ACIF alpha).
    let magic_ok = data.get(..4) == Some(MAGIC.as_slice()) || data.get(..4) == Some([0u8; 4].as_slice());
    let agf_type = data.get(4..8).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    if !magic_ok || !matches!(agf_type, Some(1) | Some(2)) {
        return Err("not an AGF (missing ACGF magic / unknown type)".into());
    }
    let (header, next) = read_block(data, 12)?;
    let (pixels, next) = read_block(data, next)?;
    let width = read_u32(&header, INFO_HEADER_OFFSET + 4)?;
    let height = read_u32(&header, INFO_HEADER_OFFSET + 8)?;
    let bits = u16::from_le_bytes(header[INFO_HEADER_OFFSET + 14..INFO_HEADER_OFFSET + 16].try_into().unwrap()) as u32;
    if bits != 8 && bits != 24 && bits != 32 {
        return Err(format!("unsupported AGF colour depth {bits}"));
    }
    let stride = row_stride(width, bits);
    let mut bgra = vec![255u8; width as usize * height as usize * 4];
    for y in 0..height as usize {
        let source_row = &pixels[(height as usize - 1 - y) * stride..];
        for x in 0..width as usize {
            let out = (y * width as usize + x) * 4;
            match bits {
                8 => {
                    let p = PALETTE_OFFSET + source_row[x] as usize * 4;
                    bgra[out..out + 3].copy_from_slice(&header[p..p + 3]);
                }
                24 => bgra[out..out + 3].copy_from_slice(&source_row[x * 3..x * 3 + 3]),
                _ => bgra[out..out + 4].copy_from_slice(&source_row[x * 4..x * 4 + 4]),
            }
        }
    }
    if data.get(next..next + 4) == Some(ALPHA_MAGIC.as_slice()) {
        let (alpha, _) = read_block(data, next + 4 + 5 * 4)?;
        let pixel_count = width as usize * height as usize;
        if alpha.len() < pixel_count {
            return Err("AGF alpha plane too small".into());
        }
        // Alpha plane is stored TOP-DOWN (unlike the colour rows), unpadded.
        for y in 0..height as usize {
            let source_row = y * width as usize;
            for x in 0..width as usize {
                bgra[(y * width as usize + x) * 4 + 3] = alpha[source_row + x];
            }
        }
    }
    Ok(AgfImage { width, height, bgra })
}

fn push_block(out: &mut Vec<u8>, bytes: &[u8]) {
    let packed = lzss_store(bytes);
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
    out.extend_from_slice(&packed);
}

/// Encode as 24 bpp colour + ACIF alpha (full colour, no palette quantisation).
pub fn encode_agf(image: &AgfImage) -> Vec<u8> {
    let (width, height) = (image.width as usize, image.height as usize);
    let stride = row_stride(image.width, 24);
    let mut header = vec![0u8; HEADER_BLOCK_LEN];
    let image_size = (stride * height) as u32;
    header[2..6].copy_from_slice(&(54 + image_size).to_le_bytes()); // bfSize (bfType left 0, as retail)
    header[10..14].copy_from_slice(&54u32.to_le_bytes()); // bfOffBits
    let info = INFO_HEADER_OFFSET;
    header[info..info + 4].copy_from_slice(&40u32.to_le_bytes());
    header[info + 4..info + 8].copy_from_slice(&image.width.to_le_bytes());
    header[info + 8..info + 12].copy_from_slice(&image.height.to_le_bytes());
    header[info + 12..info + 14].copy_from_slice(&1u16.to_le_bytes());
    header[info + 14..info + 16].copy_from_slice(&24u16.to_le_bytes());
    header[info + 20..info + 24].copy_from_slice(&image_size.to_le_bytes());

    let mut pixels = vec![0u8; stride * height];
    let mut alpha = vec![0u8; width * height];
    for y in 0..height {
        let target_row = (height - 1 - y) * stride;
        for x in 0..width {
            let source = (y * width + x) * 4;
            pixels[target_row + x * 3..target_row + x * 3 + 3].copy_from_slice(&image.bgra[source..source + 3]);
            alpha[y * width + x] = image.bgra[source + 3]; // alpha plane is top-down
        }
    }

    let mut out = Vec::with_capacity(64 + header.len() + pixels.len() + alpha.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    push_block(&mut out, &header);
    push_block(&mut out, &pixels);
    out.extend_from_slice(ALPHA_MAGIC);
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(alpha.len() as u32).to_le_bytes());
    out.extend_from_slice(&image.width.to_le_bytes());
    out.extend_from_slice(&image.height.to_le_bytes());
    push_block(&mut out, &alpha);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip_keeps_colour_and_alpha() {
        // 3x2 with a non-multiple-of-4 row (9 bytes → padded to 12).
        let bgra: Vec<u8> = (0..3 * 2 * 4).map(|i| (i * 37 % 251) as u8).collect();
        let image = AgfImage { width: 3, height: 2, bgra };
        let decoded = decode_agf(&encode_agf(&image)).unwrap();
        assert_eq!(decoded, image);
    }

    #[test]
    fn rejects_non_agf() {
        assert!(decode_agf(b"BM\0\0").is_err());
    }
}
