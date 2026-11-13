// version_compare.rs — Original vs mod: how a modded file relates to the
// original versions of the same file (base game, official updates).
//
// Every copy of a name across tiers is a "version" (`AgeArchive::version_history`).
// For a mod copy we measure its similarity to each original version and report
// which one it was most likely made from. If that is not the newest original
// (an official update changed the file after the mod was made), the mod is
// flagged as based on an outdated version. A mod identical to the newest
// original is flagged too: it changes nothing, which usually means a missed file.
//
// Similarity is a heuristic in 0..=1 (flagged as such in every UI):
//   images: share of matching pixels (within PIXEL_TOLERANCE, so palette
//           re-quantization isn't an edit; sizes differ → overlap only)
//   text:   line-based ratio (`similar`)
//   other:  share of identical bytes at the same offsets

use image::RgbaImage;

use crate::archive::{AgeArchive, EntryTier};

/// Similarity gap below which two originals count as an equally good match.
const BASIS_TIE_MARGIN: f64 = 0.005;

pub enum ComparableContent {
    Image(RgbaImage),
    Text(String),
    Binary(Vec<u8>),
}

/// Decode bytes into the form they are compared in.
pub fn comparable_content(bytes: Vec<u8>) -> ComparableContent {
    if crate::agf::has_agf_magic(&bytes) {
        if let Ok(image) = crate::agf::decode_agf(&bytes) {
            return ComparableContent::Image(image);
        }
    }
    if matches!(crate::utils::file_magic::detect_extension(&bytes), Some("png" | "bmp" | "jpg")) {
        if let Ok(image) = image::load_from_memory(&bytes) {
            return ComparableContent::Image(image.into_rgba8());
        }
    }
    match crate::preview::decode_text_content(&bytes) {
        Some(text) => ComparableContent::Text(text),
        None => ComparableContent::Binary(bytes),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageDifference {
    pub same_size: bool,
    pub changed_pixels: usize,
    /// Pixels of the larger image (changes outside the overlap count as changed).
    pub total_pixels: usize,
    /// x, y, width, height of the changed area (in overlap coordinates).
    pub changed_bounds: Option<[u32; 4]>,
}

/// Largest per-channel difference still counted as "unchanged". Re-encoding an
/// edited 8bpp AGF re-quantizes its palette, shifting most colours slightly;
/// those shifts are not edits.
pub const PIXEL_TOLERANCE: u8 = 24;

/// Equal within `PIXEL_TOLERANCE`; fully transparent pixels are equal whatever
/// RGB they hide.
fn pixels_match(a: &image::Rgba<u8>, b: &image::Rgba<u8>) -> bool {
    if a[3] == 0 && b[3] == 0 {
        return true;
    }
    a.0.iter().zip(b.0.iter()).all(|(&x, &y)| x.abs_diff(y) <= PIXEL_TOLERANCE)
}

pub fn image_difference(before: &RgbaImage, after: &RgbaImage) -> ImageDifference {
    let (overlap_width, overlap_height) = (before.width().min(after.width()), before.height().min(after.height()));
    let total_pixels = (before.width() * before.height()).max(after.width() * after.height()) as usize;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0, 0);
    let mut changed_in_overlap = 0usize;
    for y in 0..overlap_height {
        for x in 0..overlap_width {
            if !pixels_match(before.get_pixel(x, y), after.get_pixel(x, y)) {
                changed_in_overlap += 1;
                (min_x, min_y, max_x, max_y) = (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y));
            }
        }
    }
    let overlap_pixels = (overlap_width * overlap_height) as usize;
    ImageDifference {
        same_size: before.dimensions() == after.dimensions(),
        changed_pixels: changed_in_overlap + (total_pixels - overlap_pixels),
        total_pixels,
        changed_bounds: (changed_in_overlap > 0).then(|| [min_x, min_y, max_x - min_x + 1, max_y - min_y + 1]),
    }
}

