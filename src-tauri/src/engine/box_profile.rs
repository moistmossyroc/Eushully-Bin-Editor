//! ADV text-box profiles: fit more lines per box WITHOUT shrinking the text, by
//! moving the body text up inside its surface, growing the surface slightly and
//! tightening line spacing.
//!
//! Engine facts (docs/engine/age-sys.md §7b): the body box is text layout slot 1
//! (`0x70 slot,w,h,x,y` = the text SURFACE, which clips glyphs at its edges) and its
//! text origin inside that surface is `0x79 slot,x,y`; both are defined once in the
//! system script. The ADV font is `0x75 px` and line spacing `0x8b px` (engine
//! default 6). Scenes re-assert the ADV font + spacing pair at their start, alongside
//! other font sizes used for names/UI — so only spacing sites that follow the ADV
//! font size are changed. Moving the whole layout (`0x70` y) gains no room: the
//! surface's bottom edge moves with it (Atelial 3-line test, 2026-11-11).

use crate::bin_format::instruction::Instruction;

const OP_TEXT_LAYOUT: u32 = 0x70;
const OP_TEXT_ORIGIN: u32 = 0x79;
const OP_FONT_SIZE: u32 = 0x75;
const OP_LINE_SPACING: u32 = 0x8b;
const BODY_LAYOUT_SLOT: u32 = 1;
/// Engine default extra line spacing when a game never sets `0x8b`.
pub const ENGINE_DEFAULT_LINE_SPACING: u32 = 6;
/// How far before a spacing op the paired ADV font-size op may sit.
const FONT_SPACING_WINDOW: usize = 10;

/// A game's native ADV body box, read from its system script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxGeometry {
    /// Text surface (layout slot 1).
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    /// Text origin inside the surface (`0x79` slot 1; 0,0 when the game never sets it).
    pub text_x: u32,
    pub text_y: u32,
    /// False when the game never sets `0x79` for slot 1: the origin can't be moved in place.
    pub origin_patchable: bool,
    pub font_px: u32,
    pub line_spacing: u32,
    /// False when the game never sets 0x8b (SYS3 324): spacing can't be patched in place.
    pub spacing_patchable: bool,
}

impl BoxGeometry {
    /// Vertical room for text: from the text origin to the surface's bottom edge.
    pub fn text_height(&self) -> u32 {
        self.height.saturating_sub(self.text_y)
    }

    /// Lines of ADV text the native box holds.
    pub fn native_lines(&self) -> u32 {
        lines_that_fit(self.text_height(), self.font_px, self.line_spacing)
    }
}

/// Requested change. Font size is deliberately not part of it (text stays the same size).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxProfile {
    /// Move the text up inside its surface by this many pixels (`0x79` y).
    pub lift_px: u32,
    /// Grow the text surface downwards by this many pixels (`0x70` height).
    pub grow_px: u32,
    /// New line spacing for ADV text.
    pub line_spacing: u32,
}

/// Lines of ADV text the box holds once `profile` is applied.
pub fn profile_lines(geometry: &BoxGeometry, profile: &BoxProfile) -> u32 {
    let lift = if geometry.origin_patchable { profile.lift_px.min(geometry.text_y) } else { 0 };
    let room = geometry.text_height() + lift + profile.grow_px;
    let spacing = if geometry.spacing_patchable { profile.line_spacing } else { geometry.line_spacing };
    lines_that_fit(room, geometry.font_px, spacing)
}

fn immediate(instruction: &Instruction, index: usize) -> Option<u32> {
    instruction.arguments.get(index).filter(|a| a.arg_type == 0).map(|a| a.raw_data)
}

fn is_body_op(instruction: &Instruction, opcode: u32) -> bool {
    instruction.opcode == opcode && immediate(instruction, 0) == Some(BODY_LAYOUT_SLOT)
}

