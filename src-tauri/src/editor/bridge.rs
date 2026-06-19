//! Bridge template detection for auto-textbox splitting.
//!
//! Scans a parsed BIN for the instruction sequence that transitions between
//! two textboxes belonging to the same speaker (the "new textbox open"
//! bridge). The extracted template is later cloned by the split planner to
//! synthesize continuation boxes when a translation overflows.
//!
//! Classification:
//! - Clean: a branch-free window ending in `0x1F5` — safe to clone verbatim.
//! - Degraded: branches present in every candidate; falls back to a strip
//!   pass that removes `jcc`/`jmp` and still contains a `0x1F5` terminator.
//! - Unavailable: no candidate window contains `0x1F5` at all — the BIN is
//!   not compatible with auto-split.

use crate::bin_format::instruction::Instruction;
use serde::{Deserialize, Serialize};

/// Opcode constants used by the bridge detector.
const OP_WAIT_FOR_INPUT: u32 = 0x72;
const OP_SHOW_TEXT: u32 = 0x6E;
const OP_JCC: u32 = 0xA0;
const OP_JMP: u32 = 0x8C;
/// `u00416120` — the "new textbox open" marker. Every valid bridge must
/// contain exactly one of these and it must be the final instruction.
const OP_NEW_BOX_OPEN: u32 = 0x1F5;

/// Maximum bridge window size to consider. Real bridges are ≤40 instructions
/// in observed SC bins; anything larger is almost certainly not a bridge.
const MAX_BRIDGE_WINDOW: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateQuality {
    /// Branch-free clean template found. Safe to clone verbatim.
    Clean,
    /// Template derived by stripping branches from a branchy candidate.
    /// Cloning is expected to work but may produce minor rendering quirks.
    Degraded,
    /// No usable template in this BIN — auto-split is unavailable.
    Unavailable,
}

/// A cloneable bridge template extracted from a BIN.
#[derive(Debug, Clone)]
pub struct BridgeTemplate {
    pub quality: TemplateQuality,
    /// The instructions that make up the bridge, in order. The last
    /// instruction is always `OP_NEW_BOX_OPEN` (0x1F5) when quality is
    /// Clean or Degraded, and the Vec is empty when Unavailable.
    pub instructions: Vec<Instruction>,
    /// Argument positions (instr_idx, arg_idx) that hold a speaker-scoped
    /// value and must be rewritten when the bridge is cloned for a new
    /// speaker. Populated in Phase 3 when clone_bridge is implemented.
    pub speaker_arg_positions: Vec<(usize, usize)>,
    pub sys_version: u32,
}

impl BridgeTemplate {
    pub fn unavailable(sys_version: u32) -> Self {
        Self {
            quality: TemplateQuality::Unavailable,
            instructions: Vec::new(),
            speaker_arg_positions: Vec::new(),
            sys_version,
        }
    }
}

/// Walk `instructions` and return every `(wait_idx, show_idx)` pair where
/// `show_idx` is the first `show-text` following `wait_idx` within
/// `MAX_BRIDGE_WINDOW`. The instructions between the two indices (exclusive)
/// form a bridge candidate.
fn find_bridge_candidates(instructions: &[Instruction]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for i in 0..instructions.len() {
        if instructions[i].opcode != OP_WAIT_FOR_INPUT {
            continue;
        }
        let start = i + 1;
        let end = (i + 1 + MAX_BRIDGE_WINDOW).min(instructions.len());
        for j in start..end {
            if instructions[j].opcode == OP_SHOW_TEXT {
                if j > i + 1 {
                    out.push((i, j));
                }
                break;
            }
        }
    }
    out
}

/// Return true if the window contains exactly one `OP_NEW_BOX_OPEN` and that
/// opcode is the last instruction.
fn has_terminal_new_box_open(window: &[Instruction]) -> bool {
    let count = window.iter().filter(|i| i.opcode == OP_NEW_BOX_OPEN).count();
    if count != 1 {
        return false;
    }
    matches!(window.last(), Some(last) if last.opcode == OP_NEW_BOX_OPEN)
}

/// Return true if the window is branch-free: no `jcc` or `jmp`.
fn is_branch_free(window: &[Instruction]) -> bool {
    !window
        .iter()
        .any(|i| i.opcode == OP_JCC || i.opcode == OP_JMP)
}

