use crate::bin_format::instruction::{Argument, Instruction};
use crate::editor::extract::DialogueEntry;
use crate::project::types::InsertedInstruction;

/// Apply translations and structural modifications to the full instruction list.
///
/// Takes the original instruction list, dialogue entries (with translations),
/// inserted instructions, and produces a new instruction list ready for writing.
pub fn apply_changes(
    original_instructions: &[Instruction],
    dialogue_entries: &[DialogueEntry],
    inserted_instructions: &[InsertedInstruction],
) -> Vec<Instruction> {
    let mut instructions = original_instructions.to_vec();

    // Template for converting a display-furigana (0x196) whose translation is
    // prose (no "[reading]" suffix) into a show-text (0x6E) — see the 0x196 arm
    // in apply_translation_to_instruction. Clone the first real show-text's args
    // so the converted instruction gets a valid 2-arg shape (one string slot).
    let show_text_template: Option<Vec<Argument>> = original_instructions
        .iter()
        .find(|i| i.opcode == 0x6E && i.arguments.iter().any(|a| a.arg_type == 2))
        .map(|i| i.arguments.clone());

    // Phase 1: Apply translations to existing instructions.
    // We match dialogue entries back to instructions by instruction_index.
    // Skip entries with negative instruction_index (visually-inserted, handled by insertedInstructions).
    //
    // Empty translated_text usually means "leave the original alone".
    // Exception: entries flagged with `omit_export = true` must have their
    // string args cleared so the original kanji/text doesn't render in-game.
    // This flag is set by:
    //   - Group translation (auto-marks sibling rows whose anchor carries the
    //     full translation)
    //   - User toggling the "Include in export" checkbox off on a
    //     display-furigana row
    for entry in dialogue_entries {
        if entry.instruction_index < 0 || entry.deleted || entry.is_context_row {
            continue;
        }
        if entry.translated_text.is_empty() && !entry.omit_export {
            continue;
        }

        if let Some(instr) = instructions.get_mut(entry.instruction_index as usize) {
            if entry.omit_export {
                // Clear ALL string args on this instruction.
                for arg in &mut instr.arguments {
                    if arg.arg_type == 2 {
                        arg.decoded_string = Some(String::new());
                        arg.original_encoded_bytes = None;
                    }
                }
            } else {
                apply_translation_to_instruction(instr, entry, show_text_template.as_deref());
            }
        }
    }

    // Phase 2: one merge pass over ORIGINAL indices — keep or drop each original
    // instruction, then emit the insertions anchored after it. Both deletions
    // and `after_index` refer to original indices, so neither can shift the
    // other. (The old remove-then-insert passes applied original-index
    // insertions to the post-deletion list → every insert after a deleted row
    // landed one slot early per deletion.)
    let deleted: std::collections::HashSet<usize> = dialogue_entries
        .iter()
        .filter(|e| e.deleted && e.instruction_index >= 0)
        .map(|e| e.instruction_index as usize)
        .collect();

    // Group insertions by anchor, keeping list order. The frontend contract
    // (resolveInsertionsForExport) lists each group in REVERSE visual order —
    // it was written for back-to-front insertion — so emit each group reversed.
    let mut insertions_after: std::collections::HashMap<usize, Vec<&InsertedInstruction>> =
        std::collections::HashMap::new();
    for insertion in inserted_instructions {
        insertions_after.entry(insertion.after_index).or_default().push(insertion);
    }

    let mut merged: Vec<Instruction> = Vec::with_capacity(instructions.len() + inserted_instructions.len());
    for (index, instruction) in instructions.into_iter().enumerate() {
        if !deleted.contains(&index) {
            merged.push(instruction);
        }
        if let Some(group) = insertions_after.get(&index) {
            merged.extend(group.iter().rev().map(|insertion| build_instruction_from_insertion(insertion)));
        }
    }
    // Anchors past the end of the script have nowhere to go (same as before:
    // they were dropped by the `insert_at <= len` guard).
    let instructions = merged;

    // Original instructions keep their parse-time `offset` values so the
    // writer's label-resolution map (old_offset → new_byte_position) stays
    // correct when control-flow opcodes in the original BIN refer to other
    // original instructions. Newly-inserted instructions carry the sentinel
    // `u64::MAX` (set by build_instruction_from_insertion) so the writer
    // can skip them when building the map.

    instructions
}