/// Read the body box geometry from a system script (SYSTEM3/4/5.BIN).
pub fn detect_geometry(system: &[Instruction]) -> Option<BoxGeometry> {
    let layout = system.iter().find(|i| is_body_op(i, OP_TEXT_LAYOUT))?;
    let [width, height, x, y] = [1, 2, 3, 4].map(|k| immediate(layout, k));
    let origin = system
        .iter()
        .find(|i| is_body_op(i, OP_TEXT_ORIGIN))
        .and_then(|i| Some((immediate(i, 1)?, immediate(i, 2)?)));
    let font_index = system.iter().position(|i| i.opcode == OP_FONT_SIZE && immediate(i, 0).is_some())?;
    let font_px = immediate(&system[font_index], 0)?;
    let spacing = system[font_index..]
        .iter()
        .take(FONT_SPACING_WINDOW + 1)
        .find(|i| i.opcode == OP_LINE_SPACING)
        .and_then(|i| immediate(i, 0));
    Some(BoxGeometry {
        width: width?,
        height: height?,
        x: x?,
        y: y?,
        text_x: origin.map(|o| o.0).unwrap_or(0),
        text_y: origin.map(|o| o.1).unwrap_or(0),
        origin_patchable: origin.is_some(),
        font_px,
        line_spacing: spacing.unwrap_or(ENGINE_DEFAULT_LINE_SPACING),
        spacing_patchable: spacing.is_some(),
    })
}

/// One text layout slot (`0x70 slot,w,h,x,y` + its `0x79 slot,x,y` origin). Slot 1 is
/// the ADV body; others serve the speaker plate, history, UI and battle/info text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSlot {
    pub slot: u32,
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    pub text_x: u32,
    pub text_y: u32,
}

/// Every text layout slot the system script defines (first definition per slot).
pub fn text_slots(system: &[Instruction]) -> Vec<TextSlot> {
    let mut slots: Vec<TextSlot> = Vec::new();
    for layout in system.iter().filter(|i| i.opcode == OP_TEXT_LAYOUT) {
        let values = [0, 1, 2, 3, 4].map(|k| immediate(layout, k));
        let [Some(slot), Some(width), Some(height), Some(x), Some(y)] = values else { continue };
        if slots.iter().any(|s| s.slot == slot) {
            continue;
        }
        let origin = system
            .iter()
            .find(|i| is_body_op_for(i, OP_TEXT_ORIGIN, slot))
            .and_then(|i| Some((immediate(i, 1)?, immediate(i, 2)?)))
            .unwrap_or((0, 0));
        slots.push(TextSlot { slot, width, height, x, y, text_x: origin.0, text_y: origin.1 });
    }
    slots.sort_by_key(|s| s.slot);
    slots
}

/// The first font size a script sets and the spacing paired with it (scene preambles
/// re-assert theirs; SN / battle / UI scripts may use other sizes than the ADV text).
pub fn script_font(script: &[Instruction]) -> Option<(u32, Option<u32>)> {
    let index = script.iter().position(|i| i.opcode == OP_FONT_SIZE && immediate(i, 0).is_some())?;
    let spacing = script[index..]
        .iter()
        .take(FONT_SPACING_WINDOW + 1)
        .find(|i| i.opcode == OP_LINE_SPACING)
        .and_then(|i| immediate(i, 0));
    Some((immediate(&script[index], 0)?, spacing))
}

fn is_body_op_for(instruction: &Instruction, opcode: u32, slot: u32) -> bool {
    instruction.opcode == opcode && immediate(instruction, 0) == Some(slot)
}

/// Indices of `0x8b` ops that set the native ADV spacing right after the ADV font size.
fn adv_spacing_sites(script: &[Instruction], geometry: &BoxGeometry) -> Vec<usize> {
    (0..script.len())
        .filter(|&k| script[k].opcode == OP_LINE_SPACING && immediate(&script[k], 0) == Some(geometry.line_spacing))
        .filter(|&k| {
            // The NEAREST preceding font-size op (within the window) must be the ADV font:
            // a name/UI font set in between owns this spacing op.
            script[k.saturating_sub(FONT_SPACING_WINDOW)..k]
                .iter()
                .rev()
                .find(|i| i.opcode == OP_FONT_SIZE)
                .map(|i| immediate(i, 0) == Some(geometry.font_px))
                .unwrap_or(false)
        })
        .collect()
}

fn set_spacing(script: &mut [Instruction], geometry: &BoxGeometry, profile: &BoxProfile) -> usize {
    let sites = adv_spacing_sites(script, geometry);
    for &k in &sites {
        script[k].arguments[0].raw_data = profile.line_spacing;
    }
    sites.len()
}

/// Minimum number of identical slots that make up the history (backlog) entries.
const MIN_HISTORY_ENTRIES: usize = 3;

