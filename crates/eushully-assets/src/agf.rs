// agf.rs — The one AGF (ACGF) image codec: decoder shared by preview and
// extraction, plus the re-encoders used by image translation (agf-translator).
//
// Layout (confirmed against ~45,600 retail AGFs across 8 SYS3/SYS4 titles,
// 2026-11-09 audit, see preview.rs `image_audit`):
//
//   0x00  magic: "ACGF" (8bpp-era) or 4 × 0x00 (older 24bpp-era)
//   0x04  u32 type (1 = no alpha plane, 2 = ACIF alpha plane follows)
//   0x08  u32 unknown (kept verbatim)
//   0x0C  header sector: u32 expanded, u32 expanded (dup), u32 stored, data
//           (stored == expanded → raw, else LZSS). The block holds:
//           +0x00  BITMAPFILEHEADER (14) + 2 pad
//           +0x10  BITMAPINFOHEADER (width, height (>0 = bottom-up), bpp, clrUsed)
//           +0x10+biSize  palette, 4 bytes/entry BGRx (8bpp only)
//   then  pixel sector (same 12-byte prefix); rows 4-byte aligned
//   then  optional "ACIF" block: 24-byte header ("ACIF", type, ?, w*h, w, h),
//           then a sector holding the alpha plane — width × height bytes,
//           TOP-DOWN, no row padding.
//
// ACIF appears after BOTH 8bpp and 24bpp pixel data (24bpp + ACIF = character
// sprites `CS*`/`CA*`, effects `AE*`, map parts `MP*`). Alpha shorter than
// width × height (seen only in hand-edited files) is ignored → opaque image.
// Retail alpha sectors are LZSS; AGF2BMP2AGF and the encoders below store them
// raw, which the game also loads.
//
// Every buffer access is length-checked: malformed / modded files return Err,
// never panic.
//
// Encoding keeps the original file's structure byte-for-byte (prefix, header
// block, ACIF header) and swaps only palette / pixels / alpha, writing every
// sector uncompressed — exactly what AGF2BMP2AGF.exe does, whose output is
// known to load in-game. Dimensions cannot change.

use image::RgbaImage;

