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
//!
//! Opcode semantics (OpenMaidEngine `vm-map/opcodes.toml`, MIT, Ghidra-backed;
//! cross-checked on 709 original scenes across 6 SYS4 games):
//! - `0x71` reset-adv-text-layout(slot) — the actual box CLEAR (present in
//!   100% of native wait→show transitions). Slot 1 = body text. Where the
//!   NAME lives differs per game (age-sys.md §7e): text in layout 2 (Madou,
//!   LaDEA, Sankai, Tenmei, Hyakusen), text inside the body layout (Atelial)
//!   or a sprite (VERITA, Megami Zero, Himegari).
//! - `0x21B` set-tick-presentation(enabled) — render gate; `0` hides drawing
//!   until re-enabled (text then only appears via the backlog redraw).
//! - `0x1F4`/`0x1F5` stmt-begin/stmt-end — atomic statement block. `0x1F5` is
//!   NOT a "new box" opcode; it terminates every native transition only
//!   because the transition is wrapped in a block, which is why it still works
//!   as the detector's anchor.

use crate::bin_format::instruction::{Argument, Instruction};
use crate::bin_format::opcodes::definition_for_opcode;
use serde::{Deserialize, Serialize};

/// Opcode constants used by the bridge detector.
const OP_WAIT_FOR_INPUT: u32 = 0x72;
const OP_SHOW_TEXT: u32 = 0x6E;
const OP_END_TEXT_LINE: u32 = 0x6F;
/// `0x11` copy a layer rectangle onto another (SYS3 window blit: `src, dst, x, y, w, h, dx, dy, 1`).
const OP_BLIT_LAYER: u32 = 0x11;
/// `0x196` display-furigana: a body line drawn with ruby.
const OP_DISPLAY_FURIGANA: u32 = 0x196;
const OP_JCC: u32 = 0xA0;
const OP_JMP: u32 = 0x8C;
/// `0x71` reset-adv-text-layout(slot): clears a text layout (the box clear).
const OP_RESET_TEXT_LAYOUT: u32 = 0x71;
/// `0x21B` set-tick-presentation(enabled): frame presentation gate.
const OP_SET_TICK_PRESENTATION: u32 = 0x21B;
/// `0x1F4` stmt-begin: opens an atomic statement block.
const OP_STMT_BEGIN: u32 = 0x1F4;
/// `u00416120` = stmt-end (closes the statement block every native box
/// transition is wrapped in). Historically misnamed "new textbox open"; kept as
/// the bridge terminator because every native transition ends with it.
const OP_NEW_BOX_OPEN: u32 = 0x1F5;
/// Body-text layout slot used when a scene gives no evidence of its own.
const DEFAULT_BODY_LAYOUT_SLOT: u32 = 1;

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