/// The backlog's entry slots: the largest group (at least 3) of slots other than the body
/// that share one surface size and text origin — history redraws each retained message into
/// one of them (`HISTORY.BIN` op `0x82`; Atelial slots 2–5, Megami Zero 2–7, Tenmei 4–8).
pub fn history_slots(slots: &[TextSlot]) -> Vec<TextSlot> {
    let shape = |s: &TextSlot| (s.width, s.height, s.text_x, s.text_y);
    let candidates: Vec<&TextSlot> = slots.iter().filter(|s| s.slot != BODY_LAYOUT_SLOT).collect();
    candidates
        .iter()
        .map(|s| candidates.iter().filter(|o| shape(o) == shape(s)).map(|o| **o).collect::<Vec<TextSlot>>())
        .filter(|group| group.len() >= MIN_HISTORY_ENTRIES)
        .max_by_key(|group| group.len())
        .unwrap_or_default()
}

/// New height and text origin y for one history slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistorySlotFit {
    pub slot: u32,
    pub height: u32,
    pub text_y: u32,
}

/// Fit history entries to the profile's line count. In-game (Atelial, 2026-11-12) the 3rd row
/// of a 3-line box was clipped in history although the body showed it, so history rows are
/// sized at the LARGER of the native and profile spacing. Each slot first grows into the gap
/// before the next stacked entry (keeps text aligned with its face/frame art), then the rest
/// lifts the text origin. Empty when the entries already hold the lines.
pub fn fit_history_slots(slots: &[TextSlot], geometry: &BoxGeometry, profile: &BoxProfile) -> Vec<HistorySlotFit> {
    let lines = profile_lines(geometry, profile);
    if lines <= geometry.native_lines() {
        return Vec::new();
    }
    let spacing = geometry.line_spacing.max(profile.line_spacing);
    let needed = lines * geometry.font_px + (lines - 1) * spacing;
    let entries = history_slots(slots);
    let Some(first) = entries.first() else { return Vec::new() };
    let room = first.height.saturating_sub(first.text_y);
    let Some(deficit) = needed.checked_sub(room).filter(|d| *d > 0) else { return Vec::new() };
    // Stacked entries (distinct y): free rows between one entry and the next (smallest stride);
    // overlaid entries (same y, placed at draw time): no known gap, lift only.
    let mut tops: Vec<u32> = entries.iter().map(|s| s.y).collect();
    tops.sort_unstable();
    tops.dedup();
    let stride = tops.windows(2).map(|w| w[1] - w[0]).min();
    let gap = stride.map(|s| s.saturating_sub(first.height)).unwrap_or(0);
    let grow = deficit.min(gap);
    let lift = (deficit - grow).min(first.text_y);
    entries
        .iter()
        .map(|entry| HistorySlotFit { slot: entry.slot, height: entry.height + grow, text_y: entry.text_y - lift })
        .collect()
}

/// Patch a system script: ADV spacing, body text origin (lift) and surface height (grow),
/// and the history entries so a taller box isn't clipped in the backlog.
/// Returns the number of instructions changed.
pub fn patch_system_script(script: &mut [Instruction], geometry: &BoxGeometry, profile: &BoxProfile) -> usize {
    let history = fit_history_slots(&text_slots(script), geometry, profile);
    let mut changed = set_spacing(script, geometry, profile);
    for fit in &history {
        if let Some(layout) = script.iter_mut().find(|i| is_body_op_for(i, OP_TEXT_LAYOUT, fit.slot)) {
            if layout.arguments[2].raw_data != fit.height {
                layout.arguments[2].raw_data = fit.height;
                changed += 1;
            }
        }
        if let Some(origin) = script.iter_mut().find(|i| is_body_op_for(i, OP_TEXT_ORIGIN, fit.slot)) {
            if origin.arguments[2].raw_data != fit.text_y {
                origin.arguments[2].raw_data = fit.text_y;
                changed += 1;
            }
        }
    }
    if profile.lift_px > 0 {
        if let Some(origin) = script.iter_mut().find(|i| is_body_op(i, OP_TEXT_ORIGIN)) {
            origin.arguments[2].raw_data = geometry.text_y.saturating_sub(profile.lift_px);
            changed += 1;
        }
    }
    if profile.grow_px > 0 {
        if let Some(layout) = script.iter_mut().find(|i| is_body_op(i, OP_TEXT_LAYOUT)) {
            layout.arguments[2].raw_data = geometry.height + profile.grow_px;
            changed += 1;
        }
    }
    changed
}

