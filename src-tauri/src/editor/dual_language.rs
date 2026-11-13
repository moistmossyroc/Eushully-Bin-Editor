//! Dual-language / speaker-box pipeline for SYS4 BINs.
//!
//! Three reusable stages, all driven by aligning a translated (English) BIN to its
//! original (Japanese) BIN beat-for-beat (a "beat" = the instructions up to a
//! `wait-for-input`):
//!
//! 1. **Box-split matcher** (`align_kinds`): the scene logic (non-text opcodes) is
//!    identical between languages, so each EN beat either matches the next JP beat
//!    (a *primary*) or it's an editor-inserted overflow box (a *continuation*).
//!    Falls back to a jcc/jmp-tolerant signature when a file has stripped branches.
//!
//! 2. **Narrator repair** (`repair_continuations`): continuation boxes lose the
//!    speaker's portrait/name/color because they only got a generic bridge. We
//!    clone the parent (primary) box's full setup into them (skipping `play-voice`
//!    0xC4 to avoid the replay crash), remapping internal jumps to synthetic
//!    offsets the writer resolves.
//!
//! 3. **Bilingual generation** (`build_bilingual`): on the repaired BIN, each
//!    primary box's text region becomes `if(flag){EN}else{JP}` (JP text cloned from
//!    the original), and each beat's continuation boxes are guarded so they're
//!    skipped in Japanese mode (the JP original is a single box). The flag is the
//!    `LANG_SENTINEL` the loader patches at runtime → instant mid-scene switch.
//!
//! Proven in-game on Madou (1:1 beats) and Sankai (box-splitting translation).

use crate::bin_format::instruction::{Argument, Instruction};
use crate::bin_format::opcodes::{is_control_flow, is_label_argument};
use crate::bin_format::parser::{parse_bin, ParsedBin};
use crate::bin_format::writer::write_bin;
use std::collections::HashMap;

/// Distinctive constant ("LANG") embedded in every generated `eq`; the loader
/// scans for it and writes this value (English) or 0 (Japanese) into the flag.
pub const LANG_SENTINEL: u32 = 0x4C41_4E47;

// Opcodes referenced by the pipeline.
const OP_EQ: u32 = 0x5A;
const OP_JCC: u32 = 0xA0;
const OP_JMP: u32 = 0x8C;
const OP_WAIT: u32 = 0x72;
const OP_SHOW_TEXT: u32 = 0x6E;
const OP_END_LINE: u32 = 0x6F;
const OP_FURIGANA: u32 = 0x196;
const OP_NEW_BOX: u32 = 0x1F5;
const OP_PLAY_VOICE: u32 = 0xC4;

// Synthetic-offset bases for cloned instructions (kept disjoint from real file
// offsets, which are small, and from `u64::MAX`). Each clone gets a unique key the
// writer includes in its old→new map so internal control-flow resolves.
const SYNTH_REPAIR: u32 = 0x8000_0000;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// EN beat that matches JP beat N.
    Primary(usize),
    /// Editor-inserted overflow box (no JP counterpart).
    Continuation,
}

/// Block boundaries: `(start_idx, wait_idx)` per beat (split on `wait-for-input`).
fn beats(ins: &[Instruction]) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut start = 0usize;
    for (i, x) in ins.iter().enumerate() {
        if x.opcode == OP_WAIT {
            v.push((start, i));
            start = i + 1;
        }
    }
    v
}

/// Per-beat signature = opcode sequence excluding text-rendering ops (which reflow).
/// `tolerant` additionally drops jcc/jmp, for files whose branches were stripped.
fn beat_sig(ins: &[Instruction], s: usize, e: usize, tolerant: bool) -> Vec<u32> {
    (s..=e)
        .map(|i| ins[i].opcode)
        .filter(|&o| {
            !matches!(o, OP_SHOW_TEXT | OP_END_LINE | OP_FURIGANA)
                && !(tolerant && matches!(o, OP_JCC | OP_JMP))
        })
        .collect()
}