/// Return true if the window contains at least one `OP_NEW_BOX_OPEN` and the
/// window's last instruction is one.
///
/// Earlier this required *exactly* one `0x1F5`, which rejected builds that emit
/// two per transition (e.g. Megami Zero v415: a close-box `0x1F5` followed by an
/// open-box `0x1F5` around a `call`). We only need the bridge to *terminate* the
/// box, so the count constraint is relaxed to "≥1, ending in one". This is
/// additive — windows that already qualified still do, so clean-bridge games are
/// unaffected (Pass 1 still selects their branch-free template first).
fn has_terminal_new_box_open(window: &[Instruction]) -> bool {
    if !window.iter().any(|i| i.opcode == OP_NEW_BOX_OPEN) {
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

/// Build a 0/1-literal-argument instruction (all semantic-bridge opcodes take
/// only immediates, so no address remapping or string encoding is involved).
fn immediate_instruction(opcode: u32, values: &[u32]) -> Instruction {
    let label = definition_for_opcode(opcode)
        .map(|d| d.label.to_string())
        .unwrap_or_else(|| format!("{opcode:X}"));
    let arguments: Vec<Argument> = values
        .iter()
        .map(|&raw_data| Argument {
            arg_type: 0,
            raw_data,
            decoded_string: None,
            original_encoded_bytes: None,
            data_array: None,
        })
        .collect();
    Instruction {
        opcode,
        label,
        argument_count: arguments.len() as u32,
        arguments,
        offset: u64::MAX,
    }
}

/// Immediate first argument of an instruction, if it has one.
fn immediate_arg0(instruction: &Instruction) -> Option<u32> {
    instruction
        .arguments
        .first()
        .filter(|a| a.arg_type == 0)
        .map(|a| a.raw_data)
}

/// Longest native box transition scanned for clear evidence (averages per game:
/// VERITA 20 … Sankai 105 instructions — longer than a cloneable template).
const MAX_NATIVE_TRANSITION: usize = 600;

/// The ADV box's clear + presentation-gate ops of a transition window, in order. A
/// clear directly followed by `jmp` is the other arm of a display-mode branch (Sankai
/// `t3:0 == 4` → `0x71 8`, Hyakusen `t3:0 == 2` → `0x71 3`), not the ADV box.
fn box_clear_ops(window: &[Instruction]) -> impl Iterator<Item = &Instruction> {
    window.iter().enumerate().filter_map(move |(k, i)| {
        let alternative_arm = window.get(k + 1).is_some_and(|next| next.opcode == OP_JMP);
        let keep = i.opcode == OP_SET_TICK_PRESENTATION || (i.opcode == OP_RESET_TEXT_LAYOUT && !alternative_arm);
        keep.then_some(i)
    })
}

/// The scene's dominant native box transition: among every `wait-for-input` →
/// next body `show-text` (layout 0) window that clears a layout, the window whose
/// clear + presentation-gate pattern is the most common (one pattern per game:
/// `1`, `2 1`, `3 2 1`, `8 2 1` — corpus `box_transition_audit`).
fn dominant_native_transition(scene: &[Instruction]) -> Option<&[Instruction]> {
    let is_body_text = |i: &Instruction| i.opcode == OP_SHOW_TEXT && immediate_arg0(i) == Some(0);
    let mut patterns: Vec<(Vec<(u32, Option<u32>)>, usize, &[Instruction])> = Vec::new();
    for (wait_idx, wait) in scene.iter().enumerate() {
        if wait.opcode != OP_WAIT_FOR_INPUT {
            continue;
        }
        let rest = &scene[wait_idx + 1..scene.len().min(wait_idx + 1 + MAX_NATIVE_TRANSITION)];
        let Some(len) = rest.iter().position(|i| is_body_text(i) || i.opcode == OP_WAIT_FOR_INPUT) else { continue };
        if rest[len].opcode != OP_SHOW_TEXT {
            continue;
        }
        let window = &rest[..len];
        let key: Vec<(u32, Option<u32>)> =
            box_clear_ops(window).map(|i| (i.opcode, immediate_arg0(i))).collect();
        if !key.iter().any(|(op, _)| *op == OP_RESET_TEXT_LAYOUT) {
            continue;
        }
        match patterns.iter_mut().find(|(k, _, _)| *k == key) {
            Some(found) => found.1 += 1,
            None => patterns.push((key, 1, window)),
        }
    }
    patterns.into_iter().max_by_key(|(_, count, _)| *count).map(|(_, _, window)| window)
}

/// Evidence for the semantic bridge: the scene's dominant native box transition,
/// else the detected template.
fn semantic_evidence_window<'a>(scene: &'a [Instruction], template: Option<&'a BridgeTemplate>) -> &'a [Instruction] {
    if let Some(window) = dominant_native_transition(scene) {
        return window;
    }
    template
        .filter(|t| t.quality != TemplateQuality::Unavailable)
        .map(|t| t.instructions.as_slice())
        .unwrap_or(&[])
}

/// Instructions at the head of a procedure searched for its speaker metadata op.
const SPEAKER_PROCEDURE_HEAD: usize = 4;
/// Longest procedure body considered (instructions up to its `ret`).
const MAX_PROCEDURE_LEN: usize = 300;
const OP_CALL: u32 = 0x8F;
const OP_RET: u32 = 0x05;
/// `0x1D2` append-text-history-metadata(type, value): type 2 = the box's speaker.
const OP_APPEND_HISTORY_METADATA: u32 = 0x1D2;
const OP_PLAY_VOICE: u32 = 0xC4;
/// `0x204` draw-string into a texture (name plates drawn as sprites: VERITA, Megami Zero).
const OP_DRAW_STRING: u32 = 0x204;

/// Call depth followed when asking what a procedure reaches (dispatcher → box-begin →
/// name draw is the deepest native chain: Atelial).
const MAX_CALL_DEPTH: usize = 3;

/// A scene's procedures, addressed by code offset (`call` targets).
struct SceneProcedures<'a> {
    scene: &'a [Instruction],
    index_by_offset: std::collections::HashMap<u64, usize>,
}

impl<'a> SceneProcedures<'a> {
    /// Indexes only original instructions (inserted ones carry the `u64::MAX` sentinel).
    fn new(scene: &'a [Instruction]) -> Self {
        let index_by_offset = scene
            .iter()
            .enumerate()
            .filter(|(_, i)| i.offset != u64::MAX)
            .map(|(k, i)| (i.offset, k))
            .collect();
        Self { scene, index_by_offset }
    }

    /// Body of the procedure at `target`: up to (excluding) its first `ret`.
    fn body(&self, target: u32) -> Option<&'a [Instruction]> {
        let start = *self.index_by_offset.get(&(target as u64))?;
        let rest = &self.scene[start..self.scene.len().min(start + MAX_PROCEDURE_LEN)];
        let end = rest.iter().position(|i| i.opcode == OP_RET).unwrap_or(rest.len());
        Some(&rest[..end])
    }

    /// Whether `code` or a procedure it calls (up to `depth` levels) has an instruction
    /// matching `predicate`.
    fn reaches(&self, code: &[Instruction], predicate: &dyn Fn(&Instruction) -> bool, depth: usize) -> bool {
        code.iter().any(|i| {
            predicate(i)
                || (depth > 0
                    && i.opcode == OP_CALL
                    && immediate_arg0(i).and_then(|t| self.body(t)).is_some_and(|b| self.reaches(b, predicate, depth - 1)))
        })
    }

    /// Whether calling `target` reaches the procedure `procedure` (or is it).
    fn procedure_reaches(&self, target: u32, procedure: u32) -> bool {
        target == procedure
            || self.body(target).is_some_and(|b| {
                self.reaches(b, &|i| i.opcode == OP_CALL && immediate_arg0(i) == Some(procedure), MAX_CALL_DEPTH - 1)
            })
    }

    /// Voice- and input-free: safe to call again from a continuation box.
    fn is_silent(&self, target: u32) -> bool {
        self.body(target).is_some_and(|b| {
            !self.reaches(b, &|i| i.opcode == OP_PLAY_VOICE || i.opcode == OP_WAIT_FOR_INPUT, MAX_CALL_DEPTH - 1)
        })
    }
}

/// The scene-local **speaker box-begin procedure** (docs/engine/age-sys.md §7e), if
/// the scene has one: the procedure every speaking box calls after its clear. It
/// records the speaker for the backlog (`0x1d2` at its head), draws the name (text
/// into the plate or body layout, or a name sprite) and sets the speaker's text
/// colours — all from the speaker globals that the box transition set and that stay
/// set until the next box. Calling it again therefore redraws the CURRENT speaker.
/// Found in VERITA, Atelial, Madou, LaDEA, Sankai, Tenmei, Hyakusen; Megami Zero /
/// Himegari draw the name inline (sprite, survives a body clear), SYS3 has none.
/// Narration boxes call a different (narrator) procedure, so a continuation only
/// redraws through what its source box called (`resolve_speaker_redraws`).
///
/// Returns the procedure's code offset (a `call` target). Picks the most-called
/// candidate; rejects procedures that (through their callees) play a voice or wait
/// for input.
pub fn find_speaker_redraw_procedure(scene: &[Instruction]) -> Option<u32> {
    let procedures = SceneProcedures::new(scene);
    let mut call_counts: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for target in scene.iter().filter(|i| i.opcode == OP_CALL).filter_map(immediate_arg0) {
        *call_counts.entry(target).or_default() += 1;
    }
    let mut candidates: Vec<(u32, usize)> = call_counts
        .into_iter()
        .filter(|&(target, _)| {
            let Some(body) = procedures.body(target) else { return false };
            if !body.iter().take(SPEAKER_PROCEDURE_HEAD).any(|i| i.opcode == OP_APPEND_HISTORY_METADATA) {
                return false;
            }
            // Its own code plus one level of callees (the name draw is often a sub-call).
            procedures.reaches(body, &|i| i.opcode == OP_SHOW_TEXT || i.opcode == OP_DRAW_STRING, 1)
                && procedures.is_silent(target)
        })
        .collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    candidates.first().map(|(target, _)| *target)
}