/// Apply a translation to a specific instruction's string arguments.
///
/// `show_text_template` is the arg shape of a real show-text (0x6E), used to
/// convert a prose-carrying display-furigana (0x196) into a show-text.
fn apply_translation_to_instruction(
    instr: &mut Instruction,
    entry: &DialogueEntry,
    show_text_template: Option<&[Argument]>,
) {
    match instr.opcode {
        0x6E => {
            // show-text: the translated_text replaces string arguments
            // If there's a pipe separator, split into speaker | line
            let parts: Vec<&str> = entry.translated_text.splitn(2, " | ").collect();
            let mut string_idx = 0;
            for arg in &mut instr.arguments {
                if arg.arg_type == 2 {
                    if string_idx < parts.len() {
                        arg.decoded_string = Some(parts[string_idx].to_string());
                        arg.original_encoded_bytes = None; // Force re-encoding
                    }
                    string_idx += 1;
                }
            }
        }
        0x196 => {
            // display-furigana: text [furigana] format
            // Parse back the bracketed furigana
            let text = &entry.translated_text;
            let (main_text, furigana) = if let Some(bracket_start) = text.rfind(" [") {
                if text.ends_with(']') {
                    (
                        &text[..bracket_start],
                        &text[bracket_start + 2..text.len() - 1],
                    )
                } else {
                    (text.as_str(), "")
                }
            } else {
                (text.as_str(), "")
            };

            // Conditional convert: if the translation is PROSE (no "[reading]"),
            // this furigana entry is carrying a plain translated line, not a ruby
            // annotation — rewrite it as a show-text (0x6E) so the engine renders
            // normal dialogue instead of ruby. A real reading (furigana non-empty)
            // is KEPT as 0x196 (ruby preserved); untranslated entries never reach
            // here (skipped upstream). Needs a show-text template for a valid 2-arg
            // shape; without one, fall through to the legacy furigana write.
            if furigana.is_empty() && !main_text.is_empty() {
                if let Some(tmpl) = show_text_template {
                    let mut new_args: Vec<Argument> = tmpl
                        .iter()
                        .map(|a| Argument {
                            arg_type: a.arg_type,
                            raw_data: a.raw_data,
                            decoded_string: a.decoded_string.clone(),
                            original_encoded_bytes: None,
                            data_array: a.data_array.clone(),
                        })
                        .collect();
                    let mut wrote = false;
                    for arg in &mut new_args {
                        if arg.arg_type == 2 {
                            arg.decoded_string = Some(main_text.to_string());
                            arg.original_encoded_bytes = None;
                            wrote = true;
                            break;
                        }
                    }
                    if wrote {
                        instr.opcode = 0x6E;
                        instr.label = "show-text".to_string();
                        instr.argument_count = new_args.len() as u32;
                        instr.arguments = new_args;
                        // instr.offset stays at the parsed value (writer remaps it).
                        return;
                    }
                }
            }

            // Keep as furigana: base → string slot 0, reading → string slot 1.
            let mut string_idx = 0;
            for arg in &mut instr.arguments {
                if arg.arg_type == 2 {
                    match string_idx {
                        0 => {
                            arg.decoded_string = Some(main_text.to_string());
                            arg.original_encoded_bytes = None;
                        }
                        1 => {
                            arg.decoded_string = Some(furigana.to_string());
                            arg.original_encoded_bytes = None;
                        }
                        _ => {}
                    }
                    string_idx += 1;
                }
            }
        }
        _ => {
            // For all other opcodes, replace the first string argument
            for arg in &mut instr.arguments {
                if arg.arg_type == 2 {
                    arg.decoded_string = Some(entry.translated_text.clone());
                    arg.original_encoded_bytes = None; // Force re-encoding
                    break;
                }
            }
        }
    }
}

/// Build a new Instruction from an InsertedInstruction definition.
fn build_instruction_from_insertion(insertion: &InsertedInstruction) -> Instruction {
    let arguments: Vec<Argument> = insertion
        .arguments
        .iter()
        .map(|sa| Argument {
            arg_type: sa.arg_type,
            raw_data: sa.raw_value,
            decoded_string: sa.string_value.clone(),
            original_encoded_bytes: None,
            data_array: None,
        })
        .collect();

    Instruction {
        opcode: insertion.opcode,
        label: insertion.label.clone(),
        argument_count: arguments.len() as u32,
        arguments,
        // u64::MAX is a sentinel meaning "synthetic / no original offset".
        // Writer skips these when building the label-resolution map so they
        // don't collide with real offsets (especially offset 0 = first instr).
        offset: u64::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::types::SerializedArg;

    fn instr(opcode: u32, offset: u64) -> Instruction {
        Instruction { opcode, label: String::new(), argument_count: 0, arguments: Vec::new(), offset }
    }
    fn insertion(after_index: usize, opcode: u32) -> InsertedInstruction {
        InsertedInstruction { after_index, opcode, label: String::new(), arguments: Vec::<SerializedArg>::new(), synthetic_index: None }
    }
    fn deleted_entry(instruction_index: i64) -> DialogueEntry {
        serde_json::from_value(serde_json::json!({
            "instruction_index": instruction_index, "opcode": 0x6E, "label": "show-text",
            "original_text": "", "translated_text": "", "notes": "",
            "is_context_row": false, "string_arg_indices": [], "deleted": true
        }))
        .expect("DialogueEntry shape")
    }
    fn opcodes(list: &[Instruction]) -> Vec<u32> {
        list.iter().map(|i| i.opcode).collect()
    }

    #[test]
    fn insert_after_deleted_row_keeps_its_anchor() {
        // originals: 0xA0(0) 0xA1(1) 0xA2(2) 0xA3(3); delete #1; insert X after #2.
        let originals: Vec<Instruction> = (0..4).map(|i| instr(0xA0 + i as u32, i)).collect();
        let result = apply_changes(&originals, &[deleted_entry(1)], &[insertion(2, 0xEE)]);
        assert_eq!(opcodes(&result), vec![0xA0, 0xA2, 0xEE, 0xA3]);
    }

    #[test]
    fn group_is_emitted_in_reverse_list_order() {
        // Frontend contract: same-anchor group arrives in REVERSE visual order.
        let originals: Vec<Instruction> = (0..2).map(|i| instr(0xA0 + i as u32, i)).collect();
        let reverse_visual = [insertion(0, 0xC3), insertion(0, 0xC2), insertion(0, 0xC1)];
        let result = apply_changes(&originals, &[], &reverse_visual);
        assert_eq!(opcodes(&result), vec![0xA0, 0xC1, 0xC2, 0xC3, 0xA1]);
    }

    #[test]
    fn insertion_after_a_deleted_anchor_still_lands_in_place() {
        let originals: Vec<Instruction> = (0..3).map(|i| instr(0xA0 + i as u32, i)).collect();
        let result = apply_changes(&originals, &[deleted_entry(1)], &[insertion(1, 0xEE)]);
        assert_eq!(opcodes(&result), vec![0xA0, 0xEE, 0xA2]);
    }
}