//! Export lint: semantic sanity checks on a modified instruction stream.
//!
//! Every rule is a DELTA against the original BIN, never an absolute rule:
//! engines differ (e.g. VERITA keeps the presentation gate 0x21B at 0 for the
//! whole scene), so we only flag what the edit made worse. Warn-only — export
//! still proceeds; the corruption guard in `commands::build_translated_bin`
//! remains the hard stop.
//!
//! Semantics per OpenMaidEngine `vm-map/opcodes.toml` (MIT), verified across
//! 709 original scenes in 6 SYS4 games (see `bridge.rs` module docs).

use crate::bin_format::instruction::Instruction;

const OP_SHOW_TEXT: u32 = 0x6E;
const OP_WAIT_FOR_INPUT: u32 = 0x72;
const OP_RESET_TEXT_LAYOUT: u32 = 0x71;
const OP_SET_TICK_PRESENTATION: u32 = 0x21B;
const OP_STMT_BEGIN: u32 = 0x1F4;
const OP_STMT_END: u32 = 0x1F5;
/// Same bound the bridge detector uses for a wait→show transition window.
const MAX_TRANSITION_WINDOW: usize = 60;

/// Linear-scan metrics of one instruction stream (control flow ignored — box
/// transitions and our inserts are straight-line code).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PresentationMetrics {
    /// show-text executed while the last 0x21B in program order set it to 0.
    pub show_text_while_hidden: usize,
    /// First such show-text's visible string (for the warning message).
    pub first_hidden_text: Option<String>,
    /// count(0x1F4) - count(0x1F5).
    pub statement_block_balance: i64,
    /// wait-for-input → next show-text windows that never clear a layout (0x71).
    pub transitions_without_box_clear: usize,
}

fn show_text_string(instruction: &Instruction) -> Option<String> {
    instruction
        .arguments
        .iter()
        .rev()
        .find_map(|a| a.decoded_string.clone())
        .filter(|s| !s.trim().is_empty())
}

/// Pure: compute the metrics for a stream.
pub fn presentation_metrics(instructions: &[Instruction]) -> PresentationMetrics {
    let mut metrics = PresentationMetrics::default();
    let mut presentation_enabled = true;
    for (index, instruction) in instructions.iter().enumerate() {
        match instruction.opcode {
            OP_SET_TICK_PRESENTATION => {
                presentation_enabled = instruction.arguments.first().map_or(true, |a| a.raw_data != 0);
            }
            OP_SHOW_TEXT if !presentation_enabled => {
                metrics.show_text_while_hidden += 1;
                if metrics.first_hidden_text.is_none() {
                    metrics.first_hidden_text = show_text_string(instruction);
                }
            }
            OP_STMT_BEGIN => metrics.statement_block_balance += 1,
            OP_STMT_END => metrics.statement_block_balance -= 1,
            OP_WAIT_FOR_INPUT => {
                let window_end = (index + 1 + MAX_TRANSITION_WINDOW).min(instructions.len());
                let window = &instructions[index + 1..window_end];
                if let Some(show_offset) = window.iter().position(|i| i.opcode == OP_SHOW_TEXT) {
                    if !window[..show_offset].iter().any(|i| i.opcode == OP_RESET_TEXT_LAYOUT) {
                        metrics.transitions_without_box_clear += 1;
                    }
                }
            }
            _ => {}
        }
    }
    metrics
}

