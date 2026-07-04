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

    // Phase 2: Remove deleted instructions (process in reverse to preserve indices).
    let mut deleted_indices: Vec<usize> = dialogue_entries
        .iter()
        .filter(|e| e.deleted && e.instruction_index >= 0)
        .map(|e| e.instruction_index as usize)
        .collect();
    deleted_indices.sort_unstable();
    deleted_indices.dedup();
    for &idx in deleted_indices.iter().rev() {
        if idx < instructions.len() {
            instructions.remove(idx);
        }
    }

    // Phase 3: Insert new instructions (process in reverse to preserve indices).
    let mut sorted_insertions: Vec<&InsertedInstruction> = inserted_instructions.iter().collect();
    sorted_insertions.sort_by(|a, b| b.after_index.cmp(&a.after_index));

    for insertion in sorted_insertions {
        let insert_at = insertion.after_index + 1;
        // Account for any prior deletions that shifted indices
        // (Since we process deletions first, the after_index in InsertedInstruction
        //  should reference the post-deletion instruction list)
        if insert_at <= instructions.len() {
            let new_instr = build_instruction_from_insertion(insertion);
            instructions.insert(insert_at, new_instr);
        }
    }

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
