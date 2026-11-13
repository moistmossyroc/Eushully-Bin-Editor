//! Monospace dialogue font for Parente's message window.
//!
//! Why: the message window (`Donskoy.UI_MessageWindow.GetFontWidth`) places
//! every half-width character in a FIXED cell (half the em), ignoring each
//! glyph's real advance. The shipped font (Noto Sans JP, proportional Latin)
//! therefore renders English with uneven gaps — `I` floats, `m` crowds. The
//! cell width is a C# constant (not serialized anywhere), but the fonts are
//! baked bitmap atlases inside the UI prefab bundles, so we can redraw the 94
//! ASCII glyphs with a font DESIGNED for half-width cells: the game's own M+ 1m
//! (`ui_battlemap` ships it baked at 40 px, every ASCII advance = 20 = half em).
//!
//! Target = the RUNTIME fonts FontManager loads from `START_Data/sharedassets0.assets`
//! (+ `.resS`) — NOT the per-prefab font copies in the .arc bundles, which the
//! dialogue never uses (verified in-game). Every edit is in place (same byte
//! sizes): `m_CharacterRects` entries are rewritten and the new bitmaps are
//! blitted into free gaps of the Alpha8 4096² atlas. Japanese glyphs untouched.
//!
//! Atlas conventions (verified on the shipped data): `uv` is in texture units,
//! glyph pixels occupy memory rows `[uv.y - h, uv.y)` top→bottom and columns
//! `[uv.x, uv.x + w)` with `uv.height = -h`; `vert` = (left bearing, top above
//! baseline, width, -height) in pixels; `flipped` = stored rotated 90°.

use super::{bundle, typetree};

/// First/last ASCII glyph replaced (space has no bitmap; only its advance).
const ASCII_FIRST: u32 = 0x21;
const ASCII_LAST: u32 = 0x7E;
const SPACE: u32 = 0x20;
/// `m_CharacterRects` element: index u32, uv 4×f32, vert 4×f32, advance f32, flipped u8 (+3 pad).
const RECT_STRIDE: usize = 44;
const CLASS_FONT: i32 = 128;
const CLASS_TEXTURE2D: i32 = 28;
const TEXTURE_FORMAT_ALPHA8: i32 = 1;
/// Free space kept around each placed glyph (matches the baker's padding).
const GLYPH_PADDING: usize = 1;