/// The box-open (`0x1F5`) immediately preceding a beat's first `show-text`.
fn setup_open(ins: &[Instruction], s: usize, e: usize) -> Option<usize> {
    let show = (s..=e).find(|&i| ins[i].opcode == OP_SHOW_TEXT)?;
    (s..show).rev().find(|&i| ins[i].opcode == OP_NEW_BOX)
}

fn align_pass(en: &[Instruction], jp: &[Instruction], tolerant: bool) -> Option<Vec<Kind>> {
    let jb = beats(jp);
    let eb = beats(en);
    let jsig: Vec<Vec<u32>> = jb.iter().map(|&(s, e)| beat_sig(jp, s, e, tolerant)).collect();
    let mut out = Vec::with_capacity(eb.len());
    let mut jp_idx = 0;
    for &(s, e) in &eb {
        let es = beat_sig(en, s, e, tolerant);
        if jp_idx < jb.len() && es == jsig[jp_idx] {
            out.push(Kind::Primary(jp_idx));
            jp_idx += 1;
        } else {
            out.push(Kind::Continuation);
        }
    }
    // Clean alignment only: every JP beat must have been matched.
    if jp_idx != jb.len() {
        return None;
    }
    Some(out)
}

/// Align EN beats to JP beats. Tries an exact signature, then a jcc/jmp-tolerant
/// one. `None` if neither aligns cleanly (caller should leave the BIN untouched).
fn align_kinds(en: &[Instruction], jp: &[Instruction]) -> Option<Vec<Kind>> {
    align_pass(en, jp, false).or_else(|| align_pass(en, jp, true))
}

fn clone_arg(a: &Argument, force_reencode_strings: bool) -> Argument {
    Argument {
        arg_type: a.arg_type,
        raw_data: a.raw_data,
        decoded_string: a.decoded_string.clone(),
        original_encoded_bytes: if force_reencode_strings && a.arg_type == 2 {
            None
        } else {
            a.original_encoded_bytes.clone()
        },
        data_array: a.data_array.clone(),
    }
}

/// Clone instructions at `region` indices, skipping `play-voice`, assigning each a
/// unique synthetic offset (from `*synth`) and remapping internal control-flow
/// targets to those synthetic offsets (external targets keep their original value).
fn clone_region(ins: &[Instruction], region: &[usize], synth: &mut u32) -> Vec<Instruction> {
    let kept: Vec<usize> = region.iter().cloned().filter(|&i| ins[i].opcode != OP_PLAY_VOICE).collect();
    let base = *synth;
    let mut off_map: HashMap<u32, u32> = HashMap::new();
    for (k, &i) in kept.iter().enumerate() {
        off_map.insert(ins[i].offset as u32, base + k as u32);
    }
    *synth = base + kept.len() as u32;
    kept.iter()
        .enumerate()
        .map(|(k, &i)| {
            let src = &ins[i];
            let mut args: Vec<Argument> = src.arguments.iter().map(|a| clone_arg(a, true)).collect();
            if is_control_flow(src.opcode) {
                for (ai, a) in args.iter_mut().enumerate() {
                    if is_label_argument(src.opcode, ai, a.arg_type, a.raw_data) {
                        if let Some(&n) = off_map.get(&a.raw_data) {
                            a.raw_data = n; // internal target → cloned copy
                        }
                    }
                }
            }
            Instruction {
                opcode: src.opcode,
                label: src.label.clone(),
                argument_count: src.argument_count,
                arguments: args,
                offset: (base + k as u32) as u64,
            }
        })
        .collect()
}

