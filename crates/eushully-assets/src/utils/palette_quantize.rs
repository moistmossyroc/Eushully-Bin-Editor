// palette_quantize.rs — RGBA → ≤256-colour palette + indices, for writing
// edited images back into 8bpp AGFs.
//
// Images with ≤256 distinct RGB colours get an exact palette (lossless, e.g.
// an untouched or lightly retouched original). Anything richer goes through
// exoquant's k-means, optionally Floyd–Steinberg dithered. Alpha is ignored
// here: AGF keeps it in the separate ACIF plane.

use std::collections::HashMap;

pub const MAX_PALETTE_COLORS: usize = 256;

pub struct PalettedImage {
    pub palette: Vec<[u8; 3]>,
    /// One index per pixel, same order as the input.
    pub indices: Vec<u8>,
    /// True when the palette reproduces every pixel exactly.
    pub exact: bool,
}

/// Exact palette if the image has ≤256 RGB colours, `None` otherwise.
pub fn exact_palette(rgba: &[u8]) -> Option<PalettedImage> {
    let mut slot_by_color: HashMap<[u8; 3], u8> = HashMap::new();
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut indices = Vec::with_capacity(rgba.len() / 4);
    for pixel in rgba.chunks_exact(4) {
        let color = [pixel[0], pixel[1], pixel[2]];
        let slot = match slot_by_color.get(&color) {
            Some(&slot) => slot,
            None => {
                if palette.len() == MAX_PALETTE_COLORS {
                    return None;
                }
                let slot = palette.len() as u8;
                palette.push(color);
                slot_by_color.insert(color, slot);
                slot
            }
        };
        indices.push(slot);
    }
    Some(PalettedImage { palette, indices, exact: true })
}

/// k-means quantization to 256 colours (`width` is needed for dithering).
pub fn quantized_palette(rgba: &[u8], width: usize, dither: bool) -> PalettedImage {
    use exoquant::{convert_to_indexed, ditherer, optimizer, Color};
    let pixels: Vec<Color> = rgba.chunks_exact(4).map(|p| Color::new(p[0], p[1], p[2], 255)).collect();
    let (palette, indices) = if dither {
        convert_to_indexed(&pixels, width, MAX_PALETTE_COLORS, &optimizer::KMeans, &ditherer::FloydSteinberg::new())
    } else {
        convert_to_indexed(&pixels, width, MAX_PALETTE_COLORS, &optimizer::KMeans, &ditherer::None)
    };
    PalettedImage { palette: palette.iter().map(|c| [c.r, c.g, c.b]).collect(), indices, exact: false }
}

/// Exact when possible, quantized otherwise.
pub fn palettize(rgba: &[u8], width: usize, dither: bool) -> PalettedImage {
    exact_palette(rgba).unwrap_or_else(|| quantized_palette(rgba, width, dither))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn few_colours_get_an_exact_palette() {
        let rgba = [1, 2, 3, 255, 9, 9, 9, 0, 1, 2, 3, 128];
        let paletted = palettize(&rgba, 3, false);
        assert!(paletted.exact);
        assert_eq!(paletted.palette, [[1, 2, 3], [9, 9, 9]]);
        assert_eq!(paletted.indices, [0, 1, 0]);
    }

    #[test]
    fn many_colours_are_quantized_to_256() {
        let rgba: Vec<u8> = (0..1024u32).flat_map(|i| [(i % 256) as u8, (i / 4) as u8, (i * 7 % 256) as u8, 255]).collect();
        assert!(exact_palette(&rgba).is_none());
        let paletted = palettize(&rgba, 32, true);
        assert!(!paletted.exact && paletted.palette.len() <= MAX_PALETTE_COLORS);
        assert_eq!(paletted.indices.len(), 1024);
    }
}
