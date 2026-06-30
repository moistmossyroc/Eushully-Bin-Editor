//! Bilingual (dual-language) BIN generation.
//!
//! Wraps each dialogue block so the engine renders English or Japanese based on a
//! patchable in-BIN flag, with NO scene reload. Each block's text region becomes:
//! ```text
//!   eq  reg9_0, FLAG, SENTINEL     ; reg = (FLAG == SENTINEL)
//!   jcc reg9_0, -1, JP_region      ; if !reg jump to the Japanese region
//!   { English text region }        ; reflowed EN show-text / end-text-line / furigana
//!   jmp after_region               ; skip the Japanese region
//!   { Japanese text region }       ; original (else branch) — jcc lands here
//! ```
//! All new control-flow targets point at ORIGINAL instructions (their `.offset`),
//! which the writer remaps via `old_to_new_offset`; new instructions keep the
//! `u64::MAX` sentinel offset. The companion loader flips the FLAG word at runtime
//! (`FLAG == SENTINEL` → English, `0` → Japanese) by scanning for the eq pattern.
//!
//! Proven in-game on Madou Koukaku (SC0000): jcc is jump-if-false, so FLAG defaults
//! to SENTINEL (English on).

use crate::bin_format::instruction::{Argument, Instruction};
use std::collections::HashMap;

/// Distinctive constant ("LANG") embedded in every generated `eq` so the loader can
/// locate the flags unambiguously. The loader writes this value (English) or 0
/// (Japanese) into each eq's FLAG argument.
pub const LANG_SENTINEL: u32 = 0x4C41_4E47;

// Opcodes used by the conditional + region detection.
const OP_EQ: u32 = 0x5A;
const OP_JCC: u32 = 0xA0;
const OP_JMP: u32 = 0x8C;
const OP_WAIT: u32 = 0x72;
const OP_SHOW_TEXT: u32 = 0x6E;

fn lit(value: u32) -> Argument {
    Argument { arg_type: 0, raw_data: value, decoded_string: None, original_encoded_bytes: None, data_array: None }
}
fn reg(index: u32) -> Argument {
    Argument { arg_type: 9, raw_data: index, decoded_string: None, original_encoded_bytes: None, data_array: None }
}

/// A new (inserted) instruction with the `u64::MAX` offset sentinel.
fn new_instr(opcode: u32, label: &str, arguments: Vec<Argument>) -> Instruction {
    Instruction { opcode, label: label.to_string(), argument_count: arguments.len() as u32, arguments, offset: u64::MAX }
}

/// Clone an existing instruction as a NEW one (re-encode string args from their
/// decoded form; keep raw values for everything else).
fn clone_as_new(src: &Instruction) -> Instruction {
    let arguments = src
        .arguments
        .iter()
        .map(|a| Argument {
            arg_type: a.arg_type,
            raw_data: a.raw_data,
            decoded_string: a.decoded_string.clone(),
            original_encoded_bytes: if a.arg_type == 2 { None } else { a.original_encoded_bytes.clone() },
            data_array: a.data_array.clone(),
        })
        .collect();
    Instruction { opcode: src.opcode, label: src.label.clone(), argument_count: src.argument_count, arguments, offset: u64::MAX }
}

/// Split into blocks delimited by `wait-for-input` (0x72). Returns
/// `(block_start_idx, wait_idx)` per block (one dialogue beat).
fn split_blocks(instructions: &[Instruction]) -> Vec<(usize, usize)> {
    let mut blocks = Vec::new();
    let mut start = 0usize;
    for (i, ins) in instructions.iter().enumerate() {
        if ins.opcode == OP_WAIT {
            blocks.push((start, i));
            start = i + 1;
        }
    }
    blocks
}

/// The text-rendering region within a block: from the first `show-text` to the last
/// text-opcode (show-text / end-text-line / furigana / new-textbox) before the wait.
/// Returns `None` for blocks with no dialogue (pure setup/choice blocks).
fn text_region(instructions: &[Instruction], start: usize, wait: usize) -> Option<(usize, usize)> {
    let is_text = |op: u32| matches!(op, 0x6E | 0x6F | 0x196 | 0x1F5);
    let region_start = (start..wait).find(|&i| instructions[i].opcode == OP_SHOW_TEXT)?;
    let region_end = (start..wait).rev().find(|&i| is_text(instructions[i].opcode))?;
    Some((region_start, region_end))
}

/// Outcome of a bilingual build (for reporting/validation).
#[derive(Debug, Clone)]
pub struct BilingualReport {
    /// Number of dialogue blocks wrapped with a language conditional.
    pub wrapped_blocks: usize,
    /// Japanese block count (wait-for-input beats).
    pub jp_blocks: usize,
    /// English block count — should equal `jp_blocks` (translation reflows within
    /// blocks but must not add/remove `wait-for-input`).
    pub en_blocks: usize,
}

/// Build a bilingual instruction list from the original Japanese instructions and
/// the applied English instructions. Blocks are matched 1:1 by `wait-for-input`
/// index. Returns the new instruction list (feed to `write_bin`) plus a report.
///
/// `jp` = parsed original BIN instructions; `en` = post-`apply` (translated)
/// instructions for the same BIN.
pub fn build_bilingual(jp: &[Instruction], en: &[Instruction]) -> (Vec<Instruction>, BilingualReport) {
    let jp_blocks = split_blocks(jp);
    let en_blocks = split_blocks(en);
    let n = jp_blocks.len().min(en_blocks.len());

    // Map each JP text-region start index -> (jp region-start offset, jmp-after
    // offset, en block index). Targets are ORIGINAL offsets (writer remaps them).
    let mut wrap: HashMap<usize, (u32, u32, usize)> = HashMap::new();
    for k in 0..n {
        let (js, jw) = jp_blocks[k];
        if let Some((rs, re)) = text_region(jp, js, jw) {
            // re + 1 is always <= jw (the wait), so it exists.
            let jmp_after = jp[re + 1].offset as u32;
            wrap.insert(rs, (jp[rs].offset as u32, jmp_after, k));
        }
    }

    let mut out: Vec<Instruction> = Vec::with_capacity(jp.len() + wrap.len() * 8);
    let mut wrapped = 0usize;
    for idx in 0..jp.len() {
        if let Some(&(jp_region_off, jmp_after_off, en_k)) = wrap.get(&idx) {
            let (es, ew) = en_blocks[en_k];
            if let Some((ers, ere)) = text_region(en, es, ew) {
                // eq reg9_0, FLAG(=SENTINEL default), SENTINEL
                out.push(new_instr(OP_EQ, "eq", vec![reg(0), lit(LANG_SENTINEL), lit(LANG_SENTINEL)]));
                // jcc reg9_0, -1, <jp region start>  (jump-if-false → Japanese)
                out.push(new_instr(OP_JCC, "jcc", vec![reg(0), lit(0xFFFF_FFFF), lit(jp_region_off)]));
                // English region (reflowed show-text/end-text-line/furigana)
                for i in ers..=ere {
                    out.push(clone_as_new(&en[i]));
                }
                // jmp <after the JP region>  (skip the else branch)
                out.push(new_instr(OP_JMP, "jmp", vec![lit(jmp_after_off)]));
                wrapped += 1;
            }
        }
        // Original instruction (the JP text region becomes the else branch).
        out.push(jp[idx].clone());
    }

    let report = BilingualReport { wrapped_blocks: wrapped, jp_blocks: jp_blocks.len(), en_blocks: en_blocks.len() };
    (out, report)
}