/// Pure: human-readable warnings for regressions the edit introduced.
pub fn lint_export(original: &[Instruction], modified: &[Instruction]) -> Vec<String> {
    let before = presentation_metrics(original);
    let after = presentation_metrics(modified);
    let mut warnings = Vec::new();

    // The gate only governs text visibility in engines whose originals NEVER
    // show text with it off (survey: 0 in LaDEA/Madou/Conquista/MZ/Sankai,
    // 100% in VERITA, which runs a different draw mode). If the original already
    // does it, 0x21B evidently isn't a visibility gate here → skip the rule.
    let gate_controls_visibility = before.show_text_while_hidden == 0;
    if gate_controls_visibility && after.show_text_while_hidden > 0 {
        let added = after.show_text_while_hidden;
        let sample = after
            .first_hidden_text
            .as_deref()
            .map(|t| format!(" (first: \"{}\")", t.chars().take(40).collect::<String>()))
            .unwrap_or_default();
        warnings.push(format!(
            "{added} text line(s) now render while presentation is off (0x21B 0) — they'd be invisible until the backlog{sample}. Likely a scrambled or partial box bridge."
        ));
    }
    if after.statement_block_balance != before.statement_block_balance {
        warnings.push(format!(
            "Statement blocks unbalanced by the edit (0x1F4/0x1F5 net {} → {}).",
            before.statement_block_balance, after.statement_block_balance
        ));
    }
    if after.transitions_without_box_clear > before.transitions_without_box_clear {
        let added = after.transitions_without_box_clear - before.transitions_without_box_clear;
        warnings.push(format!(
            "{added} click-wait(s) continue into text without clearing the box (no 0x71) — the next text would pile onto the previous box."
        ));
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bin_format::instruction::Argument;

    fn ins(opcode: u32, values: &[u32]) -> Instruction {
        Instruction {
            opcode,
            label: String::new(),
            argument_count: values.len() as u32,
            arguments: values
                .iter()
                .map(|&raw_data| Argument { arg_type: 0, raw_data, decoded_string: None, original_encoded_bytes: None, data_array: None })
                .collect(),
            offset: 0,
        }
    }

    /// wait, ETL, [presentation off], clear 1, block{ [presentation on] }, text
    fn native_box(gate: bool) -> Vec<Instruction> {
        let mut v = vec![ins(OP_WAIT_FOR_INPUT, &[0]), ins(0x6F, &[0])];
        if gate { v.push(ins(OP_SET_TICK_PRESENTATION, &[0])); }
        v.push(ins(OP_RESET_TEXT_LAYOUT, &[1]));
        v.push(ins(OP_STMT_BEGIN, &[]));
        if gate { v.push(ins(OP_SET_TICK_PRESENTATION, &[1])); }
        v.push(ins(OP_STMT_END, &[]));
        v.push(ins(OP_SHOW_TEXT, &[0, 0]));
        v
    }

    #[test]
    fn clean_bridge_insert_has_no_warnings() {
        let original = native_box(true);
        let mut modified = original.clone();
        modified.extend(native_box(true));
        assert!(lint_export(&original, &modified).is_empty());
    }

    #[test]
    fn scrambled_bridge_is_flagged() {
        // The LaDEA bug: gate args swapped → ends with presentation off.
        let original = native_box(true);
        let mut scrambled = native_box(true);
        scrambled[2] = ins(OP_SET_TICK_PRESENTATION, &[1]);
        scrambled[5] = ins(OP_SET_TICK_PRESENTATION, &[0]);
        let mut modified = original.clone();
        modified.extend(scrambled);
        let warnings = lint_export(&original, &modified);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("0x21B"));
    }

    #[test]
    fn engine_without_gate_is_not_flagged() {
        // VERITA-style: presentation off for the whole scene in the original too.
        let mut original = vec![ins(OP_SET_TICK_PRESENTATION, &[0])];
        original.extend(native_box(false));
        let mut modified = original.clone();
        modified.extend(native_box(false));
        assert!(lint_export(&original, &modified).is_empty());
    }

    #[test]
    fn unbalanced_block_and_missing_clear_are_flagged() {
        let original = native_box(true);
        let mut modified = original.clone();
        modified.extend([ins(OP_WAIT_FOR_INPUT, &[0]), ins(OP_STMT_END, &[]), ins(OP_SHOW_TEXT, &[0, 0])]);
        let warnings = lint_export(&original, &modified);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }
}