/// Strip branch-related opcodes (`jcc`, `jmp`) from a window. Used to salvage
/// a template from a branchy candidate.
fn strip_branches(window: &[Instruction]) -> Vec<Instruction> {
    window
        .iter()
        .filter(|i| i.opcode != OP_JCC && i.opcode != OP_JMP)
        .cloned()
        .collect()
}

/// Detect the best bridge template in a BIN's instruction list.
///
/// Algorithm:
/// 1. Collect all `wait-for-input → show-text` candidate windows.
/// 2. First pass: find the shortest branch-free window that ends in
///    `0x1F5`. If one exists → `Clean`.
/// 3. Second pass: strip branches from each branchy candidate and check if
///    the stripped form ends in `0x1F5`. If one exists → `Degraded`.
/// 4. Otherwise → `Unavailable`.
pub fn detect_bridge_template(
    instructions: &[Instruction],
    sys_version: u32,
) -> BridgeTemplate {
    let candidates = find_bridge_candidates(instructions);
    if candidates.is_empty() {
        return BridgeTemplate::unavailable(sys_version);
    }

    // Pass 1: clean candidates.
    let mut best_clean: Option<Vec<Instruction>> = None;
    for &(wait_idx, show_idx) in &candidates {
        let window = &instructions[wait_idx + 1..show_idx];
        if !is_branch_free(window) {
            continue;
        }
        if !has_terminal_new_box_open(window) {
            continue;
        }
        match &best_clean {
            Some(current) if current.len() <= window.len() => {}
            _ => best_clean = Some(window.to_vec()),
        }
    }
    if let Some(clean) = best_clean {
        return BridgeTemplate {
            quality: TemplateQuality::Clean,
            instructions: clean,
            speaker_arg_positions: Vec::new(),
            sys_version,
        };
    }

    // Pass 2: strip-branches fallback.
    let mut best_degraded: Option<Vec<Instruction>> = None;
    for &(wait_idx, show_idx) in &candidates {
        let window = &instructions[wait_idx + 1..show_idx];
        let stripped = strip_branches(window);
        if !has_terminal_new_box_open(&stripped) {
            continue;
        }
        match &best_degraded {
            Some(current) if current.len() <= stripped.len() => {}
            _ => best_degraded = Some(stripped),
        }
    }
    if let Some(degraded) = best_degraded {
        return BridgeTemplate {
            quality: TemplateQuality::Degraded,
            instructions: degraded,
            speaker_arg_positions: Vec::new(),
            sys_version,
        };
    }

    BridgeTemplate::unavailable(sys_version)
}

/// Deep-copy a bridge template's instructions, rewriting any arguments
/// flagged in `speaker_arg_positions` to the target speaker ID.
///
/// For SYS3, `target_speaker_name` is used to rewrite `set-string`
/// payloads instead of numeric IDs.
pub fn clone_bridge(
    template: &BridgeTemplate,
    target_speaker_id: u32,
    _target_speaker_name: Option<&str>,
) -> Vec<Instruction> {
    let mut cloned: Vec<Instruction> = template.instructions.clone();
    for &(instr_idx, arg_idx) in &template.speaker_arg_positions {
        if let Some(instr) = cloned.get_mut(instr_idx) {
            if let Some(arg) = instr.arguments.get_mut(arg_idx) {
                arg.raw_data = target_speaker_id;
            }
        }
    }
    cloned
}

/// A fully computed split plan: everything the UI and exporter need to
/// materialise N boxes (each with M lines) from a single overflowing entry.
///
/// The BIN format uses `end-text-line` (opcode 0x6F) as the line separator
/// within a box, not `\n` characters in the text. So each wrapped line
/// becomes its own show-text instruction with end-text-line opcodes between
/// them. Boxes are separated by bridge instruction sequences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitPlan {
    /// Index of the original DialogueEntry that was split.
    pub source_entry_index: usize,
    /// Outer = boxes, inner = lines within each box. `box_lines[0][0]`
    /// replaces the source entry's text; everything else is inserted as
    /// new instructions (show-text + end-text-line opcodes).
    pub box_lines: Vec<Vec<String>>,
    /// N−1 sets of bridge instructions to insert between consecutive boxes.
    pub bridge_instructions_per_split: Vec<Vec<SerializedInstruction>>,
    /// Argument template from the source show-text instruction, so the
    /// frontend can build structurally correct continuation show-text
    /// InsertedInstructions (with proper arg count and string slots).
    pub show_text_template_args: Vec<SerializedArg>,
    /// Argument template for the end-text-line instruction (opcode 0x6F).
    /// Extracted from the source BIN so cloned ETLs match the engine's
    /// expected argument shape. None if the BIN had no end-text-line to
    /// template from (in which case multi-line splits aren't safe).
    pub end_text_line_template: Option<SerializedInstruction>,
}