/// Export fix-up for keep-speaker box breaks (applied to the instruction list right
/// before writing). Each INSERTED `call <speaker procedure>` is matched to the box
/// it continues — the nearest original body `show-text` before it — and that box's
/// own native transition decides:
/// - it called the procedure directly → kept (Sankai, Madou, VERITA …);
/// - it reached it through a dispatcher (Atelial: talk key → `speaker >= 0 ?
///   box-begin : narrator`) → retargeted to that dispatcher;
/// - it never reached it (narration: the speaker globals say "no speaker" and the
///   box-begin would index its name table at -1) → removed.
pub fn resolve_speaker_redraws(mut instructions: Vec<Instruction>) -> Vec<Instruction> {
    let is_inserted_call = |i: &Instruction| i.offset == u64::MAX && i.opcode == OP_CALL;
    if !instructions.iter().any(is_inserted_call) {
        return instructions;
    }
    let Some(procedure) = find_speaker_redraw_procedure(&instructions) else { return instructions };
    let decisions: Vec<(usize, Option<u32>)> = {
        let procedures = SceneProcedures::new(&instructions);
        let is_original = |i: &Instruction| i.offset != u64::MAX;
        let is_body_text = |i: &Instruction| i.opcode == OP_SHOW_TEXT && immediate_arg0(i) == Some(0);
        instructions
            .iter()
            .enumerate()
            .filter(|(_, i)| is_inserted_call(i) && immediate_arg0(i) == Some(procedure))
            .map(|(k, _)| {
                let source = instructions[..k].iter().rposition(|i| is_original(i) && is_body_text(i));
                let entry = source.and_then(|s| {
                    let window_start = instructions[..s]
                        .iter()
                        .rposition(|i| is_original(i) && i.opcode == OP_WAIT_FOR_INPUT)
                        .map_or(0, |w| w + 1);
                    // Nearest qualifying call before the text: in a scene's first box the
                    // window spans the whole preamble, whose init routines also reach the
                    // procedure but play voices (Sankai SC0700, in-game 2026-11-12).
                    instructions[window_start..s]
                        .iter()
                        .rev()
                        .filter(|i| i.opcode == OP_CALL)
                        .filter_map(immediate_arg0)
                        .find(|&t| (t == procedure || procedures.is_silent(t)) && procedures.procedure_reaches(t, procedure))
                });
                (k, entry)
            })
            .collect()
    };
    let mut removed = std::collections::HashSet::new();
    for (k, entry) in decisions {
        match entry {
            Some(target) => instructions[k].arguments[0].raw_data = target,
            None => {
                removed.insert(k);
            }
        }
    }
    let mut position = 0;
    instructions.retain(|_| {
        position += 1;
        !removed.contains(&(position - 1))
    });
    instructions
}

/// Export fix-up for SYS3 inline speaker names (Meishoku / Minefuka, docs/engine/age-sys.md
/// §7e). There a speech box opens with a short name line (`show-text 0,"フィーノ"` +
/// end-text-line) INSIDE the body layout, and the native script repeats it on every box of
/// a speech (Meishoku 1,628 named boxes vs 17 nameless speech boxes after a named one).
/// A box break clears the body, so each INSERTED continuation box (inserted click-wait →
/// first body line) whose continued box opened with a name line gets a copy of that line
/// (+ its end-text-line) on top — the translated name, as `modified` is the post-edit list.
/// Name lines are recognised on the ORIGINAL Japanese script (`original`, matched by
/// offset): after translation a short English narration line would look like a name.
/// Kuutei draws the name outside the body (it persists) and has no name lines, so nothing
/// changes there. Call for SYS3 only.
pub fn carry_inline_speaker_names(original: &[Instruction], modified: Vec<Instruction>) -> Vec<Instruction> {
    let is_inserted = |i: &Instruction| i.offset == u64::MAX;
    if !modified.iter().any(|i| is_inserted(i) && i.opcode == OP_WAIT_FOR_INPUT) {
        return modified;
    }
    let name_lines = sys3_name_line_offsets(original);
    if name_lines.is_empty() {
        return modified;
    }
    let mut out: Vec<Instruction> = Vec::with_capacity(modified.len() + 16);
    let mut copied: std::collections::HashSet<usize> = Default::default(); // out indices of name copies
    for instruction in modified {
        if is_inserted(&instruction) && is_body_text(&instruction) {
            let is_name_line = |k: usize| copied.contains(&k) || name_lines.contains(&out[k].offset);
            if let Some(first) = continued_box_name_line(&out, &is_name_line) {
                let (name, close) = (out[first].clone(), out[first + 1].clone());
                copied.insert(out.len());
                for mut copy in [name, close] {
                    copy.offset = u64::MAX;
                    out.push(copy);
                }
            }
        }
        out.push(instruction);
    }
    out
}

/// Export fix-up for SYS3 line closing. SYS3 closes every line with an end-text-line
/// BEFORE the click-wait (`show-text, 0x6F, wait, 0x6F` — 100% of waits in Kuutei,
/// Meishoku and Minefuka), SYS4/5 never do (`show-text, wait, 0x6F`). An inserted
/// click-wait right after text leaves the SYS3 line open: in-game (Meishoku, 2026-11-12)
/// the open line is redrawn into the next box after the clear, and a long one makes the
/// engine stop with "データが壊れています". Each INSERTED wait that follows text directly
/// gets an inserted end-text-line in front of it. Call for SYS3 only.
pub fn close_lines_before_inserted_waits(modified: Vec<Instruction>) -> Vec<Instruction> {
    let opens_line = |i: &Instruction| i.opcode == OP_SHOW_TEXT || i.opcode == OP_DISPLAY_FURIGANA;
    let mut out: Vec<Instruction> = Vec::with_capacity(modified.len() + 8);
    for instruction in modified {
        let inserted_wait = instruction.opcode == OP_WAIT_FOR_INPUT && instruction.offset == u64::MAX;
        if inserted_wait && out.last().is_some_and(opens_line) {
            let mut close = immediate_instruction(OP_END_TEXT_LINE, &[0]);
            close.offset = u64::MAX;
            out.push(close);
        }
        out.push(instruction);
    }
    out
}