/// Which game font gets which donor (both live in the `ui_battlemap` bundle).
pub const DONOR_BUNDLE: &str = "ui_battlemap";
pub const FONT_REPLACEMENTS: &[(&str, &str)] = &[
    ("NotoSansJP-Medium", "mplus-1m-regular"),
    ("NotoSansJP-Bold", "mplus-1mn-bold"),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphRect {
    pub index: u32,
    pub uv: [f32; 4],
    pub vert: [f32; 4],
    pub advance: f32,
    pub flipped: bool,
}

fn f32_at(d: &[u8], p: usize) -> f32 {
    f32::from_le_bytes(d[p..p + 4].try_into().unwrap())
}

fn read_rect(d: &[u8], p: usize) -> GlyphRect {
    GlyphRect {
        index: u32::from_le_bytes(d[p..p + 4].try_into().unwrap()),
        uv: [f32_at(d, p + 4), f32_at(d, p + 8), f32_at(d, p + 12), f32_at(d, p + 16)],
        vert: [f32_at(d, p + 20), f32_at(d, p + 24), f32_at(d, p + 28), f32_at(d, p + 32)],
        advance: f32_at(d, p + 36),
        flipped: d[p + 40] != 0,
    }
}

fn write_rect(d: &mut [u8], p: usize, r: &GlyphRect) {
    d[p..p + 4].copy_from_slice(&r.index.to_le_bytes());
    for (k, v) in r.uv.iter().chain(r.vert.iter()).enumerate() {
        d[p + 4 + k * 4..p + 8 + k * 4].copy_from_slice(&v.to_le_bytes());
    }
    d[p + 36..p + 40].copy_from_slice(&r.advance.to_le_bytes());
    d[p + 40] = r.flipped as u8;
}

/// A baked font located inside a SerializedFile + its atlas in the stream.
#[derive(Debug, Clone)]
pub struct BakedFont {
    pub name: String,
    pub font_size: f32,
    /// Absolute offsets of each `m_CharacterRects` element in the SerializedFile.
    pub rect_offsets: Vec<usize>,
    pub atlas: AtlasRef,
}

#[derive(Debug, Clone, Copy)]
pub struct AtlasRef {
    /// Byte offset of the Alpha8 pixels inside the bundle's `.resS` node.
    pub stream_offset: usize,
    pub width: usize,
    pub height: usize,
}

/// Read every baked font (with an Alpha8 atlas) in a SerializedFile.
pub fn read_baked_fonts(sf: &[u8]) -> Result<Vec<BakedFont>, String> {
    read_baked_fonts_in(sf, &typetree::parse(sf)?)
}

/// As `read_baked_fonts` for an already-parsed file (e.g. a stripped player
/// `.assets` whose type trees were filled via `typetree::borrow_types`).
pub fn read_baked_fonts_in(sf: &[u8], file: &typetree::SerializedFile) -> Result<Vec<BakedFont>, String> {
    let file = file.clone();
    let mut atlases = std::collections::HashMap::new();
    for object in file.objects.iter().filter(|o| file.class_id(o) == CLASS_TEXTURE2D) {
        let spans = typetree::walk_object(sf, &file, object)?;
        let i32_field = |path: &str| spans.iter().find(|s| s.path == path).map(|s| i32::from_le_bytes(sf[s.start..s.start + 4].try_into().unwrap()));
        let offset = spans
            .iter()
            .find(|s| s.path == "Base.m_StreamData.offset")
            .map(|s| u64::from_le_bytes(sf[s.start..s.start + 8].try_into().unwrap()) as usize);
        if let (Some(w), Some(h), Some(TEXTURE_FORMAT_ALPHA8), Some(offset)) = (i32_field("Base.m_Width"), i32_field("Base.m_Height"), i32_field("Base.m_TextureFormat"), offset) {
            atlases.insert(object.path_id, AtlasRef { stream_offset: offset, width: w as usize, height: h as usize });
        }
    }
    let mut fonts = Vec::new();
    for object in file.objects.iter().filter(|o| file.class_id(o) == CLASS_FONT) {
        let spans = typetree::walk_object(sf, &file, object)?;
        let name = spans
            .iter()
            .find(|s| s.path == "Base.m_Name")
            .map(|s| String::from_utf8_lossy(&sf[s.start + 4..s.end]).trim_end_matches('\0').to_string())
            .unwrap_or_default();
        let font_size = spans.iter().find(|s| s.path == "Base.m_FontSize").map_or(0.0, |s| f32_at(sf, s.start));
        let texture_id = spans
            .iter()
            .find(|s| s.path == "Base.m_Texture.m_PathID")
            .map(|s| i64::from_le_bytes(sf[s.start..s.start + 8].try_into().unwrap()));
        let Some(atlas) = texture_id.and_then(|id| atlases.get(&id).copied()) else { continue };
        let rect_offsets: Vec<usize> = spans
            .iter()
            .filter(|s| s.path == "Base.m_CharacterRects.Array.data" && s.end - s.start == RECT_STRIDE)
            .map(|s| s.start)
            .collect();
        fonts.push(BakedFont { name, font_size, rect_offsets, atlas });
    }
    Ok(fonts)
}

/// An upright 8-bit coverage bitmap plus its placement metrics (pixels).
#[derive(Debug, Clone)]
pub struct GlyphBitmap {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    pub bearing_x: f32,
    pub top: f32,
}

/// Undo the packer's 90° rotation of a `flipped` glyph: stored `w×h` → upright
/// `h×w` with `upright[y][x] = stored[x][w-1-y]` (counter-clockwise). Direction
/// verified on the shipped M+ atlases: glyphs rotated in one weight but upright
/// in the other (`"`, `*`, `^`) match only under this mapping.
fn unrotate(stored: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (uw, uh) = (h, w);
    let mut upright = vec![0u8; uw * uh];
    for y in 0..uh {
        for x in 0..uw {
            upright[y * uw + x] = stored[x * w + (w - 1 - y)];
        }
    }
    upright
}

/// Copy a glyph's pixels out of an atlas, upright (rotated glyphs are unrotated).
pub fn extract_glyph(atlas: &[u8], atlas_ref: &AtlasRef, rect: &GlyphRect) -> Option<GlyphBitmap> {
    let (w, h) = ((rect.uv[2] * atlas_ref.width as f32).round() as usize, (-rect.uv[3] * atlas_ref.height as f32).round() as usize);
    let x0 = (rect.uv[0] * atlas_ref.width as f32).round() as usize;
    let bottom = (rect.uv[1] * atlas_ref.height as f32).round() as usize;
    let y0 = bottom.checked_sub(h)?;
    let mut stored = Vec::with_capacity(w * h);
    for y in y0..bottom {
        stored.extend_from_slice(atlas.get(y * atlas_ref.width + x0..y * atlas_ref.width + x0 + w)?);
    }
    let (width, height, pixels) = if rect.flipped { (h, w, unrotate(&stored, w, h)) } else { (w, h, stored) };
    Some(GlyphBitmap { width, height, pixels, bearing_x: rect.vert[0], top: rect.vert[1] })
}

/// Area-average resample (box filter) — correct for downscaling AA coverage.
pub fn resample(glyph: &GlyphBitmap, scale: f32) -> GlyphBitmap {
    let width = ((glyph.width as f32 * scale).ceil() as usize).max(1);
    let height = ((glyph.height as f32 * scale).ceil() as usize).max(1);
    let mut pixels = vec![0u8; width * height];
    for ty in 0..height {
        for tx in 0..width {
            let (sx0, sx1) = (tx as f32 / scale, (tx + 1) as f32 / scale);
            let (sy0, sy1) = (ty as f32 / scale, (ty + 1) as f32 / scale);
            let (mut sum, mut area) = (0.0f32, 0.0f32);
            for sy in sy0.floor() as usize..(sy1.ceil() as usize).min(glyph.height) {
                let wy = (sy1.min(sy as f32 + 1.0) - sy0.max(sy as f32)).max(0.0);
                for sx in sx0.floor() as usize..(sx1.ceil() as usize).min(glyph.width) {
                    let wx = (sx1.min(sx as f32 + 1.0) - sx0.max(sx as f32)).max(0.0);
                    sum += glyph.pixels[sy * glyph.width + sx] as f32 * wx * wy;
                    area += wx * wy;
                }
            }
            pixels[ty * width + tx] = if area > 0.0 { (sum / area).round().min(255.0) as u8 } else { 0 };
        }
    }
    GlyphBitmap { width, height, pixels, bearing_x: glyph.bearing_x * scale, top: glyph.top * scale }
}

/// Occupancy map of an atlas: every glyph rect (padded) + any inked pixel.
fn occupancy(sf: &[u8], font: &BakedFont, atlas: &[u8]) -> Vec<bool> {
    let (w, h) = (font.atlas.width, font.atlas.height);
    let mut taken: Vec<bool> = atlas.iter().map(|&a| a != 0).collect();
    for &p in &font.rect_offsets {
        let r = read_rect(sf, p);
        let (ux, uy, uw, uh) = (r.uv[0] * w as f32, r.uv[1] * h as f32, r.uv[2] * w as f32, r.uv[3] * h as f32);
        let x0 = (ux.min(ux + uw).floor() as i64 - 1).max(0) as usize;
        let x1 = ((ux.max(ux + uw).ceil() as i64 + 1).max(0) as usize).min(w);
        let y0 = (uy.min(uy + uh).floor() as i64 - 1).max(0) as usize;
        let y1 = ((uy.max(uy + uh).ceil() as i64 + 1).max(0) as usize).min(h);
        for y in y0..y1 {
            taken[y * w + x0..y * w + x1].iter_mut().for_each(|t| *t = true);
        }
    }
    taken
}

/// First-fit free spot for a `w×h` box (row-major scan), marking it taken.
fn allocate(taken: &mut [bool], atlas_w: usize, atlas_h: usize, w: usize, h: usize, cursor: &mut (usize, usize)) -> Option<(usize, usize)> {
    let fits = |taken: &[bool], x: usize, y: usize| (y..y + h).all(|yy| taken[yy * atlas_w + x..yy * atlas_w + x + w].iter().all(|t| !*t));
    let (mut x, mut y) = *cursor;
    while y + h <= atlas_h {
        while x + w <= atlas_w {
            if fits(taken, x, y) {
                for yy in y..y + h {
                    taken[yy * atlas_w + x..yy * atlas_w + x + w].iter_mut().for_each(|t| *t = true);
                }
                *cursor = (x + w, y);
                return Some((x, y));
            }
            x += 1;
        }
        x = 0;
        y += 1;
    }
    None
}

/// Replace the ASCII glyphs of `target` (rects in `sf`, pixels in `stream`)
/// with `donor` glyphs (from `donor_sf`/`donor_stream`) scaled by the font-size
/// ratio. Returns how many glyphs were replaced. Pure over the given buffers.
pub fn replace_ascii_glyphs(
    sf: &mut [u8],
    stream: &mut [u8],
    target: &BakedFont,
    donor_sf: &[u8],
    donor_stream: &[u8],
    donor: &BakedFont,
) -> Result<usize, String> {
    if target.font_size <= 0.0 || donor.font_size <= 0.0 {
        return Err(format!("font size unknown for {} / {}", target.name, donor.name));
    }
    let scale = target.font_size / donor.font_size;
    let donor_atlas = donor_stream
        .get(donor.atlas.stream_offset..donor.atlas.stream_offset + donor.atlas.width * donor.atlas.height)
        .ok_or("donor atlas out of range")?;
    let donor_rects: std::collections::HashMap<u32, GlyphRect> =
        donor.rect_offsets.iter().map(|&p| read_rect(donor_sf, p)).map(|r| (r.index, r)).collect();
    let (aw, ah) = (target.atlas.width, target.atlas.height);
    let atlas_range = target.atlas.stream_offset..target.atlas.stream_offset + aw * ah;
    let mut taken = occupancy(sf, target, stream.get(atlas_range.clone()).ok_or("target atlas out of range")?);
    let atlas = &mut stream[atlas_range];
    let mut cursor = (0usize, 0usize);
    let mut replaced = 0;
    for &p in &target.rect_offsets {
        let old = read_rect(sf, p);
        if old.index == SPACE {
            if let Some(d) = donor_rects.get(&SPACE) {
                write_rect(sf, p, &GlyphRect { advance: d.advance * scale, ..old });
            }
            continue;
        }
        if !(ASCII_FIRST..=ASCII_LAST).contains(&old.index) {
            continue;
        }
        let Some(source) = donor_rects.get(&old.index).and_then(|r| extract_glyph(donor_atlas, &donor.atlas, r)) else { continue };
        let glyph = resample(&source, scale);
        let (bw, bh) = (glyph.width + 2 * GLYPH_PADDING, glyph.height + 2 * GLYPH_PADDING);
        let (x, y) = allocate(&mut taken, aw, ah, bw, bh, &mut cursor).ok_or_else(|| format!("atlas full for {}", target.name))?;
        // Blit inside the padded box; the rect covers the padding like the baker's.
        for gy in 0..glyph.height {
            let row = (y + GLYPH_PADDING + gy) * aw + x + GLYPH_PADDING;
            atlas[row..row + glyph.width].copy_from_slice(&glyph.pixels[gy * glyph.width..(gy + 1) * glyph.width]);
        }
        let donor_rect = donor_rects[&old.index];
        let new_rect = GlyphRect {
            index: old.index,
            uv: [x as f32 / aw as f32, (y + bh) as f32 / ah as f32, bw as f32 / aw as f32, -(bh as f32) / ah as f32],
            vert: [glyph.bearing_x - GLYPH_PADDING as f32, glyph.top + GLYPH_PADDING as f32, bw as f32, -(bh as f32)],
            advance: donor_rect.advance * scale,
            flipped: false,
        };
        write_rect(sf, p, &new_rect);
        replaced += 1;
    }
    Ok(replaced)
}

/// Player asset file holding the fonts `FontManager` loads at startup (the
/// fonts the message window ACTUALLY renders with — the prefab-bundle copies
/// are unused duplicates). Stripped of type trees; its `.resS` holds the atlases.
pub const RUNTIME_FONT_ASSETS: &str = "sharedassets0.assets";
const RUNTIME_FONT_STREAM_SUFFIX: &str = ".resS";

/// Patch the runtime fonts inside a stripped player `.assets` file + its
/// `.resS` stream in place, borrowing type trees and donor glyphs from the
/// `ui_battlemap` bundle (same Unity version). Returns glyphs replaced.
pub fn patch_runtime_fonts(assets: &mut [u8], stream: &mut [u8], donor_raw: &[u8]) -> Result<usize, String> {
    let donor = bundle::unpack_bundle(donor_raw)?;
    let d_sf_len = donor.nodes.first().ok_or("donor bundle has no nodes")?.size as usize;
    let d_stream = donor.nodes.get(1).ok_or("donor bundle has no .resS stream")?;
    let donor_sf = &donor.blob[..d_sf_len];
    let donor_stream = &donor.blob[d_stream.offset as usize..(d_stream.offset + d_stream.size) as usize];
    let donor_file = typetree::parse(donor_sf)?;
    let donor_fonts = read_baked_fonts_in(donor_sf, &donor_file)?;

    let mut file = typetree::parse(assets)?;
    typetree::borrow_types(&mut file, &donor_file);
    let targets = read_baked_fonts_in(assets, &file)?;
    let (assets_len, stream_len) = (assets.len(), stream.len());
    let mut total = 0;
    for (target_name, donor_name) in FONT_REPLACEMENTS {
        let (Some(t), Some(d)) = (targets.iter().find(|f| f.name == *target_name), donor_fonts.iter().find(|f| f.name == *donor_name)) else { continue };
        total += replace_ascii_glyphs(assets, stream, t, donor_sf, donor_stream, d)?;
    }
    if assets.len() != assets_len || stream.len() != stream_len {
        return Err("internal error: runtime font patch changed a file size".into());
    }
    Ok(total)
}

/// Paths of the runtime font asset + stream inside a game's `START_Data`.
pub fn runtime_font_paths(start_data: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let assets = start_data.join(RUNTIME_FONT_ASSETS);
    let stream = start_data.join(format!("{RUNTIME_FONT_ASSETS}{RUNTIME_FONT_STREAM_SUFFIX}"));
    (assets, stream)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_roundtrip() {
        let r = GlyphRect { index: 0x41, uv: [0.1, 0.2, 0.003, -0.004], vert: [1.0, 24.0, 20.0, -25.0], advance: 15.0, flipped: true };
        let mut buf = vec![0u8; RECT_STRIDE];
        write_rect(&mut buf, 0, &r);
        assert_eq!(read_rect(&buf, 0), r);
    }

    #[test]
    fn resample_preserves_solid_coverage_and_scales_metrics() {
        let g = GlyphBitmap { width: 4, height: 4, pixels: vec![255; 16], bearing_x: 4.0, top: 20.0 };
        let s = resample(&g, 0.75);
        assert_eq!((s.width, s.height), (3, 3));
        assert!(s.pixels.iter().all(|&p| p == 255));
        assert_eq!((s.bearing_x, s.top), (3.0, 15.0));
    }

    #[test]
    fn unrotate_is_counter_clockwise() {
        // stored 3x2:  a b c / d e f   → upright 2x3: c f / b e / a d
        let stored = [1, 2, 3, 4, 5, 6];
        assert_eq!(unrotate(&stored, 3, 2), vec![3, 6, 2, 5, 1, 4]);
    }

    #[test]
    fn allocate_skips_taken_cells() {
        let (w, h) = (8, 8);
        let mut taken = vec![false; w * h];
        taken[0..4].iter_mut().for_each(|t| *t = true); // row 0, x 0..4 taken
        let mut cursor = (0, 0);
        let (x, y) = allocate(&mut taken, w, h, 3, 2, &mut cursor).unwrap();
        assert!(x >= 4 || y >= 1, "must not overlap taken pixels: ({x},{y})");
    }
}
