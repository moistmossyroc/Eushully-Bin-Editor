//! Fanpatch "branding" for SYS4 games: a title badge (append-style banner or
//! tile) and an extra boot splash, all served by the fanpatch loader from NEW
//! catalog records — no official image or archive is modified.
//!
//! Every mechanism here was proven in-game first (docs/engine/age-sys.md §4–5):
//! banner array on Madou, tile row + splash cycle on Tenmei. Detection is
//! pattern-based on the title/logo scripts so other builds are recognised
//! automatically; anything unrecognised is reported as unsupported instead of
//! guessed.

use crate::bin_format::instruction::{Argument, Instruction};

const OP_JMP: u32 = 0x8c;
const OP_JCC: u32 = 0xa0;
const OP_CALL: u32 = 0x8f;
const OP_MOV: u32 = 0x55;
const OP_ADD: u32 = 0x50;
const OP_LT: u32 = 0x5c;
const OP_LOOKUP_ARRAY: u32 = 0x61;
const OP_CHECK_BIT: u32 = 0x13f;
const OP_SET_TEXTURE: u32 = 0x1f9;
const OP_DRAW_TEXTURE: u32 = 0x1fb;
const OP_FADE_COLOR: u32 = 0x202;
const OP_FADE_EFFECT: u32 = 0x323;
/// `0x101` reset-message-skip-input + `0x21c` render-wait: the single-cycle logo's
/// hold (blocks until its fades finish, ~5 s observed — inferred, not decompiled).
const OP_RESET_SKIP: u32 = 0x101;
const OP_RENDER_WAIT: u32 = 0x21c;

const ARG_IMMEDIATE: u32 = 0;
const ARG_GLOBAL_INT: u32 = 3;
const ARG_LOCAL_INT: u32 = 9;
const ARG_LOCAL_PTR: u32 = 12;

/// Banner slots the title loops over are 1..31; appends use their own number.
const BANNER_SLOT: u32 = 31;
/// Fallback banner size (Madou's official 320×30).
const DEFAULT_BANNER_SIZE: (u32, u32) = (320, 30);

/// How this game's title can show a fanpatch badge.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BadgePlan {
    /// TITLE loops `G[array_base + i]` (i = 1..31) and stacks banner AGFs.
    #[serde(rename_all = "camelCase")]
    BannerArray { array_base: u32, insert_at: usize, width: u32, height: u32 },
    /// TITLE draws pre-made append tiles in a row from an installed-mask loop.
    #[serde(rename_all = "camelCase")]
    TileRow { insert_at: usize, object_id: u32, texture_slot: u32, x_local: u32, y: u32, width: u32, height: u32 },
    Unsupported { reason: String },
}

/// How this game's boot logo script can show one more splash.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SplashPlan {
    /// LOGO shows ≥1 image cycle into a fixed texture slot, each ending in a wait call.
    #[serde(rename_all = "camelCase")]
    ImageCycles { texture_slot: u32, last_load: usize, cycle_end: usize, reveal: usize, hide: usize },
    /// LOGO shows one image: overlay R fades out (reveal), overlay H fades in at
    /// ~4.5 s (hide), then `0x101` + `0x21c` wait (Madou/LaDEA/Sankai).
    #[serde(rename_all = "camelCase")]
    SingleCycle { texture_slot: u32, reveal: usize, reveal_object: u32, hide: usize, hide_object: u32, wait: usize },
    /// LOGO shows splash ids from globals, one guarded cross-fade block each
    /// (Hyakusen, SYS5 502). `copy` = instruction indices of the last block
    /// (guards removed); `load` = its set-texture.
    #[serde(rename_all = "camelCase")]
    CrossfadeCycles { texture_slot: u32, copy: Vec<usize>, load: usize, insert_at: usize },
    Unsupported { reason: String },
}

fn arg(arg_type: u32, raw_data: u32) -> Argument {
    Argument { arg_type, raw_data, decoded_string: None, original_encoded_bytes: None, data_array: None }
}