/// Clone a beat's text region (show-text / end-text-line / furigana only — no
/// control flow) with synthetic offsets, for use as a language branch.
fn clone_text(ins: &[Instruction], s: usize, e: usize, synth: &mut u32) -> Vec<Instruction> {
    let r: Vec<usize> = (s..=e)
        .filter(|&i| matches!(ins[i].opcode, OP_SHOW_TEXT | OP_END_LINE | OP_FURIGANA))
        .collect();
    let base = *synth;
    *synth += r.len() as u32;
    r.iter()
        .enumerate()
        .map(|(k, &i)| {
            let src = &ins[i];
            Instruction {
                opcode: src.opcode,
                label: src.label.clone(),
                argument_count: src.argument_count,
                arguments: src.arguments.iter().map(|a| clone_arg(a, true)).collect(),
                offset: (base + k as u32) as u64,
            }
        })
        .collect()
}

fn lit(v: u32) -> Argument {
    Argument { arg_type: 0, raw_data: v, decoded_string: None, original_encoded_bytes: None, data_array: None }
}
fn reg(i: u32) -> Argument {
    Argument { arg_type: 9, raw_data: i, decoded_string: None, original_encoded_bytes: None, data_array: None }
}
fn new_instr(op: u32, label: &str, args: Vec<Argument>) -> Instruction {
    Instruction { opcode: op, label: label.into(), argument_count: args.len() as u32, arguments: args, offset: u64::MAX }
}

/// Report from a dual-language build.
#[derive(Debug, Clone, Default)]
pub struct DualReport {
    pub primaries: usize,
    pub continuations: usize,
}

/// Narrator-box repair: clone each continuation box's parent (primary) speaker
/// setup into it. Returns the repaired instruction list, or `None` if EN/JP don't
/// align cleanly (leave the BIN untouched). Output offsets: originals keep theirs,
/// clones use synthetic `SYNTH_REPAIR+` keys the writer resolves.
pub fn repair_continuations(en: &[Instruction], jp: &[Instruction]) -> Option<(Vec<Instruction>, DualReport)> {
    let kinds = align_kinds(en, jp)?;
    let eb = beats(en);
    let mut synth = SYNTH_REPAIR;
    let mut repl: HashMap<usize, (usize, Vec<Instruction>)> = HashMap::new();
    let mut last_primary: Option<usize> = None;
    let mut rep = DualReport::default();
    for bi in 0..eb.len() {
        match kinds[bi] {
            Kind::Primary(_) => {
                last_primary = Some(bi);
                rep.primaries += 1;
            }
            Kind::Continuation => {
                rep.continuations += 1;
                if let Some(pi) = last_primary {
                    let (cs, ce) = eb[bi];
                    let (ps, pe) = eb[pi];
                    if let (Some(co), Some(po)) = (setup_open(en, cs, ce), setup_open(en, ps, pe)) {
                        let parent_region: Vec<usize> = ((ps + 1)..=po).collect();
                        let clones = clone_region(en, &parent_region, &mut synth);
                        repl.insert(cs + 1, (co, clones));
                    }
                }
            }
        }
    }
    let mut out = Vec::with_capacity(en.len() + repl.len() * 8);
    let mut i = 0;
    while i < en.len() {
        if let Some((end, clones)) = repl.get(&i) {
            out.extend(clones.iter().cloned());
            i = *end + 1;
            continue;
        }
        out.push(en[i].clone());
        i += 1;
    }
    Some((out, rep))
}