/// `after` with unchanged pixels faded to grey and changed pixels kept, tinted
/// red where they became transparent — for spotting edits at a glance.
pub fn highlight_changes(before: &RgbaImage, after: &RgbaImage) -> RgbaImage {
    RgbaImage::from_fn(after.width(), after.height(), |x, y| {
        let pixel = *after.get_pixel(x, y);
        let unchanged = x < before.width() && y < before.height() && pixels_match(before.get_pixel(x, y), &pixel);
        if unchanged {
            let grey = ((pixel[0] as u16 + pixel[1] as u16 + pixel[2] as u16) / 3 / 3 + 40) as u8;
            image::Rgba([grey, grey, grey, 255])
        } else if pixel[3] < 32 {
            image::Rgba([255, 40, 70, 255])
        } else {
            image::Rgba([pixel[0], pixel[1], pixel[2], 255])
        }
    })
}

pub fn similarity(a: &ComparableContent, b: &ComparableContent) -> f64 {
    match (a, b) {
        (ComparableContent::Image(a), ComparableContent::Image(b)) => {
            let difference = image_difference(a, b);
            1.0 - difference.changed_pixels as f64 / difference.total_pixels.max(1) as f64
        }
        (ComparableContent::Text(a), ComparableContent::Text(b)) => similar::TextDiff::from_lines(a, b).ratio() as f64,
        (ComparableContent::Binary(a), ComparableContent::Binary(b)) => {
            let same = a.iter().zip(b).filter(|(x, y)| x == y).count();
            same as f64 / a.len().max(b.len()).max(1) as f64
        }
        _ => 0.0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModVerdict {
    /// No original version exists: the mod adds a new file.
    NewFile,
    /// Most similar to the newest original version.
    UpToDate,
    /// Byte-identical to the newest original: the mod changes nothing.
    IdenticalToOriginal,
    /// Most similar to an older original; an official update changed the file since.
    BasedOnOlderVersion { based_on: usize, latest: usize },
}

impl ModVerdict {
    pub fn needs_attention(&self) -> bool {
        matches!(self, Self::IdenticalToOriginal | Self::BasedOnOlderVersion { .. })
    }
}

#[derive(Clone, Debug)]
pub struct ModCheck {
    pub mod_entry: usize,
    /// Original-content versions older than the mod, in load order.
    pub originals: Vec<usize>,
    /// Similarity of the mod to each of `originals` (same order).
    pub similarities: Vec<f64>,
    pub verdict: ModVerdict,
}

/// Index (into `similarities`) of the original the mod was most likely made
/// from. Ties go to the NEWEST, so equal matches never flag a mod as stale.
pub fn likely_basis(similarities: &[f64]) -> Option<usize> {
    let best = similarities.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    similarities.iter().rposition(|&score| score >= best - BASIS_TIE_MARGIN)
}

/// Compare a mod entry against every original version of its file.
pub fn check_mod(archive: &AgeArchive, mod_entry: usize) -> Result<ModCheck, String> {
    // Original-content versions in load order; the newest one the game loads
    // is "latest" (skipped official patches stay in the list as history).
    let mut originals: Vec<usize> = archive
        .version_history(mod_entry)
        .into_iter()
        .filter(|&index| archive.entry_tier(index).is_original_content())
        .collect();
    let read = |index: usize| archive.read_entry(&archive.entries[index]).map_err(|e| format!("{}: {e}", archive.entries[index].name));
    let Some(latest) = originals
        .iter()
        .copied()
        .filter(|&index| archive.entry_inactive_reason(index).is_none())
        .last()
        .or_else(|| originals.last().copied())
    else {
        return Ok(ModCheck { mod_entry, originals, similarities: Vec::new(), verdict: ModVerdict::NewFile });
    };
    // Keep `latest` last so "basis == last" means up to date.
    originals.retain(|&index| index != latest);
    originals.push(latest);

    let mod_bytes = read(mod_entry)?;
    let latest_bytes = read(latest)?;
    if mod_bytes == latest_bytes {
        return Ok(ModCheck { mod_entry, similarities: vec![1.0; originals.len()], originals, verdict: ModVerdict::IdenticalToOriginal });
    }
    let mod_content = comparable_content(mod_bytes);
    let mut similarities = Vec::with_capacity(originals.len());
    for &original in &originals {
        let bytes = if original == latest { latest_bytes.clone() } else { read(original)? };
        similarities.push(similarity(&mod_content, &comparable_content(bytes)));
    }
    let basis = likely_basis(&similarities).unwrap_or(originals.len() - 1);
    let verdict = if basis == originals.len() - 1 {
        ModVerdict::UpToDate
    } else {
        ModVerdict::BasedOnOlderVersion { based_on: originals[basis], latest }
    };
    Ok(ModCheck { mod_entry, originals, similarities, verdict })
}

/// Every mod entry of the archive (including mods shadowed by a later mod).
pub fn mod_entries(archive: &AgeArchive) -> Vec<usize> {
    (0..archive.entries.len()).filter(|&index| archive.entry_tier(index) == EntryTier::Mod).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_with(width: u32, height: u32, changed: &[(u32, u32)]) -> RgbaImage {
        let mut image = RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        for &(x, y) in changed {
            image.put_pixel(x, y, image::Rgba([200, 0, 0, 255]));
        }
        image
    }

    #[test]
    fn image_difference_counts_and_bounds_changes() {
        let difference = image_difference(&image_with(10, 10, &[]), &image_with(10, 10, &[(2, 3), (5, 7)]));
        assert_eq!(difference.changed_pixels, 2);
        assert_eq!(difference.changed_bounds, Some([2, 3, 4, 5]));
        let resized = image_difference(&image_with(10, 10, &[]), &image_with(10, 12, &[]));
        assert!(!resized.same_size);
        assert_eq!(resized.changed_pixels, 20);
    }

    #[test]
    fn basis_prefers_best_match_and_newest_on_ties() {
        assert_eq!(likely_basis(&[0.95, 0.60]), Some(0));
        assert_eq!(likely_basis(&[0.90, 0.90]), Some(1));
        assert_eq!(likely_basis(&[]), None);
    }

    #[test]
    fn similarity_by_content_kind() {
        let text = |s: &str| ComparableContent::Text(s.into());
        assert!(similarity(&text("a\nb\nc\n"), &text("a\nb\nX\n")) > 0.6);
        let image = similarity(&ComparableContent::Image(image_with(4, 4, &[])), &ComparableContent::Image(image_with(4, 4, &[(0, 0)])));
        assert!((image - 15.0 / 16.0).abs() < 1e-9);
        assert_eq!(similarity(&ComparableContent::Binary(vec![1, 2, 3, 4]), &ComparableContent::Binary(vec![1, 2, 9, 4])), 0.75);
        assert_eq!(similarity(&text("x"), &ComparableContent::Binary(vec![1])), 0.0);
    }

    #[test]
    fn quantization_noise_and_hidden_rgb_are_not_changes() {
        let before = RgbaImage::from_pixel(2, 1, image::Rgba([100, 100, 100, 255]));
        let mut after = before.clone();
        after.put_pixel(0, 0, image::Rgba([110, 95, 104, 255])); // palette shift
        assert_eq!(image_difference(&before, &after).changed_pixels, 0);
        let hidden = (RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 0])), RgbaImage::from_pixel(1, 1, image::Rgba([200, 9, 9, 0])));
        assert_eq!(image_difference(&hidden.0, &hidden.1).changed_pixels, 0);
    }

    #[test]
    fn highlight_keeps_changed_pixels_only() {
        let highlighted = highlight_changes(&image_with(2, 1, &[]), &image_with(2, 1, &[(1, 0)]));
        assert_eq!(highlighted.get_pixel(1, 0).0, [200, 0, 0, 255]);
        assert_ne!(highlighted.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }
}