const ACGF_MAGIC: &[u8; 4] = b"ACGF";
const NULL_MAGIC: [u8; 4] = [0; 4];
const ACIF_MAGIC: &[u8; 4] = b"ACIF";
const FILE_PREFIX_LEN: usize = 0x0C;
const BIH_OFFSET_IN_HEADER: usize = 0x10;
const BIH_COLORS_USED_OFFSET: usize = 32;
const MIN_BIH_SIZE: usize = 40;
const SECTOR_PREFIX_LEN: usize = 12;
const ACIF_HEADER_LEN: usize = 24;
const AGF_TYPE_WITH_ALPHA: u32 = 2;
const MAX_DIMENSION: u32 = 16384;
const MAX_PALETTE_ENTRIES: usize = 256;
const LZSS_INITIAL_FILL: u8 = 0x00;

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    data.get(offset..offset.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

/// True for both AGF magics. Callers use this to route non-AGF content that
/// merely carries an `.agf` name (e.g. MPEG movies) elsewhere.
pub fn has_agf_magic(data: &[u8]) -> bool {
    data.get(0..4).is_some_and(|magic| magic == ACGF_MAGIC || magic == NULL_MAGIC)
}

/// Sector at `offset`: (expanded bytes, offset just past the stored data).
fn read_sector(data: &[u8], offset: usize, what: &str) -> Result<(Vec<u8>, usize), String> {
    let truncated = || format!("{what} sector header truncated");
    let expanded_size = read_u32(data, offset).ok_or_else(truncated)? as usize;
    let stored_size = read_u32(data, offset + 8).ok_or_else(truncated)? as usize;
    let start = offset + SECTOR_PREFIX_LEN;
    let end = start.checked_add(stored_size).ok_or_else(truncated)?;
    let stored = data
        .get(start..end)
        .ok_or_else(|| format!("{what} truncated (need {stored_size} bytes, have {})", data.len().saturating_sub(start)))?;
    let expanded = if stored_size == expanded_size {
        stored.to_vec()
    } else {
        crate::lzss::decompress(stored, LZSS_INITIAL_FILL).map_err(|e| format!("{what} LZSS error: {e}"))?
    };
    Ok((expanded, end))
}

/// Stored (uncompressed) sector: expanded == dup == stored size.
fn write_sector(out: &mut Vec<u8>, payload: &[u8]) {
    let size = (payload.len() as u32).to_le_bytes();
    for _ in 0..3 {
        out.extend_from_slice(&size);
    }
    out.extend_from_slice(payload);
}

struct BitmapInfo {
    width: usize,
    height: usize,
    bottom_up: bool,
    bits_per_pixel: u16,
    header_size: usize,
    colors_used: usize,
}

fn parse_bitmap_info(header_block: &[u8]) -> Result<BitmapInfo, String> {
    let at = BIH_OFFSET_IN_HEADER;
    let field = |offset: usize| read_u32(header_block, at + offset).ok_or("header block too small for BITMAPINFOHEADER");
    let header_size = field(0)? as usize;
    if header_size < MIN_BIH_SIZE {
        return Err(format!("bad BITMAPINFOHEADER size {header_size}"));
    }
    let width = field(4)?;
    let raw_height = field(8)? as i32;
    let bits_per_pixel = read_u16(header_block, at + 14).ok_or("header block too small")?;
    let colors_used = field(BIH_COLORS_USED_OFFSET)? as usize;
    let height = raw_height.unsigned_abs();
    if width == 0 || width > MAX_DIMENSION || height == 0 || height > MAX_DIMENSION {
        return Err(format!("implausible dimensions {width}x{raw_height}"));
    }
    Ok(BitmapInfo {
        width: width as usize,
        height: height as usize,
        bottom_up: raw_height > 0,
        bits_per_pixel,
        header_size,
        colors_used,
    })
}

fn palette_offset(info: &BitmapInfo) -> usize {
    BIH_OFFSET_IN_HEADER + info.header_size
}

/// BGRx palette → RGB triples (`clrUsed` entries, 0 = 256).
fn parse_palette(header_block: &[u8], info: &BitmapInfo) -> Result<Vec<[u8; 3]>, String> {
    let entry_count = if info.colors_used > 0 { info.colors_used.min(MAX_PALETTE_ENTRIES) } else { MAX_PALETTE_ENTRIES };
    let start = palette_offset(info);
    let raw = header_block
        .get(start..start + entry_count * 4)
        .ok_or("palette extends past header block")?;
    Ok(raw.chunks_exact(4).map(|bgrx| [bgrx[2], bgrx[1], bgrx[0]]).collect())
}

/// ACIF header + alpha plane, if present and complete.
fn read_acif(data: &[u8], acif_offset: usize, pixel_count: usize) -> Option<([u8; ACIF_HEADER_LEN], Vec<u8>)> {
    let header: [u8; ACIF_HEADER_LEN] = data.get(acif_offset..acif_offset.checked_add(ACIF_HEADER_LEN)?)?.try_into().ok()?;
    if &header[0..4] != ACIF_MAGIC {
        return None;
    }
    let (alpha, _) = read_sector(data, acif_offset + ACIF_HEADER_LEN, "alpha").ok()?;
    (alpha.len() >= pixel_count).then_some((header, alpha))
}

fn row_stride(width: usize, bytes_per_pixel: usize) -> usize {
    (width * bytes_per_pixel + 3) & !3
}

/// A decoded AGF plus everything needed to write it back.
pub struct AgfDocument {
    pub width: u32,
    pub height: u32,
    pub bits_per_pixel: u16,
    /// File header type field (1 = opaque, 2 = with alpha plane).
    pub agf_type: u32,
    pub bottom_up: bool,
    /// 8bpp: the stored palette (RGB, `clrUsed` entries). Empty for 24bpp.
    pub palette: Vec<[u8; 3]>,
    /// 8bpp: palette index per pixel, top-down, `width × height`. Empty for 24bpp.
    pub indices: Vec<u8>,
    /// Top-down RGBA (alpha 255 where the file has no alpha plane).
    pub image: RgbaImage,
    file_prefix: [u8; FILE_PREFIX_LEN],
    header_block: Vec<u8>,
    acif_header: Option<[u8; ACIF_HEADER_LEN]>,
}

impl AgfDocument {
    /// True if the file carries an ACIF alpha plane (re-encodes keep one).
    pub fn has_alpha_plane(&self) -> bool {
        self.acif_header.is_some()
    }

    /// Alpha-plane writes are allowed when the original had one, or claims
    /// one via its type field (agf-translator synthesizes the ACIF header).
    fn writes_alpha_plane(&self) -> bool {
        self.acif_header.is_some() || self.agf_type == AGF_TYPE_WITH_ALPHA
    }

    fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

/// Decode AGF bytes into a document (image + structure for re-encoding).
pub fn decode_agf_document(data: &[u8]) -> Result<AgfDocument, String> {
    if !has_agf_magic(data) {
        return Err(format!("not an AGF file (magic {:02X?})", data.get(0..4).unwrap_or(data)));
    }
    let file_prefix: [u8; FILE_PREFIX_LEN] =
        data.get(..FILE_PREFIX_LEN).and_then(|prefix| prefix.try_into().ok()).ok_or("file header truncated")?;
    let agf_type = read_u32(data, 4).ok_or("file header truncated")?;
    let (header_block, pixel_sector_offset) = read_sector(data, FILE_PREFIX_LEN, "header block")?;
    let info = parse_bitmap_info(&header_block)?;

    let bytes_per_pixel = match info.bits_per_pixel {
        8 => 1,
        24 => 3,
        other => return Err(format!("unsupported bpp {other}")),
    };

    let (pixels, acif_offset) = read_sector(data, pixel_sector_offset, "pixel data")?;
    let stride = row_stride(info.width, bytes_per_pixel);
    let required = stride * info.height;
    if pixels.len() < required {
        return Err(format!("pixel data short: have {} bytes, need {required}", pixels.len()));
    }

    let pixel_count = info.width * info.height;
    let acif = read_acif(data, acif_offset, pixel_count);
    let palette = if bytes_per_pixel == 1 { parse_palette(&header_block, &info)? } else { Vec::new() };

    let mut rgba = vec![255u8; pixel_count * 4];
    let mut indices = if bytes_per_pixel == 1 { vec![0u8; pixel_count] } else { Vec::new() };
    for row in 0..info.height {
        let source_row = if info.bottom_up { info.height - 1 - row } else { row };
        let source_row_bytes = &pixels[source_row * stride..source_row * stride + info.width * bytes_per_pixel];
        let destination_row = &mut rgba[row * info.width * 4..(row + 1) * info.width * 4];
        for (col, destination) in destination_row.chunks_exact_mut(4).enumerate() {
            let rgb = if bytes_per_pixel == 1 {
                let index = source_row_bytes[col];
                indices[row * info.width + col] = index;
                palette.get(index as usize).copied().unwrap_or([0, 0, 0])
            } else {
                let bgr = &source_row_bytes[col * 3..col * 3 + 3];
                [bgr[2], bgr[1], bgr[0]]
            };
            destination[..3].copy_from_slice(&rgb);
            if let Some((_, alpha)) = &acif {
                destination[3] = alpha[row * info.width + col];
            }
        }
    }

    let image = RgbaImage::from_raw(info.width as u32, info.height as u32, rgba)
        .ok_or_else(|| "failed to construct image".to_string())?;
    Ok(AgfDocument {
        width: info.width as u32,
        height: info.height as u32,
        bits_per_pixel: info.bits_per_pixel,
        agf_type,
        bottom_up: info.bottom_up,
        palette,
        indices,
        image,
        file_prefix,
        header_block,
        acif_header: acif.map(|(header, _)| header),
    })
}

/// Decode AGF bytes into a top-down RGBA image.
pub fn decode_agf(data: &[u8]) -> Result<RgbaImage, String> {
    decode_agf_document(data).map(|document| document.image)
}

/// Top-down rows → file row order with 4-byte row padding.
fn to_stored_rows(top_down: &[u8], row_len: usize, height: usize, bottom_up: bool) -> Vec<u8> {
    let stride = (row_len + 3) & !3;
    let mut stored = vec![0u8; stride * height];
    for (row, source) in top_down.chunks_exact(row_len).enumerate() {
        let destination_row = if bottom_up { height - 1 - row } else { row };
        stored[destination_row * stride..destination_row * stride + row_len].copy_from_slice(source);
    }
    stored
}

fn plane_is_opaque(alpha: &[u8]) -> bool {
    alpha.iter().all(|&value| value == u8::MAX)
}

/// Shared tail of both encoders: prefix, header, pixels, optional ACIF.
fn assemble_agf(original: &AgfDocument, header_block: &[u8], stored_pixels: &[u8], alpha: &[u8]) -> Result<Vec<u8>, String> {
    if !original.writes_alpha_plane() && !plane_is_opaque(alpha) {
        return Err("the original AGF has no alpha plane, so transparency can't be stored; keep the image opaque".into());
    }
    let mut out = Vec::with_capacity(FILE_PREFIX_LEN + 3 * SECTOR_PREFIX_LEN + header_block.len() + stored_pixels.len() + alpha.len() + ACIF_HEADER_LEN);
    out.extend_from_slice(&original.file_prefix);
    write_sector(&mut out, header_block);
    write_sector(&mut out, stored_pixels);
    if original.writes_alpha_plane() {
        let acif_header = original.acif_header.unwrap_or_else(|| synthesized_acif_header(original.width, original.height));
        out.extend_from_slice(&acif_header);
        write_sector(&mut out, alpha);
    }
    Ok(out)
}

fn synthesized_acif_header(width: u32, height: u32) -> [u8; ACIF_HEADER_LEN] {
    let mut header = [0u8; ACIF_HEADER_LEN];
    header[0..4].copy_from_slice(ACIF_MAGIC);
    header[4..8].copy_from_slice(&AGF_TYPE_WITH_ALPHA.to_le_bytes());
    header[12..16].copy_from_slice(&(width * height).to_le_bytes());
    header[16..20].copy_from_slice(&width.to_le_bytes());
    header[20..24].copy_from_slice(&height.to_le_bytes());
    header
}

/// Header block with the palette replaced. If the new palette has more entries
/// than the original header holds, the palette area grows and `clrUsed`
/// (plus the embedded BITMAPFILEHEADER size/offset, if present) follow it.
fn header_with_palette(original: &AgfDocument, palette: &[[u8; 3]]) -> Result<Vec<u8>, String> {
    let info = parse_bitmap_info(&original.header_block)?;
    let start = palette_offset(&info);
    let stored_capacity = original.palette.len();
    let entry_count = palette.len().max(stored_capacity);
    let mut header = original.header_block.clone();

    if entry_count > stored_capacity {
        let grown_by = (entry_count - stored_capacity) * 4;
        let insert_at = start + stored_capacity * 4;
        header.splice(insert_at..insert_at, std::iter::repeat(0u8).take(grown_by));
        let colors_used_at = BIH_OFFSET_IN_HEADER + BIH_COLORS_USED_OFFSET;
        header[colors_used_at..colors_used_at + 4].copy_from_slice(&(entry_count as u32).to_le_bytes());
        if header.starts_with(b"BM") {
            for field_offset in [2usize, 10] {
                let value = read_u32(&header, field_offset).unwrap_or(0) + grown_by as u32;
                header[field_offset..field_offset + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
    }

    for (slot, bgrx) in header[start..start + entry_count * 4].chunks_exact_mut(4).enumerate() {
        let [red, green, blue] = palette.get(slot).copied().unwrap_or([0, 0, 0]);
        bgrx.copy_from_slice(&[blue, green, red, 0]);
    }
    Ok(header)
}

/// Re-encode an 8bpp AGF with a new palette (≤ 256 RGB entries), top-down
/// `indices` and top-down `alpha` (both `width × height`).
pub fn encode_agf_8bpp(original: &AgfDocument, palette: &[[u8; 3]], indices: &[u8], alpha: &[u8]) -> Result<Vec<u8>, String> {
    if original.bits_per_pixel != 8 {
        return Err(format!("encode_agf_8bpp needs an 8bpp original (got {}bpp)", original.bits_per_pixel));
    }
    let pixel_count = original.pixel_count();
    if palette.is_empty() || palette.len() > MAX_PALETTE_ENTRIES {
        return Err(format!("palette must have 1..=256 entries (got {})", palette.len()));
    }
    if indices.len() != pixel_count || alpha.len() != pixel_count {
        return Err(format!("expected {pixel_count} indices and alpha values, got {} and {}", indices.len(), alpha.len()));
    }
    if let Some(&bad) = indices.iter().find(|&&index| index as usize >= palette.len()) {
        return Err(format!("index {bad} is past the {}-entry palette", palette.len()));
    }
    let header_block = header_with_palette(original, palette)?;
    let stored_pixels = to_stored_rows(indices, original.width as usize, original.height as usize, original.bottom_up);
    assemble_agf(original, &header_block, &stored_pixels, alpha)
}

/// Re-encode a 24bpp AGF from a top-down RGBA image of the same size.
/// Alpha goes to the ACIF plane when the original has one.
pub fn encode_agf_24bpp(original: &AgfDocument, image: &RgbaImage) -> Result<Vec<u8>, String> {
    if original.bits_per_pixel != 24 {
        return Err(format!("encode_agf_24bpp needs a 24bpp original (got {}bpp)", original.bits_per_pixel));
    }
    if image.dimensions() != (original.width, original.height) {
        return Err(format!(
            "image is {}x{}, the AGF is {}x{} (dimensions can't change)",
            image.width(), image.height(), original.width, original.height
        ));
    }
    let bgr: Vec<u8> = image.pixels().flat_map(|pixel| [pixel[2], pixel[1], pixel[0]]).collect();
    let alpha: Vec<u8> = image.pixels().map(|pixel| pixel[3]).collect();
    let stored_pixels = to_stored_rows(&bgr, original.width as usize * 3, original.height as usize, original.bottom_up);
    assemble_agf(original, &original.header_block, &stored_pixels, &alpha)
}

/// Encode an edited top-down RGBA image (same size) in the original's format:
/// 8bpp originals get an exact palette when the image has ≤256 colours, else a
/// k-means one (`dither` = Floyd–Steinberg); 24bpp keeps full colour.
pub fn encode_agf_from_rgba(original: &AgfDocument, edited: &RgbaImage, dither: bool) -> Result<Vec<u8>, String> {
    if edited.dimensions() != (original.width, original.height) {
        return Err(format!(
            "image is {}x{}, the AGF is {}x{} (dimensions can't change)",
            edited.width(), edited.height(), original.width, original.height
        ));
    }
    match original.bits_per_pixel {
        8 => {
            let paletted = crate::utils::palette_quantize::palettize(edited.as_raw(), original.width as usize, dither);
            encode_agf_8bpp(original, &paletted.palette, &paletted.indices, &alpha_of(edited.as_raw()))
        }
        _ => encode_agf_24bpp(original, edited),
    }
}

/// Re-encode with the document's own pixels (palette/indices or image).
/// Decoding the result gives back exactly `original.image`.
pub fn reencode_unchanged(original: &AgfDocument) -> Result<Vec<u8>, String> {
    let alpha = alpha_of(original.image.as_raw());
    match original.bits_per_pixel {
        8 => encode_agf_8bpp(original, &original.palette, &original.indices, &alpha),
        _ => encode_agf_24bpp(original, &original.image),
    }
}

/// Nearest palette entry (squared RGB distance) for every top-down RGBA pixel.
pub fn indices_for_palette(rgba: &[u8], palette: &[[u8; 3]]) -> Vec<u8> {
    let mut nearest_by_color: std::collections::HashMap<[u8; 3], u8> = std::collections::HashMap::new();
    rgba.chunks_exact(4)
        .map(|pixel| {
            let color = [pixel[0], pixel[1], pixel[2]];
            *nearest_by_color.entry(color).or_insert_with(|| nearest_palette_index(color, palette))
        })
        .collect()
}

fn nearest_palette_index(color: [u8; 3], palette: &[[u8; 3]]) -> u8 {
    let distance = |entry: &[u8; 3]| -> i32 { entry.iter().zip(color).map(|(&a, b)| (a as i32 - b as i32).pow(2)).sum() };
    palette
        .iter()
        .enumerate()
        .min_by_key(|(_, entry)| distance(entry))
        .map_or(0, |(index, _)| index as u8)
}

/// The A channel of a top-down RGBA buffer.
pub fn alpha_of(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4).map(|pixel| pixel[3]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LZSS stream made only of literals (flag byte 0xFF per 8 bytes).
    fn lzss_literals(bytes: &[u8]) -> Vec<u8> {
        bytes.chunks(8).flat_map(|chunk| std::iter::once(0xFF).chain(chunk.iter().copied())).collect()
    }

    /// Minimal AGF: 2×2, bottom-up, header/pixels stored raw, optional ACIF
    /// (LZSS like retail, or stored raw like AGF2BMP2AGF output).
    fn build_agf(bits_per_pixel: u16, pixel_rows_bottom_up: &[u8], alpha_top_down: Option<&[u8]>, alpha_compressed: bool) -> Vec<u8> {
        let mut header_block = vec![0u8; 0x10];
        let mut bih = vec![0u8; 40];
        bih[0..4].copy_from_slice(&40u32.to_le_bytes());
        bih[4..8].copy_from_slice(&2u32.to_le_bytes());
        bih[8..12].copy_from_slice(&2i32.to_le_bytes());
        bih[14..16].copy_from_slice(&bits_per_pixel.to_le_bytes());
        if bits_per_pixel == 8 {
            bih[32..36].copy_from_slice(&2u32.to_le_bytes()); // 2 palette entries
        }
        header_block.extend(bih);
        if bits_per_pixel == 8 {
            header_block.extend([0, 0, 255, 0]); // idx 0 = red (BGRx)
            header_block.extend([255, 0, 0, 0]); // idx 1 = blue
        }

        let mut file = vec![0u8; FILE_PREFIX_LEN];
        file[0..4].copy_from_slice(if bits_per_pixel == 8 { b"ACGF" } else { &[0; 4] });
        let agf_type: u32 = if alpha_top_down.is_some() { 2 } else { 1 };
        file[4..8].copy_from_slice(&agf_type.to_le_bytes());
        write_sector(&mut file, &header_block);
        write_sector(&mut file, pixel_rows_bottom_up);
        if let Some(alpha) = alpha_top_down {
            file.extend(synthesized_acif_header(2, 2));
            if alpha_compressed {
                let compressed = lzss_literals(alpha);
                file.extend((alpha.len() as u32).to_le_bytes());
                file.extend((alpha.len() as u32).to_le_bytes());
                file.extend((compressed.len() as u32).to_le_bytes());
                file.extend(compressed);
            } else {
                write_sector(&mut file, alpha);
            }
        }
        file
    }

    const PIXELS_8: [u8; 8] = [1, 1, 0, 0, 0, 1, 0, 0]; // bottom row [1,1], top row [0,1]
    const PIXELS_24: [u8; 16] = [0, 255, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0];

    #[test]
    fn decodes_8bpp_bottom_up_with_top_down_alpha() {
        let document = decode_agf_document(&build_agf(8, &PIXELS_8, Some(&[10, 20, 30, 40]), true)).unwrap();
        assert_eq!(document.image.get_pixel(0, 0).0, [255, 0, 0, 10]); // top-left: red, alpha[0]
        assert_eq!(document.image.get_pixel(1, 1).0, [0, 0, 255, 40]); // bottom-right: blue, alpha[3]
        assert_eq!(document.indices, [0, 1, 1, 1]);
        assert_eq!(document.palette.len(), 2);
    }

    #[test]
    fn decodes_24bpp_with_acif_alpha_compressed_or_stored() {
        // 2 px × 3 bytes = 6, padded to 8. Bottom row green, top row white (BGR).
        for compressed in [true, false] {
            let img = decode_agf(&build_agf(24, &PIXELS_24, Some(&[0, 255, 128, 64]), compressed)).unwrap();
            assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 0]);
            assert_eq!(img.get_pixel(1, 0).0, [255, 255, 255, 255]);
            assert_eq!(img.get_pixel(0, 1).0, [0, 255, 0, 128]);
        }
    }

    #[test]
    fn opaque_without_acif_and_with_short_alpha() {
        assert_eq!(decode_agf(&build_agf(24, &PIXELS_24, None, true)).unwrap().get_pixel(0, 0).0[3], 255);
        assert_eq!(decode_agf(&build_agf(24, &PIXELS_24, Some(&[0, 0]), true)).unwrap().get_pixel(0, 0).0[3], 255);
    }

    #[test]
    fn truncated_pixels_are_an_error_not_a_panic() {
        let mut file = build_agf(24, &[0; 16], None, true);
        file.truncate(file.len() - 6);
        assert!(decode_agf(&file).is_err());
        assert!(decode_agf(&[0u8; 0x20]).is_err());
        assert!(decode_agf(b"\0\0\x01\xBAmpeg").is_err());
    }

    #[test]
    fn unchanged_reencode_is_byte_identical_for_stored_files() {
        for file in [
            build_agf(8, &PIXELS_8, Some(&[10, 20, 30, 40]), false),
            build_agf(8, &PIXELS_8, None, false),
            build_agf(24, &PIXELS_24, Some(&[0, 255, 128, 64]), false),
            build_agf(24, &PIXELS_24, None, false),
        ] {
            let original = decode_agf_document(&file).unwrap();
            assert_eq!(reencode_unchanged(&original).unwrap(), file);
        }
    }

    #[test]
    fn edited_24bpp_alpha_and_grown_palette_encode() {
        let original = decode_agf_document(&build_agf(24, &PIXELS_24, Some(&[0, 255, 128, 64]), true)).unwrap();
        let mut edited = original.image.clone();
        edited.put_pixel(1, 1, image::Rgba([1, 2, 3, 77]));
        let decoded = decode_agf(&encode_agf_24bpp(&original, &edited).unwrap()).unwrap();
        assert_eq!(decoded, edited);

        let paletted = decode_agf_document(&build_agf(8, &PIXELS_8, None, false)).unwrap();
        let palette = [[9, 9, 9], [1, 1, 1], [200, 100, 50]];
        let encoded = encode_agf_8bpp(&paletted, &palette, &[2, 2, 0, 1], &[255; 4]).unwrap();
        let regrown = decode_agf_document(&encoded).unwrap();
        assert_eq!(regrown.palette, palette);
        assert_eq!(regrown.image.get_pixel(0, 0).0, [200, 100, 50, 255]);
    }

    #[test]
    fn encoders_reject_impossible_edits() {
        let opaque = decode_agf_document(&build_agf(24, &PIXELS_24, None, false)).unwrap();
        let mut transparent = opaque.image.clone();
        transparent.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        assert!(encode_agf_24bpp(&opaque, &transparent).is_err());
        assert!(encode_agf_24bpp(&opaque, &RgbaImage::new(3, 2)).is_err());
        assert!(encode_agf_8bpp(&opaque, &[[0; 3]], &[0; 4], &[255; 4]).is_err());

        let paletted = decode_agf_document(&build_agf(8, &PIXELS_8, None, false)).unwrap();
        assert!(encode_agf_8bpp(&paletted, &[[0; 3]], &[1; 4], &[255; 4]).is_err());
        assert!(encode_agf_8bpp(&paletted, &[[0; 3]], &[0; 3], &[255; 4]).is_err());
    }

    /// Every AGF in every AGE game below `AGE_GAMES_ROOT`: decode → re-encode
    /// unchanged → decode must give identical pixels, alpha plane and palette.
    /// `AGE_GAMES_ROOT=<folder> cargo test --release real_agf_roundtrip -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_agf_roundtrip() {
        use std::collections::BTreeMap;
        let games_root = std::path::PathBuf::from(std::env::var("AGE_GAMES_ROOT").expect("set AGE_GAMES_ROOT"));
        let mut tallies: BTreeMap<String, (usize, String)> = BTreeMap::new();
        let mut record = |key: String, name: &str| {
            tallies.entry(key).or_insert_with(|| (0, name.to_string())).0 += 1;
        };
        for root in crate::utils::game_discovery::find_game_roots(&games_root) {
            if root.kind != crate::utils::game_discovery::GameKind::Age {
                continue;
            }
            let archive = crate::archive::AgeArchive::open(&root.path).expect("open failed");
            for entry in archive.entries.iter().filter(|entry| entry.name.to_ascii_lowercase().ends_with(".agf")) {
                let Ok(data) = archive.read_entry(entry) else { record("read error".into(), &entry.name); continue };
                if !has_agf_magic(&data) {
                    continue;
                }
                let original = match decode_agf_document(&data) {
                    Ok(document) => document,
                    Err(error) => { record(format!("decode error: {error}"), &entry.name); continue }
                };
                record(
                    format!(
                        "type={} bpp={} acif={} palette={}",
                        original.agf_type,
                        original.bits_per_pixel,
                        original.has_alpha_plane(),
                        match (original.bits_per_pixel, original.palette.len()) {
                            (8, MAX_PALETTE_ENTRIES) => "256",
                            (8, _) => "<256",
                            _ => "-",
                        }
                    ),
                    &entry.name,
                );
                let outcome = reencode_unchanged(&original).and_then(|bytes| decode_agf_document(&bytes));
                match outcome {
                    Err(error) => record(format!("RE-ENCODE ERROR: {error}"), &entry.name),
                    Ok(again) if again.image != original.image => record("ROUND-TRIP PIXEL MISMATCH".into(), &entry.name),
                    Ok(again) if again.has_alpha_plane() != original.has_alpha_plane() || again.palette != original.palette => {
                        record("ROUND-TRIP STRUCTURE MISMATCH".into(), &entry.name)
                    }
                    Ok(_) => record("round-trip ok".into(), &entry.name),
                }
            }
        }
        for (key, (count, example)) in &tallies {
            println!("{count:>7}  {key:<50} e.g. {example}");
        }
        assert!(!tallies.keys().any(|key| key.starts_with("ROUND-TRIP") || key.starts_with("RE-ENCODE")));
    }

    #[test]
    fn nearest_palette_lookup() {
        let palette = [[0, 0, 0], [250, 250, 250], [255, 0, 0]];
        assert_eq!(indices_for_palette(&[240, 240, 255, 255, 200, 10, 10, 0], &palette), [1, 2]);
    }
}
