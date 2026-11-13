//! Message-window art as the system script places it (game-accurate dialog preview).
//!
//! Engine facts (docs/engine/age-sys.md §7b, Atelial SYSTEM4): `0x1f9 set-texture
//! (catalog_index, texture, …)` binds an AGF (by master-catalog index) to a texture
//! slot; `0x1fb draw-texture (sprite, texture, src_x, src_y, w, h, dst_x, dst_y)` puts
//! an atlas rectangle on screen. The message band, name-plate frame and the
//! Q.SAVE…✕ button row are such sprites. Variant draws (alternate frame, highlighted
//! buttons) reuse a placement; the first draw per placement is the resting look.

use crate::bin_format::instruction::Instruction;

const OP_SET_TEXTURE: u32 = 0x1f9;
const OP_DRAW_TEXTURE: u32 = 0x1fb;
/// Vertical reach above/below the body text surface that still counts as box art.
const BOX_ART_MARGIN: i64 = 64;

/// One atlas rectangle drawn on screen (canvas pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxArtSprite {
    pub texture: u32,
    pub src_x: u32,
    pub src_y: u32,
    pub width: u32,
    pub height: u32,
    pub dst_x: i64,
    pub dst_y: i64,
}

fn immediate(instruction: &Instruction, index: usize) -> Option<u32> {
    instruction.arguments.get(index).filter(|a| a.arg_type == 0).map(|a| a.raw_data)
}

/// Texture slot → master-catalog index, from `set-texture` with literal arguments.
pub fn texture_bindings(system: &[Instruction]) -> Vec<(u32, u32)> {
    system
        .iter()
        .filter(|i| i.opcode == OP_SET_TEXTURE)
        .filter_map(|i| Some((immediate(i, 1)?, immediate(i, 0)?)))
        .collect()
}

/// Sprites drawn from bound atlases near the body text surface (`body_top..body_bottom`),
/// one per placement (first draw wins), in draw order.
pub fn box_art(system: &[Instruction], body_top: i64, body_bottom: i64) -> Vec<BoxArtSprite> {
    let bound: Vec<u32> = texture_bindings(system).into_iter().map(|(texture, _)| texture).collect();
    let mut sprites: Vec<BoxArtSprite> = Vec::new();
    for draw in system.iter().filter(|i| i.opcode == OP_DRAW_TEXTURE) {
        let values = [1, 2, 3, 4, 5, 6, 7].map(|k| immediate(draw, k));
        let [Some(texture), Some(src_x), Some(src_y), Some(width), Some(height), Some(dst_x), Some(dst_y)] = values else { continue };
        if !bound.contains(&texture) || width == 0 || height == 0 {
            continue;
        }
        let sprite = BoxArtSprite { texture, src_x, src_y, width, height, dst_x: dst_x as i64, dst_y: dst_y as i64 };
        let near_box = sprite.dst_y < body_bottom + BOX_ART_MARGIN && sprite.dst_y + height as i64 > body_top - BOX_ART_MARGIN;
        if near_box && !sprites.iter().any(|s| same_placement(s, &sprite)) {
            sprites.push(sprite);
        }
    }
    sprites
}

/// An alternate look / state overlay of an element already placed: same spot and
/// size, or the two cover at least half of EACH other (AUTO/SKIP "on" icons). A small
/// button resting on the edge of the big band is not a variant of it.
fn same_placement(a: &BoxArtSprite, b: &BoxArtSprite) -> bool {
    if (a.dst_x - b.dst_x).abs() <= 6 && (a.dst_y - b.dst_y).abs() <= 6 && a.width.abs_diff(b.width) <= 16 {
        return true;
    }
    let overlap_w = (a.dst_x + a.width as i64).min(b.dst_x + b.width as i64) - a.dst_x.max(b.dst_x);
    let overlap_h = (a.dst_y + a.height as i64).min(b.dst_y + b.height as i64) - a.dst_y.max(b.dst_y);
    if overlap_w <= 0 || overlap_h <= 0 {
        return false;
    }
    let larger = (a.width as i64 * a.height as i64).max(b.width as i64 * b.height as i64);
    overlap_w * overlap_h * 2 >= larger
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bin_format::instruction::Argument;

    fn op(opcode: u32, values: &[u32]) -> Instruction {
        Instruction {
            opcode,
            label: String::new(),
            argument_count: values.len() as u32,
            arguments: values
                .iter()
                .map(|&v| Argument { arg_type: 0, raw_data: v, decoded_string: None, original_encoded_bytes: None, data_array: None })
                .collect(),
            offset: 0,
        }
    }

    /// Atelial SYSTEM4: atlas SO001 (catalog 21253) on texture 17; band, alternate band,
    /// Q.SAVE + its highlight, a far-away title sprite.
    fn atelial() -> Vec<Instruction> {
        vec![
            op(OP_SET_TEXTURE, &[21253, 17, 0]),
            op(OP_DRAW_TEXTURE, &[44000, 17, 77, 411, 890, 102, 67, 451]),
            op(OP_DRAW_TEXTURE, &[44001, 17, 77, 302, 890, 108, 67, 448]),
            op(OP_DRAW_TEXTURE, &[44300, 17, 113, 558, 44, 27, 579, 542]),
            op(OP_DRAW_TEXTURE, &[44301, 17, 113, 586, 44, 27, 579, 542]),
            op(OP_DRAW_TEXTURE, &[50000, 17, 0, 0, 100, 40, 10, 20]),
            op(OP_DRAW_TEXTURE, &[50001, 99, 0, 0, 100, 40, 67, 451]), // unbound texture
            op(OP_SET_TEXTURE, &[43, 62, 0]),
            op(OP_DRAW_TEXTURE, &[50002, 62, 0, 0, 64, 32, 569, 539]), // "on" overlay over Q.SAVE
        ]
    }

    #[test]
    fn binds_textures_to_catalog_entries() {
        assert_eq!(texture_bindings(&atelial()), vec![(17, 21253), (62, 43)]);
    }

    #[test]
    fn keeps_resting_box_art_near_the_body() {
        let art = box_art(&atelial(), 423, 543);
        assert_eq!(art.len(), 2); // band + resting Q.SAVE (alternate band, highlight, overlay dropped)
        assert_eq!((art[0].src_y, art[0].dst_y), (411, 451)); // first band, not the alternate
        assert_eq!((art[1].src_y, art[1].dst_x), (558, 579)); // resting Q.SAVE, not highlighted
    }
}