/// Export fix-up for the SYS3 box clear. On SYS3 `0x71 1` only resets the text layout:
/// the drawn glyphs stay on the window until the script re-blits the window background,
/// which every native clear does right after it (Meishoku 380/380 `call <0x11 3,… box
/// blit>`, Kuutei 929 the same, Minefuka 153/153 `call <compose>` + `0x11 3,0,…` full
/// window blit). Without it a continuation box draws over the previous box's text
/// (in-game 2026-11-12). Each INSERTED body clear gets a copy of the redraw run (silent
/// calls + `0x11` blits) after the nearest preceding ORIGINAL body clear (same window as
/// the continued box; the next one when the break is in the scene's first box). Call for
/// SYS3 only.
pub fn redraw_box_after_inserted_clears(modified: Vec<Instruction>) -> Vec<Instruction> {
    let is_body_clear = |i: &Instruction| i.opcode == OP_RESET_TEXT_LAYOUT && immediate_arg0(i) == Some(DEFAULT_BODY_LAYOUT_SLOT);
    if !modified.iter().any(|i| i.offset == u64::MAX && is_body_clear(i)) {
        return modified;
    }
    let procedures = SceneProcedures::new(&modified);
    // The redraw run after a native clear: silent calls and window blits (Meishoku/Kuutei:
    // `call <0x11 box blit>`; Minefuka: `call <compose>` + `0x11` full-window blit).
    let is_redraw = |i: &Instruction| {
        i.opcode == OP_BLIT_LAYER || (i.opcode == OP_CALL && immediate_arg0(i).is_some_and(|t| procedures.is_silent(t)))
    };
    let native_redraws: Vec<(usize, &[Instruction])> = modified
        .iter()
        .enumerate()
        .filter(|(_, i)| i.offset != u64::MAX && is_body_clear(i))
        .filter_map(|(k, _)| {
            let run = modified[k + 1..].iter().take_while(|i| i.offset != u64::MAX && is_redraw(i)).count();
            (run > 0).then(|| (k, &modified[k + 1..k + 1 + run]))
        })
        .collect();
    let mut redraw_for: std::collections::HashMap<usize, &[Instruction]> = Default::default(); // inserted clear → run
    for (k, instruction) in modified.iter().enumerate() {
        if instruction.offset != u64::MAX || !is_body_clear(instruction) {
            continue;
        }
        // The nearest earlier native clear (same window as the continued box); a break in
        // the scene's first box borrows the next one (Minefuka's opening narration).
        let run = native_redraws
            .iter()
            .rev()
            .find(|(native, _)| *native < k)
            .or_else(|| native_redraws.iter().find(|(native, _)| *native > k))
            .map(|(_, run)| *run);
        let Some(run) = run else { continue };
        let already = modified.get(k + 1).is_some_and(|next| next.opcode == run[0].opcode && immediate_arg0(next) == immediate_arg0(&run[0]));
        if !already {
            redraw_for.insert(k, run);
        }
    }
    let mut out = Vec::with_capacity(modified.len() + redraw_for.values().map(|r| r.len()).sum::<usize>());
    for (k, instruction) in modified.iter().enumerate() {
        out.push(instruction.clone());
        if let Some(run) = redraw_for.remove(&k) {
            out.extend(run.iter().map(|i| Instruction { offset: u64::MAX, ..i.clone() }));
        }
    }
    out
}

fn is_body_text(instruction: &Instruction) -> bool {
    instruction.opcode == OP_SHOW_TEXT && immediate_arg0(instruction) == Some(0)
}

/// Offsets of the original name lines: the first body line of a box when it is a SYS3
/// speaker name (`characters::is_sys3_speaker_name`) and more body text follows in the box.
fn sys3_name_line_offsets(original: &[Instruction]) -> std::collections::HashSet<u64> {
    let mut offsets = std::collections::HashSet::new();
    let mut box_first: Option<usize> = None; // first body line of the current box
    for (k, instruction) in original.iter().enumerate() {
        if instruction.opcode == OP_WAIT_FOR_INPUT {
            box_first = None;
        } else if is_body_text(instruction) {
            match box_first {
                None => box_first = Some(k),
                Some(first) => {
                    let text = original[first].arguments.iter().find_map(|a| a.decoded_string.as_deref()).unwrap_or_default();
                    if crate::editor::characters::is_sys3_speaker_name(original, first, text) {
                        offsets.insert(original[first].offset);
                    }
                }
            }
        }
    }
    offsets
}

/// Index (in `preceding`) of the name line of the box continued by a box about to start at
/// the end of `preceding` — when that box was opened by an inserted click-wait and the
/// continued box's first body line is a name line (`is_name_line`) followed by an
/// end-text-line.
fn continued_box_name_line(preceding: &[Instruction], is_name_line: &dyn Fn(usize) -> bool) -> Option<usize> {
    let wait = preceding.iter().rposition(|i| i.opcode == OP_WAIT_FOR_INPUT || is_body_text(i))?;
    if preceding[wait].opcode != OP_WAIT_FOR_INPUT || preceding[wait].offset != u64::MAX {
        return None; // not the first line of an inserted continuation box
    }
    let box_start = preceding[..wait].iter().rposition(|i| i.opcode == OP_WAIT_FOR_INPUT).map_or(0, |w| w + 1);
    let first = (box_start..wait).find(|&k| is_body_text(&preceding[k]))?;
    (is_name_line(first) && preceding.get(first + 1).is_some_and(|i| i.opcode == OP_END_TEXT_LINE)).then_some(first)
}