fn instruction(opcode: u32, label: &str, arguments: Vec<Argument>) -> Instruction {
    Instruction { opcode, label: label.into(), argument_count: arguments.len() as u32, arguments, offset: u64::MAX }
}

fn is(instruction: &Instruction, opcode: u32, index: usize, arg_type: u32) -> bool {
    instruction.opcode == opcode && instruction.arguments.get(index).map(|a| a.arg_type == arg_type).unwrap_or(false)
}

/// Index of the instruction at `offset` (control-flow targets are offsets).
fn index_at_offset(instructions: &[Instruction], offset: u32) -> Option<usize> {
    instructions.iter().position(|i| i.offset == offset as u64)
}

/// The loop's entry `jmp` (forward jump to its condition) that precedes `body`.
fn loop_entry_jump(instructions: &[Instruction], body: usize) -> Option<usize> {
    (body.saturating_sub(12)..body).rev().find(|&k| {
        let i = &instructions[k];
        i.opcode == OP_JMP && i.arguments.first().map(|a| a.raw_data as u64 > i.offset).unwrap_or(false)
    })
}

fn detect_banner_array(title: &[Instruction]) -> Option<BadgePlan> {
    for (k, read) in title.iter().enumerate() {
        if !(is(read, OP_LOOKUP_ARRAY, 0, ARG_LOCAL_PTR) && is(read, OP_LOOKUP_ARRAY, 1, ARG_GLOBAL_INT)) {
            continue;
        }
        let pointer = read.arguments[0].raw_data;
        let loads_texture = title[k + 1..(k + 6).min(title.len())].iter().any(|i| {
            i.opcode == OP_SET_TEXTURE && i.arguments[0].arg_type == ARG_LOCAL_PTR && i.arguments[0].raw_data == pointer
        });
        if !loads_texture {
            continue;
        }
        let insert_at = loop_entry_jump(title, k)?;
        // Stack step: `add y, y, imm` after the draw → banners are at most `imm` tall.
        let draw = (k..(k + 16).min(title.len())).find(|&i| title[i].opcode == OP_DRAW_TEXTURE)?;
        let y_local = title[draw].arguments.get(7).filter(|a| a.arg_type == ARG_LOCAL_INT)?.raw_data;
        let step = title[draw + 1..(draw + 4).min(title.len())].iter().find_map(|i| {
            (i.opcode == OP_ADD
                && i.arguments[0].arg_type == ARG_LOCAL_INT
                && i.arguments[0].raw_data == y_local
                && i.arguments[2].arg_type == ARG_IMMEDIATE)
                .then(|| i.arguments[2].raw_data)
        });
        let height = step.map(|s| s.min(DEFAULT_BANNER_SIZE.1)).unwrap_or(DEFAULT_BANNER_SIZE.1);
        return Some(BadgePlan::BannerArray {
            array_base: read.arguments[1].raw_data,
            insert_at,
            width: DEFAULT_BANNER_SIZE.0,
            height,
        });
    }
    None
}

fn used_texture_slots(script: &[Instruction]) -> Vec<u32> {
    script
        .iter()
        .filter(|i| i.opcode == OP_SET_TEXTURE && i.arguments.get(1).map(|a| a.arg_type == ARG_IMMEDIATE).unwrap_or(false))
        .map(|i| i.arguments[1].raw_data)
        .collect()
}

fn free_texture_slot(script: &[Instruction]) -> u32 {
    let used = used_texture_slots(script);
    (60..100).rev().find(|slot| !used.contains(slot)).unwrap_or(99)
}

