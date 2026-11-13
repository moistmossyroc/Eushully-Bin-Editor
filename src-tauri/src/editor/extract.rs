use crate::bin_format::instruction::Instruction;
use crate::bin_format::opcodes::{CONTEXT_OPCODES, DIALOGUE_OPCODES};
use serde::{Deserialize, Serialize};

/// A single entry in the editor table, representing a dialogue-relevant instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogueEntry {
    /// Index of this instruction in the full instruction list.
    /// Negative values indicate visually-inserted entries (handled by insertedInstructions).
    pub instruction_index: i64,
    /// Opcode value
    pub opcode: u32,
    /// Human-readable label
    pub label: String,
    /// Original Japanese text (from string arguments)
    pub original_text: String,
    /// Translated English text (user-editable)
    pub translated_text: String,
    /// Notes/comments (user-editable)
    pub notes: String,
    /// Whether this is a context-only row (not editable for translation)
    pub is_context_row: bool,
    /// Which argument indices contain strings (for applying translations back)
    pub string_arg_indices: Vec<usize>,
    /// Whether this entry has been marked for deletion
    pub deleted: bool,
    /// When true, write empty strings to this instruction's string args at
    /// export time (instead of leaving the original text). Used for both
    /// auto-omitted group-translation siblings and user-toggled blanks on
    /// display-furigana rows.
    #[serde(default)]
    pub omit_export: bool,
}

/// Extract dialogue-relevant instructions from a parsed instruction list.
pub fn extract_dialogue(instructions: &[Instruction]) -> Vec<DialogueEntry> {
    let mut entries = Vec::new();

    for (idx, instr) in instructions.iter().enumerate() {
        let is_dialogue = DIALOGUE_OPCODES.contains(&instr.opcode);
        let is_context = CONTEXT_OPCODES.contains(&instr.opcode);

        if !is_dialogue && !is_context {
            continue;
        }

        let string_arg_indices: Vec<usize> = instr
            .arguments
            .iter()
            .enumerate()
            .filter(|(_, arg)| arg.arg_type == 2)
            .map(|(i, _)| i)
            .collect();

        let original_text = if is_context {
            // For context rows, show the label and raw argument values
            format_context_text(instr)
        } else {
            // For dialogue rows, concatenate all string arguments
            format_dialogue_text(instr)
        };

        entries.push(DialogueEntry {
            instruction_index: idx as i64,
            opcode: instr.opcode,
            label: instr.label.clone(),
            original_text,
            translated_text: String::new(),
            notes: String::new(),
            is_context_row: is_context,
            string_arg_indices,
            deleted: false,
            omit_export: false,
        });
    }

    entries
}

/// Format dialogue text from string arguments.
/// For show-text (0x6E): both args can be strings (speaker + line).
/// For display-furigana (0x196): text + furigana reading.
/// For others: concatenate all string arguments.
fn format_dialogue_text(instr: &Instruction) -> String {
    let strings: Vec<&str> = instr
        .arguments
        .iter()
        .filter_map(|arg| {
            if arg.arg_type == 2 {
                arg.decoded_string.as_deref()
            } else {
                None
            }
        })
        .collect();

    match instr.opcode {
        0x6E => {
            // show-text: may have speaker name + dialogue
            strings.join(" | ")
        }
        0x196 => {
            // display-furigana: text | furigana
            if strings.len() >= 2 {
                format!("{} [{}]", strings[0], strings[1])
            } else {
                strings.join(" | ")
            }
        }
        _ => strings.join(" | "),
    }
}

/// Format context text for non-editable rows.
fn format_context_text(instr: &Instruction) -> String {
    let args: Vec<String> = instr
        .arguments
        .iter()
        .map(|arg| {
            if let Some(ref s) = arg.decoded_string {
                format!("\"{}\"", s)
            } else {
                format!("0x{:X}", arg.raw_data)
            }
        })
        .collect();

    if args.is_empty() {
        instr.label.clone()
    } else {
        format!("{} {}", instr.label, args.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bin_format::parser::parse_bin;
    use std::path::Path;

    /// A sample game BIN next to the crate (dev machine only; tests skip when absent so the
    /// public repo, which ships no game data, stays green).
    fn load_test_bin(name: &str) -> Option<Vec<u8>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(name);
        std::fs::read(&path).ok()
    }

    #[test]
    fn test_extract_dialogue_sys4() {
        let Some(data) = load_test_bin("SC1310.BIN") else { return };
        let parsed = parse_bin(&data).unwrap();
        let entries = extract_dialogue(&parsed.instructions);

        assert!(!entries.is_empty(), "No dialogue entries extracted");

        // Should have some show-text entries
        let show_text_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.opcode == 0x6E && !e.is_context_row)
            .collect();
        assert!(
            !show_text_entries.is_empty(),
            "No show-text entries found"
        );

        // Most show-text entries should have text (some may have empty speaker names)
        let non_empty_count = show_text_entries
            .iter()
            .filter(|e| !e.original_text.is_empty())
            .count();
        assert!(
            non_empty_count > 0,
            "All show-text entries have empty text"
        );
    }

    #[test]
    fn test_extract_dialogue_sys5() {
        let Some(data) = load_test_bin("SC2230.BIN") else { return };
        let parsed = parse_bin(&data).unwrap();
        let entries = extract_dialogue(&parsed.instructions);

        assert!(!entries.is_empty(), "No dialogue entries extracted");
    }
}