/// Patch a scene script: only its re-asserted ADV spacing. Returns sites changed.
pub fn patch_scene_script(script: &mut [Instruction], geometry: &BoxGeometry, profile: &BoxProfile) -> usize {
    set_spacing(script, geometry, profile)
}

/// Lines of `font_px` text that fit in `available_px` with `spacing` between lines.
pub fn lines_that_fit(available_px: u32, font_px: u32, spacing: u32) -> u32 {
    if available_px < font_px {
        return 0;
    }
    1 + (available_px - font_px) / (font_px + spacing)
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

    /// Atelial's SYSTEM4: layout slot 1 800×120 @ 112,423, text origin 78,52; font 22 /
    /// spacing 12; a name font 20 with spacing 12 that must NOT be touched.
    fn atelial_system() -> Vec<Instruction> {
        vec![
            op(OP_TEXT_LAYOUT, &[1, 800, 120, 112, 423]),
            op(OP_TEXT_LAYOUT, &[2, 800, 120, 112, 33]),
            op(OP_TEXT_ORIGIN, &[1, 78, 52]),
            op(OP_TEXT_ORIGIN, &[2, 78, 44]),
            op(OP_FONT_SIZE, &[22]),
            op(OP_LINE_SPACING, &[12]),
            op(OP_FONT_SIZE, &[20]),
            op(OP_LINE_SPACING, &[12]),
        ]
    }

    const ATELIAL_3_LINES: BoxProfile = BoxProfile { lift_px: 3, grow_px: 5, line_spacing: 5 };

    #[test]
    fn detects_body_box() {
        let geometry = detect_geometry(&atelial_system()).unwrap();
        assert_eq!((geometry.width, geometry.height, geometry.x, geometry.y), (800, 120, 112, 423));
        assert_eq!((geometry.text_x, geometry.text_y, geometry.origin_patchable), (78, 52, true));
        assert_eq!((geometry.font_px, geometry.line_spacing, geometry.spacing_patchable), (22, 12, true));
        assert_eq!(geometry.text_height(), 68);
        assert_eq!(geometry.native_lines(), 2); // matches the game
    }

    #[test]
    fn patches_origin_height_and_adv_spacing_only() {
        let mut script = atelial_system();
        let geometry = detect_geometry(&script).unwrap();
        let changed = patch_system_script(&mut script, &geometry, &ATELIAL_3_LINES);
        assert_eq!(changed, 3);
        assert_eq!(script[0].arguments[2].raw_data, 125); // slot 1 surface grown
        assert_eq!(script[0].arguments[4].raw_data, 423); // ...but not moved
        assert_eq!(script[1].arguments[2].raw_data, 120); // slot 2 untouched
        assert_eq!(script[2].arguments[2].raw_data, 49); // slot 1 text lifted
        assert_eq!(script[3].arguments[2].raw_data, 44); // slot 2 origin untouched
        assert_eq!(script[5].arguments[0].raw_data, 5); // ADV spacing
        assert_eq!(script[7].arguments[0].raw_data, 12); // name-font spacing untouched
    }

    #[test]
    fn atelial_profile_holds_three_lines() {
        let geometry = detect_geometry(&atelial_system()).unwrap();
        assert_eq!(profile_lines(&geometry, &ATELIAL_3_LINES), 3);
        // The first test only moved the layout + spacing 6: still 2 lines (as seen in-game).
        assert_eq!(profile_lines(&geometry, &BoxProfile { lift_px: 0, grow_px: 0, line_spacing: 6 }), 2);
    }

    #[test]
    fn games_without_spacing_or_origin_ops() {
        let geometry = detect_geometry(&[op(OP_TEXT_LAYOUT, &[1, 560, 130, 208, 425]), op(OP_FONT_SIZE, &[22])]).unwrap();
        assert_eq!((geometry.line_spacing, geometry.spacing_patchable), (ENGINE_DEFAULT_LINE_SPACING, false));
        assert_eq!((geometry.text_y, geometry.origin_patchable), (0, false));
        // Lift is impossible without 0x79 and spacing stays native: only growth counts.
        assert_eq!(profile_lines(&geometry, &BoxProfile { lift_px: 10, grow_px: 0, line_spacing: 0 }), geometry.native_lines());
    }

    #[test]
    fn lists_every_slot_with_its_origin() {
        let slots = text_slots(&atelial_system());
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0], TextSlot { slot: 1, width: 800, height: 120, x: 112, y: 423, text_x: 78, text_y: 52 });
        assert_eq!((slots[1].slot, slots[1].text_y), (2, 44));
        assert_eq!(script_font(&atelial_system()), Some((22, Some(12))));
    }

    /// Atelial's full slot table (SYSTEM4 ops 5–43): body 1, history 2–5 stacked 130 apart.
    fn atelial_full_system() -> Vec<Instruction> {
        vec![
            op(OP_TEXT_LAYOUT, &[1, 800, 120, 112, 423]),
            op(OP_TEXT_LAYOUT, &[2, 800, 120, 112, 33]),
            op(OP_TEXT_LAYOUT, &[3, 800, 120, 112, 163]),
            op(OP_TEXT_LAYOUT, &[4, 800, 120, 112, 293]),
            op(OP_TEXT_LAYOUT, &[5, 800, 120, 112, 423]),
            op(OP_TEXT_LAYOUT, &[6, 494, 140, 265, 59]),
            op(OP_TEXT_LAYOUT, &[7, 200, 52, 0, 0]),
            op(OP_TEXT_ORIGIN, &[1, 78, 52]),
            op(OP_TEXT_ORIGIN, &[2, 78, 44]),
            op(OP_TEXT_ORIGIN, &[3, 78, 44]),
            op(OP_TEXT_ORIGIN, &[4, 78, 44]),
            op(OP_TEXT_ORIGIN, &[5, 78, 44]),
            op(OP_TEXT_ORIGIN, &[6, 40, 20]),
            op(OP_TEXT_ORIGIN, &[7, 8, 15]),
            op(OP_FONT_SIZE, &[22]),
            op(OP_LINE_SPACING, &[12]),
        ]
    }

    #[test]
    fn history_entries_fit_three_lines() {
        let script = atelial_full_system();
        let slots = text_slots(&script);
        assert_eq!(history_slots(&slots).iter().map(|s| s.slot).collect::<Vec<_>>(), vec![2, 3, 4, 5]);
        let geometry = detect_geometry(&script).unwrap();
        // 3 rows at the native 12 px spacing need 90 px; entries have 76. The 10 px gap to the
        // next entry is used first, then the text moves up 4.
        let fits = fit_history_slots(&slots, &geometry, &ATELIAL_3_LINES);
        assert_eq!(fits.len(), 4);
        assert!(fits.iter().all(|f| (f.height, f.text_y) == (130, 40)), "{fits:?}");
        // No change for a profile that doesn't add lines.
        assert!(fit_history_slots(&slots, &geometry, &BoxProfile { lift_px: 0, grow_px: 0, line_spacing: 12 }).is_empty());

        let mut patched = script.clone();
        let changed = patch_system_script(&mut patched, &geometry, &ATELIAL_3_LINES);
        assert_eq!(changed, 3 + 8); // body origin/height/spacing + 4 × (height, origin)
        let slot = |n: usize| text_slots(&patched)[n];
        assert_eq!((slot(1).height, slot(1).text_y, slot(1).y), (130, 40, 33));
        assert_eq!((slot(5).height, slot(5).text_y), (140, 20)); // other slots untouched
    }

    #[test]
    fn overlaid_history_entries_lift_only() {
        // Tenmei-style: entries all at y 0 (placed at draw time) → no gap, lift only.
        let slots: Vec<TextSlot> = (4..=8).map(|slot| TextSlot { slot, width: 880, height: 60, x: 310, y: 0, text_x: 38, text_y: 20 }).collect();
        let geometry = BoxGeometry { width: 880, height: 148, x: 360, y: 715, text_x: 38, text_y: 15, origin_patchable: true, font_px: 30, line_spacing: 16, spacing_patchable: true };
        let profile = BoxProfile { lift_px: 15, grow_px: 40, line_spacing: 8 };
        assert!(profile_lines(&geometry, &profile) > geometry.native_lines());
        let fits = fit_history_slots(&slots, &geometry, &profile);
        assert!(fits.iter().all(|f| f.height == 60 && f.text_y < 20), "{fits:?}");
    }

    #[test]
    fn line_capacity() {
        assert_eq!(lines_that_fit(68, 22, 12), 2);
        assert_eq!(lines_that_fit(76, 22, 5), 3);
        assert_eq!(lines_that_fit(10, 22, 6), 0);
    }
}