fn detect_tile_row(title: &[Instruction]) -> Option<BadgePlan> {
    let check = title.iter().position(|i| is(i, OP_CHECK_BIT, 1, ARG_GLOBAL_INT))?;
    // Loop condition `lt flag, counter, bound` + its exit `jcc` just before the check.
    let exit_jcc = (check.saturating_sub(6)..check).rev().find(|&k| title[k].opcode == OP_JCC)?;
    let bound = (exit_jcc.saturating_sub(2)..exit_jcc).find_map(|k| {
        (title[k].opcode == OP_LT && title[k].arguments[2].arg_type == ARG_IMMEDIATE).then(|| title[k].arguments[2].raw_data)
    })?;
    let exit_index = index_at_offset(title, title[exit_jcc].arguments[2].raw_data)?;
    let draw_index = (check..exit_index).find(|&k| title[k].opcode == OP_DRAW_TEXTURE)?;
    let draw = &title[draw_index];
    let [object, _slot, _sx, sy, width, height, x, y] = draw.arguments.as_slice() else { return None };
    if x.arg_type != ARG_LOCAL_INT || y.arg_type != ARG_IMMEDIATE || width.arg_type != ARG_IMMEDIATE || height.arg_type != ARG_IMMEDIATE || sy.arg_type != ARG_IMMEDIATE {
        return None;
    }
    // Object id = `add obj_local, base, counter` → our tile takes the last id of the range.
    let object_base = (check..draw_index).find_map(|k| {
        let i = &title[k];
        (i.opcode == OP_ADD && i.arguments[0].arg_type == object.arg_type && i.arguments[0].raw_data == object.raw_data && i.arguments[1].arg_type == ARG_IMMEDIATE)
            .then(|| i.arguments[1].raw_data)
    })?;
    Some(BadgePlan::TileRow {
        insert_at: exit_index + 1, // after the loop-exit target (a jump target stays first)
        object_id: object_base + bound - 1,
        texture_slot: free_texture_slot(title),
        x_local: x.raw_data,
        y: y.raw_data,
        width: width.raw_data,
        height: height.raw_data,
    })
}

/// Work out how the title can show a badge (banner array first, then tile row).
pub fn plan_badge(title: &[Instruction]) -> BadgePlan {
    detect_banner_array(title)
        .or_else(|| detect_tile_row(title))
        .unwrap_or(BadgePlan::Unsupported {
            reason: "title has no append banner/tile loop (e.g. VERITA, Himegari, Megami Zero) — explicit-draw badge not implemented yet".into(),
        })
}

/// Insert the badge into the title script for catalog record `badge_id`.
pub fn apply_badge(title: &[Instruction], plan: &BadgePlan, badge_id: u32) -> Result<Vec<Instruction>, String> {
    let mut out = title.to_vec();
    match *plan {
        BadgePlan::BannerArray { array_base, insert_at, .. } => {
            out.insert(insert_at, instruction(OP_MOV, "mov", vec![arg(ARG_GLOBAL_INT, array_base + BANNER_SLOT), arg(ARG_IMMEDIATE, badge_id)]));
        }
        BadgePlan::TileRow { insert_at, object_id, texture_slot, x_local, y, width, height } => {
            let new = [
                instruction(OP_SET_TEXTURE, "set-texture", vec![arg(ARG_IMMEDIATE, badge_id), arg(ARG_IMMEDIATE, texture_slot), arg(ARG_IMMEDIATE, u32::MAX)]),
                instruction(OP_DRAW_TEXTURE, "draw-texture", vec![
                    arg(ARG_IMMEDIATE, object_id), arg(ARG_IMMEDIATE, texture_slot), arg(ARG_IMMEDIATE, 0), arg(ARG_IMMEDIATE, 0),
                    arg(ARG_IMMEDIATE, width), arg(ARG_IMMEDIATE, height), arg(ARG_LOCAL_INT, x_local), arg(ARG_IMMEDIATE, y),
                ]),
            ];
            out.splice(insert_at..insert_at, new);
        }
        BadgePlan::Unsupported { ref reason } => return Err(reason.clone()),
    }
    Ok(out)
}