/// Build the bilingual instruction list from a (repaired) base BIN + the JP
/// original + the alignment `kinds`. Primary text regions become EN/JP
/// conditionals; the first continuation of each beat is guarded to skip in JP.
fn build_bilingual_core(base: &ParsedBin, jp: &ParsedBin, kinds: &[Kind]) -> Option<Vec<Instruction>> {
    let reb = beats(&base.instructions);
    let jb = beats(&jp.instructions);
    if reb.len() != kinds.len() {
        return None;
    }
    let mut synth = SYNTH_REPAIR; // fresh per build (base re-parsed → real offsets, no clash)
    let mut ins_before: HashMap<usize, Vec<Instruction>> = HashMap::new();
    let mut ins_after: HashMap<usize, Vec<Instruction>> = HashMap::new();
    for bi in 0..reb.len() {
        let (s, e) = reb[bi];
        match kinds[bi] {
            Kind::Primary(jpi) => {
                let Some(trs) = (s..=e).find(|&i| base.instructions[i].opcode == OP_SHOW_TEXT) else { continue };
                let tre = (s..=e).rev().find(|&i| matches!(base.instructions[i].opcode, OP_SHOW_TEXT | OP_END_LINE | OP_FURIGANA)).unwrap();
                if jpi >= jb.len() { continue; }
                let (js, je) = jb[jpi];
                let Some(jtrs) = (js..=je).find(|&i| jp.instructions[i].opcode == OP_SHOW_TEXT) else { continue };
                let jtre = (js..=je).rev().find(|&i| matches!(jp.instructions[i].opcode, OP_SHOW_TEXT | OP_END_LINE | OP_FURIGANA)).unwrap();
                let jpclone = clone_text(&jp.instructions, jtrs, jtre, &mut synth);
                if jpclone.is_empty() || tre + 1 >= base.instructions.len() { continue; }
                let jp_first = jpclone[0].offset as u32;
                let after = base.instructions[tre + 1].offset as u32;
                ins_before.entry(trs).or_default().extend([
                    new_instr(OP_EQ, "eq", vec![reg(0), lit(LANG_SENTINEL), lit(LANG_SENTINEL)]),
                    new_instr(OP_JCC, "jcc", vec![reg(0), lit(0xFFFF_FFFF), lit(jp_first)]),
                ]);
                let mut tail = vec![new_instr(OP_JMP, "jmp", vec![lit(after)])];
                tail.extend(jpclone);
                ins_after.entry(tre).or_default().extend(tail);
            }
            Kind::Continuation => {
                let first = bi == 0 || kinds[bi - 1] != Kind::Continuation;
                if first {
                    let np = (bi + 1..reb.len()).find(|&k| matches!(kinds[k], Kind::Primary(_)));
                    let skip = match np {
                        Some(k) => base.instructions[reb[k].0].offset as u32,
                        None => base.instructions[base.instructions.len() - 1].offset as u32,
                    };
                    ins_before.entry(s).or_default().extend([
                        new_instr(OP_EQ, "eq", vec![reg(0), lit(LANG_SENTINEL), lit(LANG_SENTINEL)]),
                        new_instr(OP_JCC, "jcc", vec![reg(0), lit(0xFFFF_FFFF), lit(skip)]),
                    ]);
                }
            }
        }
    }
    let mut out = Vec::new();
    for i in 0..base.instructions.len() {
        if let Some(v) = ins_before.get(&i) { out.extend(v.iter().cloned()); }
        out.push(base.instructions[i].clone());
        if let Some(v) = ins_after.get(&i) { out.extend(v.iter().cloned()); }
    }
    Some(out)
}

/// Full pipeline: align → repair continuations → bilingual. Returns the bilingual
/// BIN bytes (ready to write), or `Err` with a reason if it can't be produced
/// (the caller should then ship the plain translated BIN unchanged).
pub fn make_bilingual(en: &ParsedBin, jp: &ParsedBin) -> Result<Vec<u8>, String> {
    let kinds = align_kinds(&en.instructions, &jp.instructions).ok_or("EN/JP beats do not align")?;
    // Stage 1: repair (synthetic offsets resolved by writing, then re-parse so the
    // bilingual stage sees real offsets and can use a fresh synthetic range).
    let (repaired_ins, _) = repair_continuations(&en.instructions, &jp.instructions).ok_or("repair: align failed")?;
    let repaired_bytes = write_bin(&en.header, &repaired_ins).map_err(|e| format!("repair write: {e:?}"))?;
    let repaired = parse_bin(&repaired_bytes).map_err(|e| format!("repair roundtrip: {e:?}"))?;
    // Stage 2: bilingual on the repaired base, using the original alignment.
    let bi = build_bilingual_core(&repaired, jp, &kinds).ok_or("bilingual: beat mismatch")?;
    let bytes = write_bin(&repaired.header, &bi).map_err(|e| format!("bilingual write: {e:?}"))?;
    parse_bin(&bytes).map_err(|e| format!("bilingual roundtrip: {e:?}"))?; // corruption guard
    Ok(bytes)
}