/// Synthesize a box break from opcode SEMANTICS instead of cloning the scene's
/// transition verbatim. Shape (mirrors the native order, minus scene logic):
///
/// ```text
/// end-text-line 0                    (close; callers may reuse an existing one)
/// [set-tick-presentation 0]          (only if the scene gates presentation)
/// reset-adv-text-layout <slot>…      (the box clear, native slot order)
/// [stmt-begin]                       (only if the build has statement blocks)
/// [call <speaker box-begin>]         (keep_speaker + the scene has one)
/// [set-tick-presentation 1]
/// [stmt-end]                         (terminator on SYS4/5; SYS3 has neither, so
///                                     the bridge ends on the box clear there)
/// ```
///
/// Without `keep_speaker` it contains NO `call`/`jmp`/`jcc`/`mov` — nothing
/// scene-specific — so it is available in every scene and can't mis-resolve.
/// With `keep_speaker` the continuation box keeps the speaker of the box it
/// continues: when the scene has a speaker box-begin procedure
/// (`find_speaker_redraw_procedure`) the native layouts are cleared and the
/// procedure is called again (name, colours and backlog speaker redrawn from the
/// still-set speaker globals; no voice, no portrait change). Otherwise only the
/// body layout is cleared, so a name drawn in another layout or as a sprite stays.
pub fn build_semantic_bridge(
    scene: &[Instruction],
    template: Option<&BridgeTemplate>,
    keep_speaker: bool,
) -> Vec<Instruction> {
    let evidence = semantic_evidence_window(scene, template);

    let mut slots: Vec<u32> = Vec::new();
    for slot in box_clear_ops(evidence)
        .filter(|i| i.opcode == OP_RESET_TEXT_LAYOUT)
        .filter_map(immediate_arg0)
    {
        if !slots.contains(&slot) {
            slots.push(slot);
        }
    }
    let speaker_redraw = if keep_speaker { find_speaker_redraw_procedure(scene) } else { None };
    if keep_speaker && speaker_redraw.is_none() && slots.contains(&DEFAULT_BODY_LAYOUT_SLOT) {
        slots.retain(|&s| s == DEFAULT_BODY_LAYOUT_SLOT);
    }
    if slots.is_empty() {
        slots.push(DEFAULT_BODY_LAYOUT_SLOT);
    }

    // Mirror each presentation-gate op only where the native transition uses
    // it: VERITA never touches 0x21B in transitions (different draw mode), and
    // Megami Zero only re-enables it (its "off" happens elsewhere).
    let native_gate_op = |enabled: u32| {
        evidence
            .iter()
            .any(|i| i.opcode == OP_SET_TICK_PRESENTATION && immediate_arg0(i) == Some(enabled))
    };
    let (hides_presentation, shows_presentation) = (native_gate_op(0), native_gate_op(1));

    let statements = scene_uses_statement_blocks(scene);

    let mut bridge = vec![immediate_instruction(OP_END_TEXT_LINE, &[0])];
    if hides_presentation {
        bridge.push(immediate_instruction(OP_SET_TICK_PRESENTATION, &[0]));
    }
    bridge.extend(slots.iter().map(|&slot| immediate_instruction(OP_RESET_TEXT_LAYOUT, &[slot])));
    if statements {
        bridge.push(immediate_instruction(OP_STMT_BEGIN, &[]));
    }
    if let Some(procedure) = speaker_redraw {
        // A code-address immediate: the writer remaps it like any call target.
        bridge.push(immediate_instruction(OP_CALL, &[procedure]));
    }
    if shows_presentation {
        bridge.push(immediate_instruction(OP_SET_TICK_PRESENTATION, &[1]));
    }
    if statements {
        bridge.push(immediate_instruction(OP_NEW_BOX_OPEN, &[]));
    }
    bridge
}