/// Object-relative fade on `add t9:L, base, k` + op(t9:L, delay, dur, target, …):
/// returns (index of the fade op, its `k`).
fn object_fade(logo: &[Instruction], opcode: u32, delayed: bool, target: u32) -> Option<(usize, u32)> {
    logo.iter().enumerate().find_map(|(k, i)| {
        let matches = i.opcode == opcode
            && i.arguments.len() >= 4
            && (if delayed { i.arguments[1].raw_data > 1000 } else { i.arguments[1].raw_data == 0 })
            && i.arguments[3].raw_data == target;
        let add = logo.get(k.checked_sub(1)?)?;
        (matches && add.opcode == OP_ADD && add.arguments.get(2).map(|a| a.arg_type == ARG_IMMEDIATE).unwrap_or(false))
            .then(|| (k, add.arguments[2].raw_data))
    })
}

/// Hyakusen (SYS5 502) style: splash ids come from globals set by INIT2, each
/// shown into the same slot by a self-contained block guarded by `jcc G[..]`:
/// fade the previous splash out + wait, load, draw, fade in + wait. The last
/// block (minus its guards) is cloned with our id.
fn detect_crossfade_cycles(logo: &[Instruction]) -> Option<SplashPlan> {
    let global_loads: Vec<usize> = logo
        .iter()
        .enumerate()
        .filter(|(_, i)| is(i, OP_SET_TEXTURE, 0, ARG_GLOBAL_INT) && is(i, OP_SET_TEXTURE, 1, ARG_IMMEDIATE))
        .map(|(k, _)| k)
        .collect();
    let last_load = *global_loads.last()?;
    let texture_slot = logo[last_load].arguments[1].raw_data;
    if global_loads.iter().filter(|&&k| logo[k].arguments[1].raw_data == texture_slot).count() < 2 {
        return None;
    }
    let calls_before: Vec<usize> = (0..last_load).filter(|&k| logo[k].opcode == OP_CALL).collect();
    // The block starts after the wait that ends the PREVIOUS splash (two calls back).
    let [.., block_start_call, _previous_fade_wait] = calls_before.as_slice() else { return None };
    let cycle_end = (last_load..logo.len()).find(|&k| logo[k].opcode == OP_CALL)?;
    let copy: Vec<usize> = (block_start_call + 1..=cycle_end).filter(|&k| logo[k].opcode != OP_JCC).collect();
    Some(SplashPlan::CrossfadeCycles { texture_slot, copy, load: last_load, insert_at: cycle_end + 1 })
}

/// Work out whether LOGO can show one more splash.
pub fn plan_splash(logo: &[Instruction]) -> SplashPlan {
    let loads: Vec<usize> = logo
        .iter()
        .enumerate()
        .filter(|(_, i)| is(i, OP_SET_TEXTURE, 0, ARG_IMMEDIATE) && is(i, OP_SET_TEXTURE, 1, ARG_IMMEDIATE))
        .map(|(k, _)| k)
        .collect();
    if let Some(plan) = detect_crossfade_cycles(logo) {
        return plan;
    }
    if loads.is_empty() {
        return SplashPlan::Unsupported { reason: "logo is movie-only (SYS4 415/422/424) or SYS3 — self-built splash not implemented yet".into() };
    }
    // Tenmei style: the splash slot is loaded once per cycle, each cycle ends in a wait call.
    let repeated = loads.iter().map(|&k| logo[k].arguments[1].raw_data).find(|slot| loads.iter().filter(|&&k| logo[k].arguments[1].raw_data == *slot).count() >= 2);
    if let Some(texture_slot) = repeated {
        let last_load = *loads.iter().rev().find(|&&k| logo[k].arguments[1].raw_data == texture_slot).unwrap();
        let Some(cycle_end) = (last_load..logo.len()).find(|&k| logo[k].opcode == OP_CALL) else {
            return SplashPlan::Unsupported { reason: "logo cycle has no wait call".into() };
        };
        // Reveal and hide must act on DIFFERENT objects (one fade channel per object).
        let reveal = object_fade(logo, OP_FADE_EFFECT, false, 0).map(|(k, _)| k);
        let hide = object_fade(logo, OP_FADE_COLOR, true, 255).map(|(k, _)| k);
        return match (reveal, hide) {
            (Some(reveal), Some(hide)) => SplashPlan::ImageCycles { texture_slot, last_load, cycle_end, reveal, hide },
            _ => SplashPlan::Unsupported { reason: "logo cycles lack a reveal/hide fade pair".into() },
        };
    }
    // Single-cycle style (443/445/448): one image, overlay R reveals, overlay H hides, then wait.
    let (Some((reveal, reveal_object)), Some((hide, hide_object))) =
        (object_fade(logo, OP_FADE_EFFECT, false, 0), object_fade(logo, OP_FADE_EFFECT, true, 255))
    else {
        return SplashPlan::Unsupported { reason: "logo image has no reveal/hide overlay pair".into() };
    };
    let Some(wait) = (hide..logo.len()).find(|&k| logo[k].opcode == OP_RENDER_WAIT && k > 0 && logo[k - 1].opcode == OP_RESET_SKIP) else {
        return SplashPlan::Unsupported { reason: "logo has no render-wait after its fades".into() };
    };
    let texture_slot = logo[*loads.iter().rev().find(|&&k| k < reveal).unwrap_or(&loads[0])].arguments[1].raw_data;
    SplashPlan::SingleCycle { texture_slot, reveal, reveal_object, hide, hide_object, wait }
}