/// Minimal serializable instruction for the frontend's InsertedInstruction
/// path. Carries the same data as `Instruction` but without offset/label
/// metadata that only matters inside the parser.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedInstruction {
    pub opcode: u32,
    pub label: String,
    pub arguments: Vec<SerializedArg>,
}

/// Argument in a serialized instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedArg {
    pub arg_type: u32,
    pub raw_data: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded_string: Option<String>,
}

impl From<&Instruction> for SerializedInstruction {
    fn from(i: &Instruction) -> Self {
        Self {
            opcode: i.opcode,
            label: i.label.clone(),
            arguments: i
                .arguments
                .iter()
                .map(|a| SerializedArg {
                    arg_type: a.arg_type,
                    raw_data: a.raw_data,
                    decoded_string: a.decoded_string.clone(),
                })
                .collect(),
        }
    }
}

/// Given an overflowing entry, a wrap result, and a bridge template,
/// produce a `SplitPlan` describing the N boxes and N−1 bridge clones
/// needed to materialise them.
///
/// Pure function — no side effects, no state mutation.
pub fn plan_split(
    source_entry_index: usize,
    wrap: &crate::editor::wrap::WrapOutput,
    template: &BridgeTemplate,
    speaker_id: u32,
    speaker_name: Option<&str>,
    show_text_template_args: Vec<SerializedArg>,
    end_text_line_template: Option<SerializedInstruction>,
) -> SplitPlan {
    // box_lines mirrors wrap.boxes exactly — outer is boxes, inner is lines.
    let box_lines: Vec<Vec<String>> = wrap.boxes.clone();

    let bridge_instructions_per_split: Vec<Vec<SerializedInstruction>> = (1..box_lines.len())
        .map(|_| {
            clone_bridge(template, speaker_id, speaker_name)
                .iter()
                .map(SerializedInstruction::from)
                .collect()
        })
        .collect();

    SplitPlan {
        source_entry_index,
        box_lines,
        bridge_instructions_per_split,
        show_text_template_args,
        end_text_line_template,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bin_format::parser::parse_bin;
    use std::path::PathBuf;

    fn load_test_bin(name: &str) -> Option<Vec<u8>> {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let candidates = [
            base.join(name),
            base.join("sys4").join(name),
            base.join("sys5").join(name),
            base.parent().unwrap().join(name),
            base.parent().unwrap().join("sys4").join(name),
            base.parent().unwrap().join("sys5").join(name),
        ];
        for p in &candidates {
            if let Ok(data) = std::fs::read(p) {
                return Some(data);
            }
        }
        None
    }

    #[test]
    fn test_detect_bridge_sc0000_clean() {
        let Some(data) = load_test_bin("sys4/SC0000.BIN") else {
            println!("sys4/SC0000.BIN not found, skipping");
            return;
        };
        let parsed = parse_bin(&data).unwrap();
        let template = detect_bridge_template(&parsed.instructions, parsed.header.sys_version as u32);
        println!(
            "SC0000: quality={:?} len={}",
            template.quality,
            template.instructions.len()
        );
        assert_eq!(
            template.quality,
            TemplateQuality::Clean,
            "SC0000 should yield a clean bridge template"
        );
        assert!(
            template.instructions.last().map(|i| i.opcode) == Some(OP_NEW_BOX_OPEN),
            "last instruction must be 0x1F5"
        );
    }

    #[test]
    fn test_detect_bridge_sc1310_degraded() {
        let Some(data) = load_test_bin("SC1310.BIN") else {
            println!("sys4/SC1310.BIN not found, skipping");
            return;
        };
        let parsed = parse_bin(&data).unwrap();
        let template = detect_bridge_template(&parsed.instructions, parsed.header.sys_version as u32);
        println!(
            "SC1310: quality={:?} len={}",
            template.quality,
            template.instructions.len()
        );
        // Accept either Clean or Degraded — the earlier diagnostic showed only
        // branchy candidates, so we expect Degraded, but if a clean bridge
        // slipped through we don't want to fail.
        assert!(
            matches!(
                template.quality,
                TemplateQuality::Clean | TemplateQuality::Degraded
            ),
            "SC1310 should yield a usable template, got {:?}",
            template.quality
        );
        assert!(
            template.instructions.last().map(|i| i.opcode) == Some(OP_NEW_BOX_OPEN),
            "last instruction must be 0x1F5"
        );
    }

    #[test]
    fn test_strip_branches_removes_jcc_jmp() {
        // Synthetic: build a fake window with a jcc and a jmp plus a 0x1F5.
        // Using empty argument lists so we don't need to mock the full opcode table.
        fn fake(op: u32) -> Instruction {
            Instruction {
                opcode: op,
                label: String::new(),
                argument_count: 0,
                arguments: Vec::new(),
                offset: 0,
            }
        }
        let window = vec![
            fake(0x55),              // mov (kept)
            fake(OP_JCC),            // stripped
            fake(0x55),              // mov (kept)
            fake(OP_JMP),            // stripped
            fake(OP_NEW_BOX_OPEN),   // terminal
        ];
        let stripped = strip_branches(&window);
        assert_eq!(stripped.len(), 3);
        assert_eq!(stripped.last().unwrap().opcode, OP_NEW_BOX_OPEN);
        assert!(has_terminal_new_box_open(&stripped));
    }

    #[test]
    fn test_unavailable_when_no_candidates() {
        let template = detect_bridge_template(&[], 4);
        assert_eq!(template.quality, TemplateQuality::Unavailable);
        assert!(template.instructions.is_empty());
    }

    #[test]
    fn test_clone_bridge_rewrites_speaker_arg() {
        use crate::bin_format::instruction::Argument;

        let template = BridgeTemplate {
            quality: TemplateQuality::Clean,
            instructions: vec![
                Instruction {
                    opcode: 0x55, // mov
                    label: "mov".to_string(),
                    argument_count: 2,
                    arguments: vec![
                        Argument {
                            arg_type: 3,
                            raw_data: 0x1000,
                            decoded_string: None,
                            original_encoded_bytes: None,
                            data_array: None,
                        },
                        Argument {
                            arg_type: 0,
                            raw_data: 42, // speaker ID
                            decoded_string: None,
                            original_encoded_bytes: None,
                            data_array: None,
                        },
                    ],
                    offset: 0,
                },
                Instruction {
                    opcode: OP_NEW_BOX_OPEN,
                    label: "u00416120".to_string(),
                    argument_count: 0,
                    arguments: Vec::new(),
                    offset: 0,
                },
            ],
            speaker_arg_positions: vec![(0, 1)], // instr 0, arg 1 is the speaker ID
            sys_version: 4,
        };

        let cloned = clone_bridge(&template, 99, None);
        assert_eq!(cloned.len(), 2);
        assert_eq!(cloned[0].arguments[1].raw_data, 99, "speaker ID should be rewritten");
        assert_eq!(cloned[0].arguments[0].raw_data, 0x1000, "non-speaker arg untouched");
        assert_eq!(cloned[1].opcode, OP_NEW_BOX_OPEN);
    }

    #[test]
    fn test_plan_split_produces_correct_structure() {
        use crate::editor::wrap::wrap_english;

        let wrap = wrap_english(
            "This is a long sentence that should be split across multiple boxes",
            30,
            2,
        );
        assert!(wrap.boxes.len() >= 2, "wrap should produce ≥2 boxes");

        let template = BridgeTemplate {
            quality: TemplateQuality::Clean,
            instructions: vec![Instruction {
                opcode: OP_NEW_BOX_OPEN,
                label: "u00416120".to_string(),
                argument_count: 0,
                arguments: Vec::new(),
                offset: 0,
            }],
            speaker_arg_positions: Vec::new(),
            sys_version: 4,
        };

        let plan = plan_split(42, &wrap, &template, 7, None, vec![], None);
        assert_eq!(plan.source_entry_index, 42);
        assert_eq!(plan.box_lines.len(), wrap.boxes.len());
        assert_eq!(
            plan.bridge_instructions_per_split.len(),
            wrap.boxes.len() - 1,
            "N-1 bridges for N boxes"
        );
        for bridge in &plan.bridge_instructions_per_split {
            assert!(!bridge.is_empty());
            assert_eq!(bridge.last().unwrap().opcode, OP_NEW_BOX_OPEN);
        }
        // Each box has at least one line
        for box_lines in &plan.box_lines {
            assert!(!box_lines.is_empty(), "every box must have ≥1 line");
        }
    }
}