/// Whether the engine build understands stmt-begin/stmt-end (`0x1F4`/`0x1F5`): the
/// scene uses them itself. SYS3 builds (Meishoku / Minefuka 324, Kuutei) have no such
/// opcodes and abort with "このコマンドはサポートされていません COMMAND=-(500)"
/// (in-game 2026-11-12). An empty `scene` (no evidence) keeps the SYS4/5 shape.
fn scene_uses_statement_blocks(scene: &[Instruction]) -> bool {
    scene.is_empty() || scene.iter().any(|i| i.opcode == OP_STMT_BEGIN || i.opcode == OP_NEW_BOX_OPEN)
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

    /// LaDEA-shaped native transition (SC3600's detected template, calls kept).
    fn ladea_template() -> BridgeTemplate {
        let call = |target: u32| {
            let mut i = immediate_instruction(0x8F, &[target]);
            i.offset = 0;
            i
        };
        BridgeTemplate {
            quality: TemplateQuality::Clean,
            instructions: vec![
                immediate_instruction(OP_END_TEXT_LINE, &[0]),
                immediate_instruction(OP_SET_TICK_PRESENTATION, &[0]),
                immediate_instruction(OP_RESET_TEXT_LAYOUT, &[2]),
                immediate_instruction(OP_RESET_TEXT_LAYOUT, &[1]),
                call(0x1d845),
                immediate_instruction(OP_STMT_BEGIN, &[]),
                call(0x1d89a),
                immediate_instruction(OP_SET_TICK_PRESENTATION, &[1]),
                immediate_instruction(OP_NEW_BOX_OPEN, &[]),
            ],
            speaker_arg_positions: Vec::new(),
            sys_version: 4,
        }
    }

    fn shape(bridge: &[Instruction]) -> Vec<(u32, Vec<u32>)> {
        bridge.iter().map(|i| (i.opcode, i.arguments.iter().map(|a| a.raw_data).collect())).collect()
    }

    #[test]
    fn semantic_bridge_mirrors_native_without_scene_logic() {
        let bridge = build_semantic_bridge(&[], Some(&ladea_template()), false);
        assert_eq!(
            shape(&bridge),
            vec![
                (OP_END_TEXT_LINE, vec![0]),
                (OP_SET_TICK_PRESENTATION, vec![0]),
                (OP_RESET_TEXT_LAYOUT, vec![2]),
                (OP_RESET_TEXT_LAYOUT, vec![1]),
                (OP_STMT_BEGIN, vec![]),
                (OP_SET_TICK_PRESENTATION, vec![1]),
                (OP_NEW_BOX_OPEN, vec![]),
            ]
        );
        assert!(bridge.iter().all(|i| !crate::bin_format::opcodes::is_control_flow(i.opcode)));
    }

    #[test]
    fn semantic_bridge_keep_speaker_plate_skips_slot_2() {
        let bridge = build_semantic_bridge(&[], Some(&ladea_template()), true);
        let cleared: Vec<u32> = bridge
            .iter()
            .filter(|i| i.opcode == OP_RESET_TEXT_LAYOUT)
            .map(|i| i.arguments[0].raw_data)
            .collect();
        assert_eq!(cleared, vec![1]);
    }

    /// Scene with two call targets: a voice procedure (rejected) at 50 and a Sankai-shaped
    /// speaker box-begin at 100 (`0x1d2 2,t3:16152`, name `show-text 2`, no voice).
    fn scene_with_speaker_procedure() -> Vec<Instruction> {
        let at = |offset: u64, mut i: Instruction| {
            i.offset = offset;
            i
        };
        let mut metadata = immediate_instruction(OP_APPEND_HISTORY_METADATA, &[2, 16152]);
        metadata.arguments[1].arg_type = 3;
        vec![
            at(0, immediate_instruction(OP_WAIT_FOR_INPUT, &[0])),
            at(1, immediate_instruction(OP_END_TEXT_LINE, &[0])),
            at(2, immediate_instruction(OP_RESET_TEXT_LAYOUT, &[2])),
            at(3, immediate_instruction(OP_RESET_TEXT_LAYOUT, &[1])),
            at(4, immediate_instruction(OP_CALL, &[50])),
            at(5, immediate_instruction(OP_CALL, &[100])),
            at(6, immediate_instruction(OP_SHOW_TEXT, &[0, 0])),
            at(7, immediate_instruction(OP_CALL, &[50])),
            // SYS4: the build has statement blocks (elsewhere in the scene).
            at(8, immediate_instruction(OP_STMT_BEGIN, &[])),
            at(9, immediate_instruction(OP_NEW_BOX_OPEN, &[])),
            at(50, immediate_instruction(OP_APPEND_HISTORY_METADATA, &[2, 1])),
            at(51, immediate_instruction(OP_SHOW_TEXT, &[2, 0])),
            at(52, immediate_instruction(OP_PLAY_VOICE, &[7])),
            at(53, immediate_instruction(OP_RET, &[])),
            at(100, metadata),
            at(101, immediate_instruction(OP_SHOW_TEXT, &[2, 0])),
            at(102, immediate_instruction(OP_RET, &[])),
        ]
    }

    #[test]
    fn speaker_procedure_found_and_voice_procedure_rejected() {
        let scene = scene_with_speaker_procedure();
        assert_eq!(find_speaker_redraw_procedure(&scene), Some(100));
        assert_eq!(find_speaker_redraw_procedure(&scene[..8]), None, "targets outside the scene");
    }

    #[test]
    fn keep_speaker_bridge_clears_natively_and_redraws_the_speaker() {
        let scene = scene_with_speaker_procedure();
        let bridge = build_semantic_bridge(&scene, None, true);
        assert_eq!(
            shape(&bridge),
            vec![
                (OP_END_TEXT_LINE, vec![0]),
                (OP_RESET_TEXT_LAYOUT, vec![2]),
                (OP_RESET_TEXT_LAYOUT, vec![1]),
                (OP_STMT_BEGIN, vec![]),
                (OP_CALL, vec![100]),
                (OP_NEW_BOX_OPEN, vec![]),
            ]
        );
        // Without keep_speaker: no call, nothing scene-specific.
        assert!(build_semantic_bridge(&scene, None, false).iter().all(|i| i.opcode != OP_CALL));
    }

    /// Inserted (offset sentinel) copy of `instruction`.
    fn inserted(mut instruction: Instruction) -> Instruction {
        instruction.offset = u64::MAX;
        instruction
    }

    /// Export fix-up: a continuation after the speaking box keeps its redraw; one after
    /// a narration box (its transition never reached the box-begin) loses it.
    #[test]
    fn speaker_redraws_kept_for_speech_removed_after_narration() {
        let mut scene = scene_with_speaker_procedure();
        // Narration box: transition calls only the voice-free... nothing speaker-related.
        let narration = [
            immediate_instruction(OP_WAIT_FOR_INPUT, &[0]),
            immediate_instruction(OP_RESET_TEXT_LAYOUT, &[1]),
            immediate_instruction(OP_SHOW_TEXT, &[0, 0]),
        ];
        for (k, mut i) in narration.into_iter().enumerate() {
            i.offset = 10 + k as u64;
            scene.insert(8 + k, i);
        }
        let bridge: Vec<Instruction> = build_semantic_bridge(&scene, None, true).into_iter().map(inserted).collect();
        assert!(bridge.iter().any(|i| i.opcode == OP_CALL));
        // After the speaking box (#6) and after the narration box (#10).
        let mut exported = scene.clone();
        for &after in &[10usize, 6] {
            for (n, i) in bridge.iter().cloned().enumerate() {
                exported.insert(after + 1 + n, i);
            }
        }
        let resolved = resolve_speaker_redraws(exported);
        let redraws: Vec<usize> = resolved
            .iter()
            .enumerate()
            .filter(|(_, i)| i.offset == u64::MAX && i.opcode == OP_CALL)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(redraws.len(), 1, "only the speech continuation redraws");
        let speech = resolved.iter().position(|i| i.offset == 6).unwrap();
        assert!(redraws[0] > speech && redraws[0] < resolved.iter().position(|i| i.offset == 7).unwrap());
    }

    /// Atelial: the transition reaches the box-begin through a talk-key dispatcher
    /// (`speaker >= 0 ? box-begin : narrator`) → the redraw calls the dispatcher.
    #[test]
    fn speaker_redraw_retargeted_to_dispatcher() {
        let at = |offset: u64, mut i: Instruction| {
            i.offset = offset;
            i
        };
        let mut metadata = immediate_instruction(OP_APPEND_HISTORY_METADATA, &[2, 12608]);
        metadata.arguments[1].arg_type = 3;
        let mut scene = vec![
            at(0, immediate_instruction(OP_WAIT_FOR_INPUT, &[0])),
            at(1, immediate_instruction(OP_RESET_TEXT_LAYOUT, &[1])),
            at(2, immediate_instruction(OP_CALL, &[200])), // dispatcher
            at(3, immediate_instruction(OP_SHOW_TEXT, &[0, 0])),
            at(100, metadata),
            at(101, immediate_instruction(OP_SHOW_TEXT, &[1, 0])),
            at(102, immediate_instruction(OP_RET, &[])),
            at(200, immediate_instruction(OP_CALL, &[100])),
            at(201, immediate_instruction(OP_RET, &[])),
        ];
        let bridge: Vec<Instruction> = build_semantic_bridge(&scene, None, true).into_iter().map(inserted).collect();
        for (n, i) in bridge.into_iter().enumerate() {
            scene.insert(4 + n, i);
        }
        let resolved = resolve_speaker_redraws(scene);
        let redraw = resolved.iter().find(|i| i.offset == u64::MAX && i.opcode == OP_CALL).expect("kept");
        assert_eq!(redraw.arguments[0].raw_data, 200);
    }

    /// Sankai SC0700: the scene's FIRST box has no wait before it, so its window spans the
    /// preamble, where a voiced init routine also reaches the box-begin. The box's own
    /// (later) direct call must win — the redraw is kept, not dropped.
    #[test]
    fn first_box_redraw_uses_its_own_box_begin_call() {
        let at = |offset: u64, mut i: Instruction| {
            i.offset = offset;
            i
        };
        let mut metadata = immediate_instruction(OP_APPEND_HISTORY_METADATA, &[2, 16152]);
        metadata.arguments[1].arg_type = 3;
        let mut scene = vec![
            at(0, immediate_instruction(OP_CALL, &[300])), // voiced init that reaches the box-begin
            at(1, immediate_instruction(OP_RESET_TEXT_LAYOUT, &[1])),
            at(2, immediate_instruction(OP_CALL, &[100])), // the box's own box-begin
            at(3, immediate_instruction(OP_SHOW_TEXT, &[0, 0])),
            at(4, immediate_instruction(OP_STMT_BEGIN, &[])),
            at(5, immediate_instruction(OP_NEW_BOX_OPEN, &[])),
            at(100, metadata),
            at(101, immediate_instruction(OP_SHOW_TEXT, &[2, 0])),
            at(102, immediate_instruction(OP_RET, &[])),
            at(300, immediate_instruction(OP_CALL, &[100])),
            at(301, immediate_instruction(OP_PLAY_VOICE, &[7])),
            at(302, immediate_instruction(OP_RET, &[])),
        ];
        let bridge: Vec<Instruction> = build_semantic_bridge(&scene, None, true).into_iter().map(inserted).collect();
        for (n, i) in bridge.into_iter().enumerate() {
            scene.insert(4 + n, i);
        }
        let resolved = resolve_speaker_redraws(scene);
        let redraw = resolved.iter().find(|i| i.offset == u64::MAX && i.opcode == OP_CALL).expect("kept");
        assert_eq!(redraw.arguments[0].raw_data, 100);
    }

    #[test]
    fn semantic_bridge_without_gate_or_template_is_minimal() {
        // VERITA-style scene: clears slot 1, never re-enables presentation.
        let scene = vec![
            immediate_instruction(OP_WAIT_FOR_INPUT, &[0]),
            immediate_instruction(OP_END_TEXT_LINE, &[0]),
            immediate_instruction(OP_RESET_TEXT_LAYOUT, &[1]),
            immediate_instruction(OP_STMT_BEGIN, &[]),
            immediate_instruction(OP_NEW_BOX_OPEN, &[]),
            immediate_instruction(OP_SHOW_TEXT, &[0, 0]),
        ];
        let bridge = build_semantic_bridge(&scene, None, true);
        assert_eq!(
            shape(&bridge),
            vec![(OP_END_TEXT_LINE, vec![0]), (OP_RESET_TEXT_LAYOUT, vec![1]), (OP_STMT_BEGIN, vec![]), (OP_NEW_BOX_OPEN, vec![])]
        );
        // No evidence at all → still a usable body-slot clear.
        let fallback = build_semantic_bridge(&[], None, false);
        assert!(fallback.iter().any(|i| i.opcode == OP_RESET_TEXT_LAYOUT && i.arguments[0].raw_data == 1));
    }

    #[test]
    fn semantic_bridge_omits_statement_blocks_on_sys3() {
        // SYS3 scene (Meishoku 324): no 0x1F4/0x1F5 anywhere — the engine aborts on them.
        let scene = vec![
            immediate_instruction(OP_WAIT_FOR_INPUT, &[0]),
            immediate_instruction(OP_END_TEXT_LINE, &[0]),
            immediate_instruction(OP_RESET_TEXT_LAYOUT, &[1]),
            immediate_instruction(OP_SHOW_TEXT, &[0, 0]),
        ];
        for keep_speaker in [false, true] {
            let bridge = build_semantic_bridge(&scene, None, keep_speaker);
            assert_eq!(shape(&bridge), vec![(OP_END_TEXT_LINE, vec![0]), (OP_RESET_TEXT_LAYOUT, vec![1])]);
        }
    }

    #[test]
    fn semantic_bridge_arg_counts_match_opcode_table() {
        for i in build_semantic_bridge(&[], Some(&ladea_template()), false) {
            let def = definition_for_opcode(i.opcode).expect("known opcode");
            assert_eq!(def.argument_count as usize, i.arguments.len(), "{:#x}", i.opcode);
            assert_eq!(i.argument_count as usize, i.arguments.len());
        }
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

    fn text_line(offset: u64, text: &str) -> Instruction {
        let mut line = immediate_instruction(OP_SHOW_TEXT, &[0, 0]);
        line.arguments[1].arg_type = 2;
        line.arguments[1].decoded_string = Some(text.to_string());
        line.offset = offset;
        line
    }

    fn original(offset: u64, opcode: u32, values: &[u32]) -> Instruction {
        let mut instruction = immediate_instruction(opcode, values);
        instruction.offset = offset;
        instruction
    }

    /// A continuation (inserted wait + plain semantic bridge + inserted line) after `last`.
    fn continuation(text: &str) -> Vec<Instruction> {
        let mut inserted = vec![immediate_instruction(OP_WAIT_FOR_INPUT, &[0])];
        inserted.extend(build_semantic_bridge(&[], None, false));
        inserted.push(text_line(u64::MAX, text));
        inserted
    }

    fn texts(instructions: &[Instruction]) -> Vec<String> {
        instructions.iter().filter_map(|i| i.arguments.iter().find_map(|a| a.decoded_string.clone())).collect()
    }

    /// Minefuka box: name line, then the speech (indented), then a continuation.
    fn minefuka_box(first_line: &str) -> Vec<Instruction> {
        vec![
            original(0, OP_WAIT_FOR_INPUT, &[0]),
            original(1, OP_END_TEXT_LINE, &[0]),
            original(2, OP_RESET_TEXT_LAYOUT, &[1]),
            text_line(3, first_line),
            original(4, OP_END_TEXT_LINE, &[0]),
            text_line(5, "\u{3000}Who came up with these traps?!"),
            original(6, OP_END_TEXT_LINE, &[0]),
        ]
    }

    #[test]
    fn inline_name_carried_into_speech_continuation() {
        let original = minefuka_box("フィーノ");
        let mut scene = minefuka_box("Fino");
        scene.extend(continuation("\u{3000}I'll find them."));
        scene.extend(continuation("\u{3000}And then...!"));
        let out = carry_inline_speaker_names(&original, scene);
        assert_eq!(texts(&out), ["Fino", "\u{3000}Who came up with these traps?!", "Fino", "\u{3000}I'll find them.", "Fino", "\u{3000}And then...!"]);
        let copy = out.iter().rposition(|i| texts(std::slice::from_ref(i)) == ["Fino"]).unwrap();
        assert_eq!((out[copy].offset, out[copy + 1].opcode, out[copy + 1].offset), (u64::MAX, OP_END_TEXT_LINE, u64::MAX));
    }

    #[test]
    fn narration_continuation_gets_no_name() {
        // Original first line is narration (sentence punctuation); its short English
        // translation must not be mistaken for a name.
        let original = minefuka_box("俺の叫び声で二人が足を止める……なんだよ、その顔は。");
        let mut scene = minefuka_box("Silence.");
        scene.extend(continuation("Rude."));
        assert_eq!(texts(&carry_inline_speaker_names(&original, scene.clone())), texts(&scene));
    }

    #[test]
    fn no_inserted_boxes_no_change() {
        let scene = minefuka_box("フィーノ");
        assert_eq!(carry_inline_speaker_names(&scene, scene.clone()).len(), scene.len());
    }

    #[test]
    fn sys3_inserted_wait_gets_its_line_closed() {
        let inserted = |mut i: Instruction| {
            i.offset = u64::MAX;
            i
        };
        let modified = vec![
            immediate_instruction(OP_SHOW_TEXT, &[0, 0]),
            inserted(immediate_instruction(OP_WAIT_FOR_INPUT, &[0])), // inserted break: needs a close
            inserted(immediate_instruction(OP_END_TEXT_LINE, &[0])),
            inserted(immediate_instruction(OP_RESET_TEXT_LAYOUT, &[1])),
            inserted(immediate_instruction(OP_SHOW_TEXT, &[0, 0])),
            immediate_instruction(OP_END_TEXT_LINE, &[0]),
            immediate_instruction(OP_WAIT_FOR_INPUT, &[0]), // native, already closed
        ];
        let fixed = close_lines_before_inserted_waits(modified.clone());
        let ops: Vec<u32> = fixed.iter().map(|i| i.opcode).collect();
        assert_eq!(
            ops,
            vec![OP_SHOW_TEXT, OP_END_TEXT_LINE, OP_WAIT_FOR_INPUT, OP_END_TEXT_LINE, OP_RESET_TEXT_LAYOUT, OP_SHOW_TEXT, OP_END_TEXT_LINE, OP_WAIT_FOR_INPUT]
        );
        assert_eq!(fixed[1].offset, u64::MAX);
        // Idempotent: an already-closed inserted wait is left alone.
        assert_eq!(close_lines_before_inserted_waits(fixed.clone()).len(), fixed.len());
    }

    #[test]
    fn sys3_inserted_clear_copies_the_native_box_redraw() {
        let at = |offset: u64, mut i: Instruction| {
            i.offset = offset;
            i
        };
        let inserted = |i: Instruction| at(u64::MAX, i);
        let modified = vec![
            at(0, immediate_instruction(OP_RESET_TEXT_LAYOUT, &[1])),
            at(1, immediate_instruction(OP_CALL, &[100])), // native clear + window blit
            at(2, immediate_instruction(OP_SHOW_TEXT, &[0, 0])),
            inserted(immediate_instruction(OP_WAIT_FOR_INPUT, &[0])),
            inserted(immediate_instruction(OP_END_TEXT_LINE, &[0])),
            inserted(immediate_instruction(OP_RESET_TEXT_LAYOUT, &[1])),
            inserted(immediate_instruction(OP_SHOW_TEXT, &[0, 0])),
            at(100, immediate_instruction(0x11, &[3, 0, 208, 425, 555, 130, 208, 425, 1])),
            at(101, immediate_instruction(OP_RET, &[])),
        ];
        let fixed = redraw_box_after_inserted_clears(modified.clone());
        assert_eq!(fixed.len(), modified.len() + 1);
        assert_eq!(fixed[6].opcode, OP_CALL);
        assert_eq!(fixed[6].arguments[0].raw_data, 100);
        assert_eq!(fixed[6].offset, u64::MAX);
        assert_eq!(redraw_box_after_inserted_clears(fixed.clone()).len(), fixed.len(), "idempotent");
        // A voiced procedure is never copied.
        let mut voiced = modified.clone();
        voiced[7] = at(100, immediate_instruction(OP_PLAY_VOICE, &[7]));
        assert_eq!(redraw_box_after_inserted_clears(voiced.clone()).len(), voiced.len());
        // Minefuka: the run is `call <compose>` + a full-window blit; a break before the
        // first native clear borrows the next one.
        let mut minefuka = modified[3..].to_vec(); // inserted break first
        minefuka.splice(4..4, [
            at(0, immediate_instruction(OP_RESET_TEXT_LAYOUT, &[1])),
            at(1, immediate_instruction(OP_CALL, &[100])),
            at(2, immediate_instruction(OP_BLIT_LAYER, &[3, 0, 0, 0, 800, 600, 0, 0, 1])),
            at(3, immediate_instruction(OP_SHOW_TEXT, &[0, 0])),
        ]);
        let fixed = redraw_box_after_inserted_clears(minefuka);
        let ops: Vec<u32> = fixed[2..5].iter().map(|i| i.opcode).collect();
        assert_eq!(ops, vec![OP_RESET_TEXT_LAYOUT, OP_CALL, OP_BLIT_LAYER]);
        assert!(fixed[3..5].iter().all(|i| i.offset == u64::MAX));
    }
}