/// Append one splash cycle for catalog record `splash_id` after the official one(s).
pub fn apply_splash(logo: &[Instruction], plan: &SplashPlan, splash_id: u32) -> Result<Vec<Instruction>, String> {
    let (insert_at, mut cycle) = match plan {
        // Tenmei: colour-key setup + load, overlay reveal + add, cover hide + add, wait call.
        &SplashPlan::ImageCycles { last_load, cycle_end, reveal, hide, .. } => {
            let mut cycle: Vec<Instruction> = logo[last_load - 1..=last_load].to_vec();
            cycle.extend_from_slice(&logo[reveal - 1..=reveal]);
            cycle.extend_from_slice(&logo[hide - 1..=hide]);
            cycle.extend_from_slice(&logo[cycle_end - 1..=cycle_end]);
            for copied in cycle.iter_mut() {
                if copied.opcode == OP_SET_TEXTURE {
                    copied.arguments[0].raw_data = splash_id;
                }
            }
            (cycle_end + 1, cycle)
        }
        // Single cycle: afterwards R sits at 0 and H at 255, so the new cycle reveals
        // with H and hides with R (swapped → no fade-channel clash), then waits.
        &SplashPlan::SingleCycle { texture_slot, reveal, reveal_object, hide, hide_object, wait } => {
            let load = instruction(OP_SET_TEXTURE, "set-texture", vec![arg(ARG_IMMEDIATE, splash_id), arg(ARG_IMMEDIATE, texture_slot), arg(ARG_IMMEDIATE, u32::MAX)]);
            let mut reveal_pair = logo[reveal - 1..=reveal].to_vec();
            reveal_pair[0].arguments[2].raw_data = hide_object;
            let mut hide_pair = logo[hide - 1..=hide].to_vec();
            hide_pair[0].arguments[2].raw_data = reveal_object;
            let mut cycle = vec![load];
            cycle.extend(reveal_pair);
            cycle.extend(hide_pair);
            cycle.extend_from_slice(&logo[wait - 1..=wait]);
            (wait + 1, cycle)
        }
        SplashPlan::CrossfadeCycles { copy, load, insert_at, .. } => {
            let cycle = copy
                .iter()
                .map(|&k| {
                    let mut copied = logo[k].clone();
                    if k == *load {
                        copied.arguments[0] = arg(ARG_IMMEDIATE, splash_id);
                    }
                    copied
                })
                .collect();
            (*insert_at, cycle)
        }
        SplashPlan::Unsupported { reason } => return Err(reason.clone()),
    };
    for copied in cycle.iter_mut() {
        copied.offset = u64::MAX; // new instructions; copied calls keep their targets (writer remaps)
    }
    let mut out = logo.to_vec();
    out.splice(insert_at..insert_at, cycle);
    Ok(out)
}