/// Inline each speaker-name display with a literal name from `id_to_name`, keyed by
/// the current speaker id. A name display is a `show-text` that READS the speaker
/// register (the engine fills that register once at boot from CVINIT, so the name
/// can't flip live); inlining moves the name INTO the flag-gated text region so
/// `make_bilingual` swaps it with the dialogue. Displays whose speaker doesn't
/// resolve (e.g. the literal "？？？？" unknown) are left untouched.
pub fn inline_speaker_names(parsed: &ParsedBin, id_to_name: &HashMap<u32, String>) -> Vec<Instruction> {
    use crate::editor::characters::{detect_speaker_register, extract_row_speakers};
    let ins = &parsed.instructions;
    let mut out = ins.to_vec();
    let Some(reg) = detect_speaker_register(ins) else { return out };
    let rows = extract_row_speakers(ins, reg); // show-text index → speaker
    for (i, x) in ins.iter().enumerate() {
        if x.opcode != OP_SHOW_TEXT {
            continue;
        }
        // A name display references the speaker register (non-literal, non-inline arg).
        if !x.arguments.iter().any(|a| a.arg_type != 0 && a.arg_type != 2 && a.raw_data == reg) {
            continue;
        }
        let Some(sp) = rows.get(&i) else { continue };
        let Some(name) = id_to_name.get(&sp.source_raw_data) else { continue };
        // Replace the register reference with an inline literal — the writer
        // recomputes string offsets, so raw_data is irrelevant; clearing
        // original_encoded_bytes forces a re-encode from the name string.
        for a in out[i].arguments.iter_mut() {
            if a.arg_type != 0 && a.arg_type != 2 && a.raw_data == reg {
                a.arg_type = 2;
                a.raw_data = 0;
                a.decoded_string = Some(name.clone());
                a.original_encoded_bytes = None;
                a.data_array = None;
            }
        }
    }
    out
}

/// Like [`make_bilingual`], but first inlines each speaker-name display per language
/// (EN names into the EN scene, JP into the JP scene) so the name swaps with the
/// dialogue. `en_names`/`jp_names` map speaker id → name (build from CVINIT via
/// `extract_all_character_definitions`). Speakers not in the maps keep their
/// register reference (fall back to the boot-loaded name).
pub fn make_bilingual_with_names(
    en: &ParsedBin,
    jp: &ParsedBin,
    en_names: &HashMap<u32, String>,
    jp_names: &HashMap<u32, String>,
) -> Result<Vec<u8>, String> {
    let en2 = ParsedBin { header: en.header.clone(), instructions: inline_speaker_names(en, en_names) };
    let jp2 = ParsedBin { header: jp.header.clone(), instructions: inline_speaker_names(jp, jp_names) };
    make_bilingual(&en2, &jp2)
}

/// Narrator-repair only (no language conditional). Returns repaired BIN bytes.
pub fn make_repaired(en: &ParsedBin, jp: &ParsedBin) -> Result<Vec<u8>, String> {
    let (repaired_ins, _) = repair_continuations(&en.instructions, &jp.instructions).ok_or("repair: EN/JP beats do not align")?;
    let bytes = write_bin(&en.header, &repaired_ins).map_err(|e| format!("repair write: {e:?}"))?;
    parse_bin(&bytes).map_err(|e| format!("repair roundtrip: {e:?}"))?;
    Ok(bytes)
}
