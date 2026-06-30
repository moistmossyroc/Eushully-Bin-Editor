use crate::bin_format::instruction::Instruction;
use crate::project::types::CharacterEntry;
use std::collections::BTreeMap;

/// Extract all character definitions from a BIN file (typically CVINIT.BIN).
///
/// Parses the structural pattern of CVINIT character blocks:
/// Each block contains a group of consecutive set-string instructions (name, role, VA)
/// followed by a series of mov instructions, one of which sets the character's sequential ID.
///
/// The character ID is identified by finding a mov to the ID variable register — a global-int
/// address that receives sequential values (1, 2, 3...) across blocks. This ID is what scene
/// scripts use to reference characters via lookup-array indirection.
///
/// Returns CharacterEntry with pointer = character ID and name = first string in the group.
pub fn extract_all_character_definitions(instructions: &[Instruction]) -> Vec<CharacterEntry> {
    // First, detect the ID variable register by finding a global-int (type 0x3) destination
    // that receives sequential values 1, 2, 3... across multiple mov instructions.
    // We look for mov instructions whose destinations form an arithmetic sequence in raw_data
    // AND whose source values form a sequential 1, 2, 3... pattern.
    let id_register_base = detect_cvinit_id_register(instructions);

    // Collect all set-string instructions with their indices and data
    let set_strings: Vec<(usize, u32, String)> = instructions
        .iter()
        .enumerate()
        .filter(|(_, instr)| instr.opcode == 0x192 && instr.arguments.len() >= 2)
        .filter_map(|(i, instr)| {
            let dest_addr = instr.arguments[0].raw_data;
            let name = instr.arguments[1].decoded_string.clone().unwrap_or_default();
            if !name.is_empty() {
                Some((i, dest_addr, name))
            } else {
                None
            }
        })
        .collect();

    if set_strings.is_empty() {
        return Vec::new();
    }

    // Detect the base string address and stride by looking at the first few destinations.
    // CVINIT string addresses follow a pattern like 0x1B11, 0x1B13, 0x1B14, 0x1B16, 0x1B17...
    // The "name" slots are the first in each group. We identify groups by finding set-strings
    // whose destination addresses form the start of new 3-slot or 2-slot groups.
    //
    // Strategy: the name string is always the first set-string written for each character block.
    // After it, there may be 1-2 more set-strings for role/VA. Then mov instructions follow.
    // So we group by finding set-strings that come after a gap (non-set-string instructions).

    let mut result: BTreeMap<u32, String> = BTreeMap::new();

    // Group set-strings into blocks: a new block starts when there's a gap of non-set-string
    // instructions between consecutive set-strings (i.e., instruction indices aren't consecutive)
    let mut block_start = 0;
    while block_start < set_strings.len() {
        let mut block_end = block_start;
        // Extend block while instruction indices are consecutive
        while block_end + 1 < set_strings.len()
            && set_strings[block_end + 1].0 == set_strings[block_end].0 + 1
        {
            block_end += 1;
        }

        // First set-string in block is the character name
        let (_instr_idx, str_addr, ref char_name) = set_strings[block_start];
        let last_set_string_idx = set_strings[block_end].0;

        // Scan forward from the last set-string to find the ID mov
        let mut char_id: Option<u32> = None;
        if let Some(id_base) = id_register_base {
            let scan_end = (last_set_string_idx + 12).min(instructions.len());
            for j in (last_set_string_idx + 1)..scan_end {
                let instr = &instructions[j];
                if instr.opcode == 0x55 && instr.arguments.len() >= 2 {
                    let dest = &instr.arguments[0];
                    let src = &instr.arguments[1];
                    if dest.arg_type == 3 && src.arg_type == 0 && dest.raw_data >= id_base {
                        let offset = dest.raw_data - id_base;
                        if offset < 200 && src.raw_data > 0 {
                            char_id = Some(src.raw_data);
                            break;
                        }
                    }
                } else if instr.opcode == 0x192 {
                    break; // Next set-string group
                }
            }
        }

        let pointer = char_id.unwrap_or(str_addr);
        result.entry(pointer).or_insert_with(|| char_name.clone());

        block_start = block_end + 1;
    }

    result
        .into_iter()
        .map(|(pointer, name)| CharacterEntry { pointer, name })
        .collect()
}

/// Extract character names from CIINIT.BIN / CNINIT.BIN.
///
/// These init files store names via set-string into a contiguous stride-1 run
/// of string addresses, but their **call numbers are NOT positional** — the
/// iteration order in the file can scramble around (e.g. a loop iterating
/// over IDs 1..22, 24..29, 23, 36..37, 34..35, 30, 31).
///
/// The real call number for each entry is encoded as the literal in the
/// `mov type=3 <- type=0 literal` instructions that follow the set-string (the
/// engine loops `for id in list: dst[pos++] = names[id]` and writes `id` into
/// the loop-index register right after each set-string).
///
/// Algorithm:
/// 1. Collect every set-string destination with its (instruction_index, name).
/// 2. Find the longest stride-1 address run whose values are mostly unique
///    (≥80%) — filters out affiliation/profile tables.
/// 3. For each set-string in the run, scan forward to the next set-string
///    (exclusive) for the dominant small-literal mov value (< 200); that's
///    the call number. Fall back to positional index on miss.
pub fn extract_ciinit_character_names(instructions: &[Instruction]) -> Vec<CharacterEntry> {
    use std::collections::{HashMap, HashSet};

    // Collect set-strings as (instruction_index, addr, name).
    let set_strings: Vec<(usize, u32, String)> = instructions
        .iter()
        .enumerate()
        .filter_map(|(i, instr)| {
            if instr.opcode == 0x192 && instr.arguments.len() >= 2 {
                let addr = instr.arguments[0].raw_data;
                let name = instr.arguments[1].decoded_string.clone().unwrap_or_default();
                if !name.is_empty() {
                    return Some((i, addr, name));
                }
            }
            None
        })
        .collect();
    if set_strings.is_empty() {
        return Vec::new();
    }

    // For the run-detection pass we need addr -> last_name.
    let mut addr_to_name: HashMap<u32, String> = HashMap::new();
    for (_, addr, name) in &set_strings {
        addr_to_name.insert(*addr, name.clone());
    }
    let mut addrs: Vec<u32> = addr_to_name.keys().copied().collect();
    addrs.sort_unstable();

    let qualifies = |run: &[u32]| -> bool {
        if run.len() < 5 {
            return false;
        }
        let uniq: HashSet<&str> = run
            .iter()
            .filter_map(|a| addr_to_name.get(a).map(String::as_str))
            .collect();
        uniq.len() * 5 >= run.len() * 4
    };

    let mut best: Vec<u32> = Vec::new();
    let mut current: Vec<u32> = vec![addrs[0]];
    for &a in &addrs[1..] {
        if *current.last().unwrap() + 1 == a {
            current.push(a);
        } else {
            if qualifies(&current) && current.len() > best.len() {
                best = current.clone();
            }
            current = vec![a];
        }
    }
    if qualifies(&current) && current.len() > best.len() {
        best = current;
    }
    if best.is_empty() {
        return Vec::new();
    }
    let run_addrs: HashSet<u32> = best.iter().copied().collect();

    // Filter set_strings to those in the main run, in file order.
    let run_entries: Vec<(usize, u32, String)> = set_strings
        .iter()
        .filter(|(_, addr, _)| run_addrs.contains(addr))
        .cloned()
        .collect();

    // For each set-string in the run, find the dominant small literal in the
    // window ending at the next set-string in the run.
    let mut result: Vec<CharacterEntry> = Vec::new();
    let mut seen_ids: HashSet<u32> = HashSet::new();
    let base = best[0];
    for (k, (instr_idx, addr, name)) in run_entries.iter().enumerate() {
        let next_idx = run_entries
            .get(k + 1)
            .map(|(ni, _, _)| *ni)
            .unwrap_or(instructions.len());
        let mut counts: HashMap<u32, usize> = HashMap::new();
        for j in (*instr_idx + 1)..next_idx {
            let ins = &instructions[j];
            if ins.opcode == 0x55 && ins.arguments.len() >= 2 {
                let dest = &ins.arguments[0];
                let src = &ins.arguments[1];
                if dest.arg_type == 3 && src.arg_type == 0 && src.raw_data > 0 && src.raw_data < 200 {
                    *counts.entry(src.raw_data).or_insert(0) += 1;
                }
            }
        }
        // Pick the literal with the highest count; break ties by smallest value.
        let call_number = counts
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(&v, _)| v)
            .unwrap_or(addr - base + 1);

        // Avoid duplicate call numbers — if we somehow match the same ID twice,
        // fall back to positional.
        let pointer = if seen_ids.insert(call_number) {
            call_number
        } else {
            addr - base + 1
        };
        result.push(CharacterEntry {
            pointer,
            name: name.clone(),
        });
    }
    result.sort_by_key(|c| c.pointer);
    result
}

/// Detect the CVINIT ID register base address.
///
/// Looks for a global-int variable that receives value 1, where consecutive
/// register addresses receive incrementing values. The pattern is:
/// register[base+0] = 1, register[base+1] = 2, ..., possibly with gaps in values
/// but the register addresses are always consecutive.
fn detect_cvinit_id_register(instructions: &[Instruction]) -> Option<u32> {
    use std::collections::HashMap;

    // Collect all mov(global-int, literal) instructions: dest_addr -> value
    let mut mov_targets: HashMap<u32, u32> = HashMap::new();
    for instr in instructions {
        if instr.opcode == 0x55 && instr.arguments.len() >= 2 {
            let dest = &instr.arguments[0];
            let src = &instr.arguments[1];
            if dest.arg_type == 3 && src.arg_type == 0 {
                mov_targets.insert(dest.raw_data, src.raw_data);
            }
        }
    }

    // Find a destination that receives value 1
    // Then count how many consecutive register addresses also receive values
    let mut candidates: Vec<(u32, usize)> = Vec::new();

    for (&addr, &val) in &mov_targets {
        if val == 1 {
            let mut chain_len = 1;
            for offset in 1u32..200 {
                if mov_targets.contains_key(&(addr + offset)) {
                    chain_len += 1;
                } else {
                    break;
                }
            }
            if chain_len >= 5 {
                candidates.push((addr, chain_len));
            }
        }
    }

    // Pick the candidate with the longest chain
    candidates.sort_by_key(|(_, len)| std::cmp::Reverse(*len));
    candidates.first().map(|(addr, _)| *addr)
}

/// Detect the speaker register for a BIN file.
///
/// Looks for `set-string` instructions that write to an address which is also
/// referenced by show-text arguments. The most commonly referenced address
/// that's both a set-string target and a show-text argument is the speaker register.
pub fn detect_speaker_register(instructions: &[Instruction]) -> Option<u32> {
    use std::collections::HashMap;

    // Count how many times each address is the target of set-string
    let mut set_string_targets: HashMap<u32, usize> = HashMap::new();
    for instr in instructions {
        if instr.opcode == 0x192 && instr.arguments.len() >= 2 {
            *set_string_targets.entry(instr.arguments[0].raw_data).or_insert(0) += 1;
        }
    }

    // Find addresses referenced by show-text arguments (any non-literal, non-string arg type)
    // This covers global string vars (type 5), local string vars (type 0xB/0xE), etc.
    let mut show_text_refs: HashMap<u32, usize> = HashMap::new();
    for instr in instructions {
        if instr.opcode == 0x6E {
            for arg in &instr.arguments {
                // Skip literal (type 0) and inline string (type 2) — those aren't register refs
                if arg.arg_type != 0 && arg.arg_type != 2 {
                    *show_text_refs.entry(arg.raw_data).or_insert(0) += 1;
                }
            }
        }
    }

    // The speaker register is a set-string target that's also used by show-text
    // Pick the one with the most show-text references
    show_text_refs
        .iter()
        .filter(|(addr, _)| set_string_targets.contains_key(addr))
        .max_by_key(|(_, count)| *count)
        .map(|(addr, _)| *addr)
}

/// For each show-text instruction, determine the current speaker by tracking
/// the character ID variable — the global-int that gets set via `mov` before
/// each dialogue beat and feeds into the engine's speaker-setup function.
///
/// The character ID variable is the same one identified by `extract_referenced_speakers`:
/// the index variable used by lookup-array/lookup-array-2d in the speaker resolution chain.
///
/// ID 0 = narrator (no speaker), which is excluded from results.
///
/// Returns a map from instruction index to SpeakerRef.
pub fn extract_row_speakers(
    instructions: &[Instruction],
    speaker_register: u32,
) -> BTreeMap<usize, SpeakerRef> {
    use std::collections::HashSet;

    // Identify the character ID variable(s) by tracing the speaker assignment chain
    let mut id_variables: HashSet<(u32, u32)> = HashSet::new();
    for (i, instr) in instructions.iter().enumerate() {
        if instr.opcode == 0x192 && instr.arguments.len() >= 2 {
            if instr.arguments[0].raw_data == speaker_register {
                let src = &instr.arguments[1];
                if src.arg_type != 2 {
                    let search_start = i.saturating_sub(10);
                    for j in (search_start..i).rev() {
                        let prev = &instructions[j];
                        if prev.opcode == 0x61 && prev.arguments.len() >= 3 {
                            let idx_arg = &prev.arguments[2];
                            if idx_arg.arg_type != 0 {
                                id_variables.insert((idx_arg.arg_type, idx_arg.raw_data));
                            }
                            break;
                        }
                        if prev.opcode == 0x12C && prev.arguments.len() >= 5 {
                            let row_arg = &prev.arguments[2];
                            if row_arg.arg_type != 0 {
                                id_variables.insert((row_arg.arg_type, row_arg.raw_data));
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    let mut result = BTreeMap::new();
    let mut current_speaker_id: Option<u32> = None;
    // Track literal-string speaker assignments (set-string with arg_type 2)
    let mut current_literal_speaker: Option<(u32, String)> = None;

    for (i, instr) in instructions.iter().enumerate() {
        // Track mov writes to the character ID variable
        if instr.opcode == 0x55 && instr.arguments.len() >= 2 {
            let dest = &instr.arguments[0];
            let src = &instr.arguments[1];
            let key = (dest.arg_type, dest.raw_data);
            if id_variables.contains(&key) && src.arg_type == 0 {
                current_speaker_id = Some(src.raw_data);
                current_literal_speaker = None; // ID-based takes precedence
            }
        }

        // Track set-string writes to speaker register (both literal and variable sources)
        if instr.opcode == 0x192 && instr.arguments.len() >= 2 {
            if instr.arguments[0].raw_data == speaker_register {
                let src = &instr.arguments[1];
                if src.arg_type == 2 {
                    // Literal string speaker — record directly
                    let name = src.decoded_string.clone().unwrap_or_default();
                    current_literal_speaker = Some((src.raw_data, name));
                    current_speaker_id = None; // Literal takes over
                }
            }
        }

        // For show-text, record the current speaker
        if instr.opcode == 0x6E && !instr.arguments.is_empty() {
            if let Some(char_id) = current_speaker_id {
                // ID 0 = narrator, skip
                if char_id > 0 {
                    result.insert(i, SpeakerRef {
                        source_type: 0,
                        source_raw_data: char_id,
                        literal_name: None,
                    });
                }
            } else if let Some((raw_data, ref name)) = current_literal_speaker {
                result.insert(i, SpeakerRef {
                    source_type: 2, // literal string source
                    source_raw_data: raw_data,
                    literal_name: if name.is_empty() { None } else { Some(name.clone()) },
                });
            }
        }
    }

    result
}

/// Extract all unique character/speaker references from a BIN file.
///
/// Traces the full speaker assignment chain:
/// 1. Detect the speaker register (global string var written by set-string and read by show-text)
/// 2. Find set-string writes to speaker register — if source is a literal string (type 2),
///    record it directly. If source is a temp register, trace backward.
/// 3. For temp register sources, find the preceding lookup-array (0x61) or lookup-array-2d (0x12C)
///    that writes to that temp register to identify the index variable.
/// 4. Scan all mov (0x55) instructions for literal values assigned to the index variable.
/// 5. Those literal values are the character indices/pointers.
///
/// Returns CharacterEntry for each unique character reference found.
pub fn extract_referenced_speakers(instructions: &[Instruction]) -> Vec<CharacterEntry> {
    use std::collections::HashSet;

    let speaker_reg = match detect_speaker_register(instructions) {
        Some(reg) => reg,
        None => return Vec::new(),
    };

    let mut found: BTreeMap<u32, String> = BTreeMap::new();
    // Store (arg_type, raw_data) to avoid false positives across different variable types
    let mut index_variables: HashSet<(u32, u32)> = HashSet::new();

    // Pass 1: Find set-string writes to speaker register and trace sources
    for (i, instr) in instructions.iter().enumerate() {
        if instr.opcode == 0x192 && instr.arguments.len() >= 2 {
            let dest = &instr.arguments[0];
            if dest.raw_data == speaker_reg {
                let src = &instr.arguments[1];

                if src.arg_type == 2 {
                    // Literal string source — record directly with the name
                    if !found.contains_key(&src.raw_data) {
                        let name = src.decoded_string.clone().unwrap_or_default();
                        found.insert(src.raw_data, name);
                    }
                } else {
                    // Variable/temp register source — trace backward to find lookup-array
                    let search_start = i.saturating_sub(10);
                    for j in (search_start..i).rev() {
                        let prev = &instructions[j];
                        // lookup-array: dest = array[index] — 3 args
                        if prev.opcode == 0x61 && prev.arguments.len() >= 3 {
                            let idx_arg = &prev.arguments[2];
                            if idx_arg.arg_type != 0 {
                                index_variables.insert((idx_arg.arg_type, idx_arg.raw_data));
                            }
                            break;
                        }
                        // lookup-array-2d: dest = array[row][col] — 5 args
                        if prev.opcode == 0x12C && prev.arguments.len() >= 5 {
                            // Capture both row (arg[2]) and col (arg[4]) index variables
                            let row_arg = &prev.arguments[2];
                            if row_arg.arg_type != 0 {
                                index_variables.insert((row_arg.arg_type, row_arg.raw_data));
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    // Pass 2: Find all literal values assigned to the index variables via mov (0x55)
    if !index_variables.is_empty() {
        for instr in instructions {
            if instr.opcode == 0x55 && instr.arguments.len() >= 2 {
                let dest = &instr.arguments[0];
                let src = &instr.arguments[1];
                // dest must match type+raw_data of an index variable, src must be literal (type 0)
                if index_variables.contains(&(dest.arg_type, dest.raw_data)) && src.arg_type == 0 {
                    if !found.contains_key(&src.raw_data) {
                        found.insert(src.raw_data, String::new());
                    }
                }
            }
        }
    }

    found
        .into_iter()
        .map(|(pointer, name)| CharacterEntry { pointer, name })
        .collect()
}

/// Detect the character lookup-array for SYS5 BIN files.
///
/// SYS5 scripts set the speaker before each dialogue beat using a pattern:
///   lookup-array (dest) (g-int ARRAY) (g-int INDEX)
///   mov (dest) (lit CHAR_ID)            ← character ID override
///   lookup-array (dest) (g-int ARRAY2) (g-int INDEX)
///   mov (dest) (lit 0)
///   mov (g-int CS_VAR) (lit CS_IMAGE)   ← CS image reference
///
/// The character ID is the literal in the first `mov` after the first lookup-array
/// in each block. The array source (ARRAY) is consistent across all speaker blocks.
///
/// Detection: find lookup-array instructions near show-text where the next instruction
/// is `mov (same_dest) (literal)`. The array source with the highest hit count and
/// fewest unique character IDs is the speaker table.
///
/// Returns (array_addr, index_addr) — the global-int addresses of the array and index.
pub fn detect_speaker_lookup_array(instructions: &[Instruction]) -> Option<(u32, u32)> {
    use std::collections::{HashMap, HashSet};

    let show_text_set: HashSet<usize> = instructions.iter().enumerate()
        .filter(|(_, i)| i.opcode == 0x6E)
        .map(|(idx, _)| idx)
        .collect();

    if show_text_set.is_empty() {
        return None;
    }

    // Track (array_addr, index_addr) pairs from lookup-array near show-text
    // where the next instruction is mov with a literal to the same dest
    // Key: (array_addr, index_addr) → (unique char IDs, total hits)
    let mut candidates: HashMap<(u32, u32), (HashSet<u32>, usize)> = HashMap::new();

    for (i, instr) in instructions.iter().enumerate() {
        if instr.opcode != 0x61 || instr.arguments.len() < 3 {
            continue;
        }
        let array_arg = &instr.arguments[1];
        let index_arg = &instr.arguments[2];
        // Both array and index must be global-int (type 0x3)
        if array_arg.arg_type != 3 || index_arg.arg_type != 3 {
            continue;
        }
        // Next instruction must be mov to same dest with literal source
        if i + 1 >= instructions.len() {
            continue;
        }
        let next = &instructions[i + 1];
        if next.opcode != 0x55 || next.arguments.len() < 2 {
            continue;
        }
        let dest = &next.arguments[0];
        let src = &next.arguments[1];
        // dest should match the lookup-array dest, src must be literal
        if dest.arg_type != instr.arguments[0].arg_type
            || dest.raw_data != instr.arguments[0].raw_data
            || src.arg_type != 0
        {
            continue;
        }

        let key = (array_arg.raw_data, index_arg.raw_data);
        let entry = candidates.entry(key).or_insert_with(|| (HashSet::new(), 0));
        entry.0.insert(src.raw_data);
        entry.1 += 1;
    }

    // The speaker table has high hit count with few unique character IDs
    // Score by reuse ratio (hits / unique_ids)
    candidates.into_iter()
        .filter(|(_, (ids, hits))| ids.len() >= 2 && *hits >= 10)
        .max_by(|(_, (ids_a, hits_a)), (_, (ids_b, hits_b))| {
            let ratio_a = *hits_a as f64 / ids_a.len() as f64;
            let ratio_b = *hits_b as f64 / ids_b.len() as f64;
            ratio_a.partial_cmp(&ratio_b).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(key, _)| key)
}

/// Legacy detection: find a global-int variable set to small literal values near show-text.
/// Kept as fallback for SYS5 files that may use a simpler pattern.
pub fn detect_speaker_id_variable(instructions: &[Instruction]) -> Option<u32> {
    use std::collections::{HashMap, HashSet};

    let show_text_set: HashSet<usize> = instructions.iter().enumerate()
        .filter(|(_, i)| i.opcode == 0x6E)
        .map(|(idx, _)| idx)
        .collect();

    if show_text_set.is_empty() {
        return None;
    }

    let mut dest_info: HashMap<u32, (HashSet<u32>, usize)> = HashMap::new();

    for (i, instr) in instructions.iter().enumerate() {
        if instr.opcode == 0x55 && instr.arguments.len() >= 2 {
            let dest = &instr.arguments[0];
            let src = &instr.arguments[1];
            if dest.arg_type == 0x3 && src.arg_type == 0 && src.raw_data <= 1000 {
                let near_show = (1..=25).any(|offset| show_text_set.contains(&(i + offset)));
                let entry = dest_info.entry(dest.raw_data).or_insert_with(|| (HashSet::new(), 0));
                entry.0.insert(src.raw_data);
                if near_show {
                    entry.1 += 1;
                }
            }
        }
    }

    dest_info.into_iter()
        .filter(|(_, (unique_vals, near_count))| unique_vals.len() >= 5 && *near_count >= 10)
        .max_by(|(_, (uv_a, nc_a)), (_, (uv_b, nc_b))| {
            let ratio_a = *nc_a as f64 / uv_a.len() as f64;
            let ratio_b = *nc_b as f64 / uv_b.len() as f64;
            let score_a = ratio_a * (*nc_a as f64).sqrt();
            let score_b = ratio_b * (*nc_b as f64).sqrt();
            score_a.partial_cmp(&score_b).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(addr, _)| addr)
}

/// Extract row speakers for SYS5 using the lookup-array character ID pattern.
///
/// SYS5 pattern before each dialogue beat:
///   lookup-array (dest) (g-int ARRAY) (g-int INDEX)
///   mov (dest) (lit CHAR_ID)
///
/// Tracks the character ID from the `mov` after each speaker lookup-array,
/// then maps each show-text to the most recently seen non-zero character ID.
pub fn extract_row_speakers_sys5(
    instructions: &[Instruction],
    array_addr: u32,
) -> BTreeMap<usize, SpeakerRef> {
    let mut result = BTreeMap::new();
    let mut current_char_id: Option<u32> = None;

    for (i, instr) in instructions.iter().enumerate() {
        // Detect speaker assignment: lookup-array using the character table
        // followed by mov with a literal character ID
        if instr.opcode == 0x61 && instr.arguments.len() >= 3 {
            let arr = &instr.arguments[1];
            if arr.arg_type == 3 && arr.raw_data == array_addr {
                if i + 1 < instructions.len() {
                    let next = &instructions[i + 1];
                    if next.opcode == 0x55 && next.arguments.len() >= 2 {
                        let dest = &next.arguments[0];
                        let src = &next.arguments[1];
                        if dest.arg_type == instr.arguments[0].arg_type
                            && dest.raw_data == instr.arguments[0].raw_data
                            && src.arg_type == 0
                        {
                            current_char_id = if src.raw_data > 0 { Some(src.raw_data) } else { None };
                        }
                    }
                }
            }
        }

        // Map show-text to current speaker
        if instr.opcode == 0x6E && !instr.arguments.is_empty() {
            if let Some(id) = current_char_id {
                result.insert(i, SpeakerRef {
                    source_type: 0,
                    source_raw_data: id,
                    literal_name: None,
                });
            }
        }
    }

    result
}

/// Extract all unique character references for SYS5 using the lookup-array pattern.
///
/// Collects all unique non-zero literal character IDs from the `mov` instructions
/// that immediately follow lookup-array instructions using the character table.
pub fn extract_referenced_speakers_sys5(
    instructions: &[Instruction],
    array_addr: u32,
) -> Vec<CharacterEntry> {
    use std::collections::BTreeSet;

    let mut ids = BTreeSet::new();

    for (i, instr) in instructions.iter().enumerate() {
        if instr.opcode == 0x61 && instr.arguments.len() >= 3 {
            let arr = &instr.arguments[1];
            if arr.arg_type == 3 && arr.raw_data == array_addr {
                if i + 1 < instructions.len() {
                    let next = &instructions[i + 1];
                    if next.opcode == 0x55 && next.arguments.len() >= 2 {
                        let dest = &next.arguments[0];
                        let src = &next.arguments[1];
                        if dest.arg_type == instr.arguments[0].arg_type
                            && dest.raw_data == instr.arguments[0].raw_data
                            && src.arg_type == 0
                            && src.raw_data > 0
                        {
                            ids.insert(src.raw_data);
                        }
                    }
                }
            }
        }
    }

    ids.into_iter()
        .map(|id| CharacterEntry { pointer: id, name: String::new() })
        .collect()
}

/// Legacy: Extract row speakers using a simple global-int ID variable pattern.
/// Kept as fallback for SYS5 files that may not use lookup-array.
pub fn extract_row_speakers_sys5_legacy(
    instructions: &[Instruction],
    id_variable: u32,
) -> BTreeMap<usize, SpeakerRef> {
    let mut result = BTreeMap::new();

    for (i, instr) in instructions.iter().enumerate() {
        if instr.opcode != 0x6E || instr.arguments.is_empty() {
            continue;
        }
        let search_start = i.saturating_sub(50);
        for j in (search_start..i).rev() {
            let prev = &instructions[j];
            if prev.opcode == 0x55 && prev.arguments.len() >= 2 {
                let dest = &prev.arguments[0];
                let src = &prev.arguments[1];
                if dest.arg_type == 0x3 && dest.raw_data == id_variable
                    && src.arg_type == 0 && src.raw_data > 0
                {
                    result.insert(i, SpeakerRef {
                        source_type: 0,
                        source_raw_data: src.raw_data,
                        literal_name: None,
                    });
                    break;
                }
            }
        }
    }

    result
}

/// Legacy: Extract unique character IDs from a simple global-int ID variable.
pub fn extract_referenced_speakers_sys5_legacy(
    instructions: &[Instruction],
    id_variable: u32,
) -> Vec<CharacterEntry> {
    use std::collections::BTreeSet;

    let mut ids = BTreeSet::new();

    for instr in instructions {
        if instr.opcode == 0x55 && instr.arguments.len() >= 2 {
            let dest = &instr.arguments[0];
            let src = &instr.arguments[1];
            if dest.arg_type == 0x3 && dest.raw_data == id_variable && src.arg_type == 0 && src.raw_data > 0 {
                ids.insert(src.raw_data);
            }
        }
    }

    ids.into_iter()
        .map(|id| CharacterEntry { pointer: id, name: String::new() })
        .collect()
}

/// Detect the speaker display-slot for SYS4 BINs that set the speaker via the
/// `set-display-slot` opcode (`0x1D2`) instead of a set-string register — e.g.
/// Megami Zero, where each dialogue beat is preceded by `0x1D2 [lit SLOT, lit ID]`
/// and SLOT 2 carries the character ID (resolved to a name via CNINIT/CIINIT).
///
/// Generic detection (no per-game constant): among `0x1D2` ops whose slot and
/// value are both literals, pick the slot that identifies a speaker (non-zero
/// current value) for the MOST show-text beats and has ≥2 distinct IDs. The name
/// slot is set before nearly every line, so it dominates portrait/colour slots.
pub fn detect_speaker_display_slot(instructions: &[Instruction]) -> Option<u32> {
    use std::collections::HashSet;

    let mut slots: HashSet<u32> = HashSet::new();
    for ins in instructions {
        if ins.opcode == 0x1D2
            && ins.arguments.len() >= 2
            && ins.arguments[0].arg_type == 0
            && ins.arguments[1].arg_type == 0
        {
            slots.insert(ins.arguments[0].raw_data);
        }
    }

    let mut best: Option<(u32, usize)> = None;
    for &slot in &slots {
        let mut cur: u32 = 0;
        let mut hits = 0usize;
        let mut distinct: HashSet<u32> = HashSet::new();
        for ins in instructions {
            if ins.opcode == 0x1D2
                && ins.arguments.len() >= 2
                && ins.arguments[0].arg_type == 0
                && ins.arguments[0].raw_data == slot
                && ins.arguments[1].arg_type == 0
            {
                cur = ins.arguments[1].raw_data;
                if cur > 0 {
                    distinct.insert(cur);
                }
            } else if ins.opcode == 0x6E && cur > 0 {
                hits += 1;
            }
        }
        if distinct.len() >= 2 && hits >= 5 && best.map_or(true, |(_, h)| hits > h) {
            best = Some((slot, hits));
        }
    }
    best.map(|(s, _)| s)
}

/// Per-show-text speaker for the `0x1D2` display-slot pattern. Tracks the current
/// literal value written to `slot`; each show-text inherits the most recent
/// non-zero value as its character ID (0 = narrator/cleared → no badge).
pub fn extract_row_speakers_1d2(
    instructions: &[Instruction],
    slot: u32,
) -> BTreeMap<usize, SpeakerRef> {
    let mut result = BTreeMap::new();
    let mut cur: Option<u32> = None;
    for (i, ins) in instructions.iter().enumerate() {
        if ins.opcode == 0x1D2
            && ins.arguments.len() >= 2
            && ins.arguments[0].arg_type == 0
            && ins.arguments[0].raw_data == slot
            && ins.arguments[1].arg_type == 0
        {
            let v = ins.arguments[1].raw_data;
            cur = if v > 0 { Some(v) } else { None };
        }
        if ins.opcode == 0x6E && !ins.arguments.is_empty() {
            if let Some(id) = cur {
                result.insert(i, SpeakerRef { source_type: 0, source_raw_data: id, literal_name: None });
            }
        }
    }
    result
}

/// Unique character IDs referenced via the `0x1D2` display-slot pattern. Names are
/// resolved downstream against the init-BIN id→name maps (same as SYS5 IDs).
pub fn extract_referenced_speakers_1d2(
    instructions: &[Instruction],
    slot: u32,
) -> Vec<CharacterEntry> {
    use std::collections::BTreeSet;
    let mut ids = BTreeSet::new();
    for ins in instructions {
        if ins.opcode == 0x1D2
            && ins.arguments.len() >= 2
            && ins.arguments[0].arg_type == 0
            && ins.arguments[0].raw_data == slot
            && ins.arguments[1].arg_type == 0
            && ins.arguments[1].raw_data > 0
        {
            ids.insert(ins.arguments[1].raw_data);
        }
    }
    ids.into_iter().map(|id| CharacterEntry { pointer: id, name: String::new() }).collect()
}

/// Check if a SYS3 show-text instruction is a speaker name.
///
/// Returns true if the text looks like a name: short, no sentence punctuation,
/// not preceded by display-furigana, and followed by end-text-line.
fn is_sys3_speaker_name(instructions: &[Instruction], i: usize, text: &str) -> bool {
    // Must not start with dialogue quotes
    if text.starts_with('「') || text.starts_with('（') {
        return false;
    }
    // Must be followed by end-text-line
    if i + 1 >= instructions.len() || instructions[i + 1].opcode != 0x6F {
        return false;
    }
    // Must not be preceded by display-furigana (0x196) — those are continuation fragments
    if i > 0 && instructions[i - 1].opcode == 0x196 {
        return false;
    }
    let char_count = text.chars().count();
    char_count <= 20
        && !text.contains('。')
        && !text.contains('、')
        && !text.contains('」')
        && !text.contains('「')
        && !text.contains('…')
        && !text.contains('？')
        && !text.contains('！')
        && !text.contains('～')
        && !text.starts_with('　')
}

/// Stable u32 ID from a speaker name string (FNV-1a hash).
fn speaker_name_hash(name: &str) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for byte in name.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    // Ensure non-zero (0 means narrator/no speaker)
    if hash == 0 { 1 } else { hash }
}

/// Extract row speakers for SYS3 using inline literal name pattern.
///
/// SYS3 embeds speaker names directly as show-text instructions with literal strings.
/// Pattern: show-text "Name" → end-text-line → [play-voice] → show-text "「dialogue」"
///
/// Uses stable hash IDs so the speaker badge looks up the Characters panel,
/// allowing users to type translated names that appear in the badge.
pub fn extract_row_speakers_sys3(
    instructions: &[Instruction],
) -> BTreeMap<usize, SpeakerRef> {
    let mut result = BTreeMap::new();
    let mut current_speaker_id: Option<u32> = None;

    for (i, instr) in instructions.iter().enumerate() {
        if instr.opcode != 0x6E || instr.arguments.is_empty() {
            continue;
        }

        let text = match instr.arguments.iter().find_map(|a| a.decoded_string.as_ref()) {
            Some(t) => t,
            None => continue,
        };

        // A show-text that starts with 「 is dialogue — attach the current speaker
        if text.starts_with('「') || text.starts_with('（') {
            if let Some(id) = current_speaker_id {
                result.insert(i, SpeakerRef {
                    source_type: 0,
                    source_raw_data: id,
                    literal_name: None,
                });
            }
            continue;
        }

        if is_sys3_speaker_name(instructions, i, text) {
            current_speaker_id = Some(speaker_name_hash(text));
        }
    }

    result
}

/// Extract unique speaker names from SYS3 inline show-text patterns.
///
/// Collects all distinct speaker names detected by the SYS3 pattern.
/// Returns CharacterEntry where pointer is a stable hash of the Japanese name
/// and name is the original Japanese text (user can overwrite with translation).
pub fn extract_referenced_speakers_sys3(
    instructions: &[Instruction],
) -> Vec<CharacterEntry> {
    let mut seen: BTreeMap<u32, String> = BTreeMap::new();

    for (i, instr) in instructions.iter().enumerate() {
        if instr.opcode != 0x6E || instr.arguments.is_empty() {
            continue;
        }

        let text = match instr.arguments.iter().find_map(|a| a.decoded_string.as_ref()) {
            Some(t) => t,
            None => continue,
        };

        if is_sys3_speaker_name(instructions, i, text) {
            let id = speaker_name_hash(text);
            seen.entry(id).or_insert_with(|| text.clone());
        }
    }

    seen.into_iter()
        .map(|(id, name)| CharacterEntry { pointer: id, name })
        .collect()
}

/// Reference to the speaker for a specific show-text instruction.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpeakerRef {
    /// Argument type of the source (2 = literal string, 5 = global var, 0xE = temp, etc.)
    pub source_type: u32,
    /// Raw data value of the source argument
    pub source_raw_data: u32,
    /// If the source was a literal string (type 2), the actual name
    pub literal_name: Option<String>,
}

/// Scan `instructions` for every `set-string [dest] = "value"` opcode and
/// return a map of `dest_pointer → name` for entries in the requested set.
///
/// Only non-empty string values are recorded. First write wins (later writes
/// to the same pointer are ignored — typical CVINIT pattern where the name
/// is the first string assigned to each character slot).
///
/// Used to resolve unknown character names by directly looking up known
/// speaker-slot pointers across init BINs and the scene BIN itself, instead
/// of relying on the structural CVINIT block-pattern heuristic.
pub fn scan_setstring_writes_for_pointers(
    instructions: &[Instruction],
    wanted: &std::collections::HashSet<u32>,
) -> std::collections::HashMap<u32, String> {
    let mut out = std::collections::HashMap::new();
    for instr in instructions {
        if instr.opcode != 0x192 || instr.arguments.len() < 2 {
            continue;
        }
        let dest = instr.arguments[0].raw_data;
        if !wanted.contains(&dest) {
            continue;
        }
        if out.contains_key(&dest) {
            continue; // first non-empty write wins
        }
        if let Some(ref name) = instr.arguments[1].decoded_string {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                out.insert(dest, trimmed.to_string());
            }
        }
    }
    out
}

/// Build a `character_id → name` map for CHINIT-style init BINs.
///
/// CHINIT initializes its character table by assigning each character's ID to
/// a UNIQUE consecutive g-int register, immediately followed by the matching
/// name as a set-string. Empirically (this title's CHINIT.BIN):
///
/// ```text
/// mov g-int:0x22ABA1, lit:0x61                       (each char gets its own reg)
/// set-string g-str:0x5E4, str:"ユン・ガソル兵"        (next instruction)
/// mov g-int:0x22ABA2, lit:0x62
/// set-string g-str:0x5EA, str:"アルフィミアの私兵"
/// ...
/// ```
///
/// The literal value in the `mov` *is* the scene-facing character ID — same
/// number `extract_referenced_speakers` Pass 2 pulls out of the scene. The
/// register address varies per character; only the literal value matters.
///
/// Detection (no per-game tuning needed):
/// 1. For every `mov g-int:R <- lit:V` whose next 1-2 instructions include a
///    `set-string` with a non-empty string, record `V → name`.
/// 2. If we collect ≥10 such pairs the BIN almost certainly contains a real
///    character table; return the map. Below that threshold there's not enough
///    signal — return empty rather than risk false positives.
///
/// Scene-side IDs (e.g. `0x61` from `extract_referenced_speakers`) get looked
/// up in this map to resolve names that no other static analysis can find —
/// the scene-local indirection through engine subroutines means there's no
/// direct address path; this is the only static signal that works.
pub fn extract_chinit_id_name_map(instructions: &[Instruction]) -> std::collections::HashMap<u32, String> {
    use std::collections::HashMap;

    /// Window of immediately-following instructions to scan for a set-string.
    /// In practice the set-string is at offset +1; +2 occasionally if another
    /// mov for a sibling field intervenes. Larger windows risk leaking into the
    /// next character's slot.
    const LOOKAHEAD: usize = 2;

    fn next_set_string_name(instructions: &[Instruction], start: usize, lookahead: usize) -> Option<String> {
        let end = (start + lookahead + 1).min(instructions.len());
        for j in (start + 1)..end {
            let ins = &instructions[j];
            if ins.opcode == 0x192 && ins.arguments.len() >= 2 {
                if let Some(s) = &ins.arguments[1].decoded_string {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
        None
    }

    // Collect ALL (register, value, name) tuples where the mov is followed
    // by a set-string. CHINIT has multiple unrelated mov-then-setstring
    // patterns: the canonical ID-register tables hold a unique character ID
    // per consecutive register slot, while OTHER tables hold a constant flag
    // (e.g. always `lit:1`) across many slots. We need to identify which
    // register clusters are the real ID tables.
    let mut tuples: Vec<(u32, u32, String)> = Vec::new();
    for (i, instr) in instructions.iter().enumerate() {
        if instr.opcode != 0x55 || instr.arguments.len() < 2 {
            continue;
        }
        let dest = &instr.arguments[0];
        let src = &instr.arguments[1];
        if dest.arg_type != 3 || src.arg_type != 0 {
            continue;
        }
        if src.raw_data == 0 || src.raw_data > 0xFFFF {
            continue;
        }
        if let Some(name) = next_set_string_name(instructions, i, LOOKAHEAD) {
            tuples.push((dest.raw_data, src.raw_data, name));
        }
    }

    // Cluster by register proximity. A real ID-register bank is contiguous
    // (stride 1-2). Gaps > MAX_R_GAP almost always cross into an unrelated
    // table (mov to a far-away register address).
    tuples.sort_by_key(|(r, _, _)| *r);
    const MAX_R_GAP: u32 = 4;
    let mut clusters: Vec<Vec<(u32, u32, String)>> = Vec::new();
    let mut current: Vec<(u32, u32, String)> = Vec::new();
    for t in tuples {
        if let Some(prev) = current.last() {
            if t.0 - prev.0 > MAX_R_GAP {
                clusters.push(std::mem::take(&mut current));
            }
        }
        current.push(t);
    }
    if !current.is_empty() {
        clusters.push(current);
    }

    // Pre-compute "flag-ness" for each V value: how many distinct R addresses
    // write it. Flag-like values (e.g. `lit:1` used as an init flag) appear
    // with many R; real character IDs typically appear with 1 (each character
    // gets its own register slot). Used by the loose pass below to keep
    // isolated entries while still filtering out flag noise.
    let mut r_per_v: HashMap<u32, std::collections::HashSet<u32>> = HashMap::new();
    {
        // Re-walk the (sorted) tuples currently held by `clusters`.
        for cluster in &clusters {
            for (r, v, _) in cluster {
                r_per_v.entry(*v).or_default().insert(*r);
            }
        }
    }

    // Pass 1 — cluster-based extraction. Catches the big register banks in
    // base CHINIT files: contiguous R range, mostly-unique V values. The
    // distinctness check filters out constant-flag clusters (always V=1).
    const MIN_DISTINCT_V: usize = 5;
    let mut map: HashMap<u32, String> = HashMap::new();
    let mut accepted_clusters: Vec<&Vec<(u32, u32, String)>> = Vec::new();
    for cluster in &clusters {
        let distinct: std::collections::HashSet<u32> = cluster.iter().map(|(_, v, _)| *v).collect();
        if distinct.len() < MIN_DISTINCT_V {
            continue;
        }
        // Distinctness: ≥80% of slots have a unique V (loose enough that one
        // accidental dup doesn't kill the cluster).
        if distinct.len() * 5 < cluster.len() * 4 {
            continue;
        }
        accepted_clusters.push(cluster);
        for (_, v, name) in cluster {
            map.entry(*v).or_insert_with(|| name.clone());
        }
    }

    // Pass 2 — loose extraction for sparse character tables. Append/DLC
    // CHINIT files (`$1$CHINIT.BIN` etc.) typically introduce a handful of
    // new characters scattered across non-consecutive registers, so they
    // never form a qualifying cluster. For any tuple NOT in an accepted
    // cluster, include its (V, name) if V is "rare" — set by ≤MAX_RPV
    // distinct registers, meaning it looks like a unique character ID not
    // a flag. The first-wins map preserves cluster-pass entries.
    const MAX_RPV_FOR_LOOSE: usize = 3;
    let in_accepted: std::collections::HashSet<*const Vec<(u32, u32, String)>> =
        accepted_clusters.iter().map(|c| *c as *const _).collect();
    for cluster in &clusters {
        if in_accepted.contains(&(cluster as *const _)) {
            continue;
        }
        for (_, v, name) in cluster {
            let rpv = r_per_v.get(v).map(|s| s.len()).unwrap_or(0);
            if rpv > MAX_RPV_FOR_LOOSE {
                continue; // looks like a flag, skip
            }
            map.entry(*v).or_insert_with(|| name.clone());
        }
    }

    // Confidence threshold: lowered from 10 → 2 so small append CHINIT
    // files (which may legitimately introduce only a few new characters)
    // still produce a usable map. Above-2 still rules out files where the
    // mov-then-setstring pattern is purely coincidental.
    if map.len() < 2 {
        return HashMap::new();
    }
    map
}

#[cfg(test)]
mod tests {
    // Convention: tests prefixed `diagnose_` are interactive inspection helpers
    // (raw byte dumps, codepoint frequencies, register heatmaps) used during
    // BIN-format reverse-engineering sessions. They're `#[ignore]`'d so plain
    // `cargo test` stays fast and noise-free. Run them with:
    //     cargo test -- --ignored                                (run all)
    //     cargo test diagnose_chinit -- --ignored --nocapture    (one by name)
    // Real correctness tests use plain `#[test]` and assert!.
    use super::*;
    use crate::bin_format::parser::parse_bin;
    use std::path::Path;

    fn load_test_bin(name: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let paths = vec![
            base.join(name),
            base.join("sys4").join(name),
            base.join("sys5").join(name),
            base.parent().unwrap().join(name),
            base.parent().unwrap().join("sys4").join(name),
            base.parent().unwrap().join("sys5").join(name),
        ];
        for path in paths {
            if let Ok(data) = std::fs::read(&path) {
                return Ok(data);
            }
        }
        Err("File not found".into())
    }

    #[test]
    fn test_sys5_detection_functions() {
        let data = match load_test_bin("SC0000.BIN") {
            Ok(d) => d,
            Err(_) => { println!("SC0000.BIN not found"); return; }
        };
        let parsed = parse_bin(&data).unwrap();
        // A different SC0000.BIN (e.g. the SYS4 Sankai file) may sit on the
        // search path — only the SYS5 detection assertions apply to a SYS5 file.
        if parsed.header.sys_version != 5 {
            println!("SC0000.BIN found is SYS{} not SYS5 — skipping", parsed.header.sys_version);
            return;
        }

        // New lookup-array based detection
        let lookup = detect_speaker_lookup_array(&parsed.instructions);
        assert!(lookup.is_some(), "Should detect SYS5 speaker lookup-array");
        let (array_addr, _index_addr) = lookup.unwrap();

        let speakers = extract_referenced_speakers_sys5(&parsed.instructions, array_addr);
        assert!(speakers.len() >= 5, "Should find at least 5 speaker IDs, got {}", speakers.len());
        println!("SYS5 speakers: {:?}", speakers.iter().map(|s| s.pointer).collect::<Vec<_>>());

        let row_speakers = extract_row_speakers_sys5(&parsed.instructions, array_addr);
        assert!(row_speakers.len() >= 50, "Should have at least 50 row speaker entries, got {}", row_speakers.len());
        println!("SYS5 row speaker entries: {}", row_speakers.len());
    }

    #[test]
    #[ignore = "diagnostic — run via: cargo test -- --ignored"]
    fn diagnose_ciinit_call_numbers() {
        let data = load_test_bin("sys4/CIINIT.BIN").unwrap();
        let parsed = parse_bin(&data).unwrap();
        // For each set-string in the main name run, dump movs strictly after it
        // up to the next set-string, with literals < 200.
        let name_range: std::collections::HashSet<u32> =
            (0x11C1Cu32..=0x11C3Eu32).collect();

        let set_string_indices: Vec<(usize, u32, String)> = parsed
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(i, instr)| {
                if instr.opcode == 0x192 && instr.arguments.len() >= 2 {
                    let addr = instr.arguments[0].raw_data;
                    let name = instr.arguments[1].decoded_string.clone().unwrap_or_default();
                    if !name.is_empty() && name_range.contains(&addr) {
                        return Some((i, addr, name));
                    }
                }
                None
            })
            .collect();

        for (k, (i, addr, name)) in set_string_indices.iter().enumerate() {
            let next_i = set_string_indices
                .get(k + 1)
                .map(|(ni, _, _)| *ni)
                .unwrap_or(parsed.instructions.len());
            let mut after_lits: Vec<u32> = Vec::new();
            for j in (*i + 1)..next_i {
                let ins = &parsed.instructions[j];
                if ins.opcode == 0x55 && ins.arguments.len() >= 2 {
                    let src = &ins.arguments[1];
                    if src.arg_type == 0 && src.raw_data < 200 {
                        after_lits.push(src.raw_data);
                    }
                }
            }
            println!("  0x{:X} pos={} {:30} after={:?}", addr, k, format!("{:?}", name), after_lits);
        }
    }

    #[test]
    #[ignore = "diagnostic — run via: cargo test -- --ignored"]
    fn diagnose_ciinit_cvinit_style() {
        let data = match load_test_bin("sys4/CIINIT.BIN") {
            Ok(d) => d,
            Err(_) => { println!("skipping"); return; }
        };
        let parsed = parse_bin(&data).unwrap();
        let chars = extract_all_character_definitions(&parsed.instructions);
        println!("CVINIT-style on CIINIT: {} entries", chars.len());
        for c in &chars {
            println!("  id={} name={:?}", c.pointer, c.name);
        }
    }

    #[test]
    #[ignore = "diagnostic — run via: cargo test -- --ignored"]
    fn diagnose_ciinit_mov_pattern() {
        let data = match load_test_bin("sys4/CIINIT.BIN") {
            Ok(d) => d,
            Err(_) => { println!("skipping"); return; }
        };
        let parsed = parse_bin(&data).unwrap();

        // Dump instructions around each name set-string (addresses 0x11C1C..=0x11C3E).
        // Looking for mov(global-int, literal) patterns that might encode game IDs.
        let name_addrs: std::collections::HashSet<u32> =
            (0x11C1Cu32..=0x11C3Eu32).collect();

        for (i, instr) in parsed.instructions.iter().enumerate() {
            if instr.opcode == 0x192
                && instr.arguments.len() >= 2
                && name_addrs.contains(&instr.arguments[0].raw_data)
            {
                let addr = instr.arguments[0].raw_data;
                let name = instr.arguments[1].decoded_string.clone().unwrap_or_default();
                println!("[{:4}] set-string 0x{:X} = {:?}", i, addr, name);
                // Dump the 6 preceding and 3 following instructions
                let start = i.saturating_sub(6);
                let end = (i + 4).min(parsed.instructions.len());
                for j in start..end {
                    if j == i { continue; }
                    let ins = &parsed.instructions[j];
                    if ins.opcode == 0x55 && ins.arguments.len() >= 2 {
                        let dest = &ins.arguments[0];
                        let src = &ins.arguments[1];
                        println!("  [{:4}] mov type={}/0x{:X} <- type={}/0x{:X}",
                            j, dest.arg_type, dest.raw_data, src.arg_type, src.raw_data);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "diagnostic — run via: cargo test -- --ignored"]
    fn diagnose_sys4_cninit_and_scene() {
        use std::collections::HashMap;
        // 1. CNINIT.BIN structure
        if let Ok(data) = load_test_bin("sys4/CNINIT.BIN") {
            let parsed = parse_bin(&data).unwrap();
            println!("CNINIT.BIN: {} instructions", parsed.instructions.len());
            let mut addr_to_name: HashMap<u32, String> = HashMap::new();
            for instr in &parsed.instructions {
                if instr.opcode == 0x192 && instr.arguments.len() >= 2 {
                    let addr = instr.arguments[0].raw_data;
                    let name = instr.arguments[1].decoded_string.clone().unwrap_or_default();
                    if !name.is_empty() {
                        addr_to_name.insert(addr, name);
                    }
                }
            }
            let mut addrs: Vec<u32> = addr_to_name.keys().copied().collect();
            addrs.sort_unstable();
            println!("  unique set-string addresses: {}", addrs.len());
            for a in &addrs {
                println!("    0x{:X} = {:?}", a, addr_to_name.get(a));
            }
            let ci_chars = extract_ciinit_character_names(&parsed.instructions);
            println!("  ciinit-style extraction: {} entries", ci_chars.len());
            for c in ci_chars.iter().take(15) {
                println!("    id={} name={:?}", c.pointer, c.name);
            }
        }

        // 2. SC0000.BIN: find referenced speaker IDs
        if let Ok(data) = load_test_bin("sys4/SC0000.BIN") {
            let parsed = parse_bin(&data).unwrap();
            let speakers = extract_referenced_speakers(&parsed.instructions);
            let mut ids: Vec<u32> = speakers.iter().map(|s| s.pointer).collect();
            ids.sort_unstable();
            ids.dedup();
            println!("SC0000.BIN referenced speaker IDs ({}): {:?}", ids.len(), ids);
            println!("  max: {}", ids.iter().max().copied().unwrap_or(0));
        }
    }

    #[test]
    #[ignore = "diagnostic — run via: cargo test -- --ignored"]
    fn diagnose_ciinit_all_runs() {
        use std::collections::HashMap;
        let data = match load_test_bin("sys4/CIINIT.BIN") {
            Ok(d) => d,
            Err(_) => { println!("sys4/CIINIT.BIN not found, skipping"); return; }
        };
        let parsed = parse_bin(&data).unwrap();

        let mut addr_to_name: HashMap<u32, String> = HashMap::new();
        for instr in &parsed.instructions {
            if instr.opcode == 0x192 && instr.arguments.len() >= 2 {
                let addr = instr.arguments[0].raw_data;
                let name = instr.arguments[1].decoded_string.clone().unwrap_or_default();
                if !name.is_empty() {
                    addr_to_name.insert(addr, name);
                }
            }
        }
        let mut addrs: Vec<u32> = addr_to_name.keys().copied().collect();
        addrs.sort_unstable();
        println!("unique set-string addresses: {}", addrs.len());

        // Collect all runs of length ≥3
        let mut runs: Vec<Vec<u32>> = Vec::new();
        let mut current: Vec<u32> = vec![addrs[0]];
        for &a in &addrs[1..] {
            if *current.last().unwrap() + 1 == a {
                current.push(a);
            } else {
                if current.len() >= 3 { runs.push(current.clone()); }
                current = vec![a];
            }
        }
        if current.len() >= 3 { runs.push(current); }

        println!("{} runs of length >=3", runs.len());
        for run in &runs {
            let start = run[0];
            let end = *run.last().unwrap();
            println!("  run 0x{:X}..=0x{:X} (len={})", start, end, run.len());
            for (i, addr) in run.iter().enumerate().take(3) {
                let _ = i;
                println!("    0x{:X} = {:?}", addr, addr_to_name.get(addr));
            }
            if run.len() > 3 {
                let last = run.last().unwrap();
                println!("    ...");
                println!("    0x{:X} = {:?}", last, addr_to_name.get(last));
            }
        }
    }

    #[test]
    #[ignore = "diagnostic — run via: cargo test -- --ignored"]
    fn diagnose_ciinit_run_detection() {
        let data = match load_test_bin("sys4/CIINIT.BIN") {
            Ok(d) => d,
            Err(_) => { println!("sys4/CIINIT.BIN not found, skipping"); return; }
        };
        let parsed = parse_bin(&data).unwrap();
        let chars = extract_ciinit_character_names(&parsed.instructions);
        println!("extract_ciinit_character_names: {} entries", chars.len());
        for c in chars.iter().take(15) {
            println!("  id={} name={:?}", c.pointer, c.name);
        }
        println!("  ...");
        for c in chars.iter().rev().take(5).rev() {
            println!("  id={} name={:?}", c.pointer, c.name);
        }
    }

    #[test]
    #[ignore = "diagnostic — run via: cargo test -- --ignored"]
    fn diagnose_ciinit_structure() {
        let data = match load_test_bin("sys4/CIINIT.BIN") {
            Ok(d) => d,
            Err(_) => { println!("sys4/CIINIT.BIN not found, skipping"); return; }
        };
        let parsed = parse_bin(&data).unwrap();
        println!("CIINIT.BIN: {} instructions", parsed.instructions.len());

        // Dump first 40 set-string instructions
        let mut count = 0;
        for (i, instr) in parsed.instructions.iter().enumerate() {
            if instr.opcode == 0x192 && instr.arguments.len() >= 2 {
                let dest = instr.arguments[0].raw_data;
                let name = instr.arguments[1].decoded_string.clone().unwrap_or_default();
                println!("  [{:5}] set-string @ 0x{:04X} = {:?}", i, dest, name);
                count += 1;
                if count >= 40 { break; }
            }
        }
        let total_ss = parsed.instructions.iter().filter(|i| i.opcode == 0x192).count();
        println!("  total set-string instructions: {}", total_ss);

        // Try extraction
        let chars = extract_all_character_definitions(&parsed.instructions);
        println!("  extract_all_character_definitions: {} entries", chars.len());
        for c in chars.iter().take(20) {
            println!("    pointer={} name={:?}", c.pointer, c.name);
        }

        // Try CVINIT for comparison
        if let Ok(cv_data) = load_test_bin("sys4/CVINIT.BIN") {
            let cv_parsed = parse_bin(&cv_data).unwrap();
            let cv_chars = extract_all_character_definitions(&cv_parsed.instructions);
            println!("CVINIT.BIN: {} character entries", cv_chars.len());
            for c in cv_chars.iter().take(10) {
                println!("    pointer={} name={:?}", c.pointer, c.name);
            }
            let cv_max = cv_chars.iter().map(|c| c.pointer).max().unwrap_or(0);
            println!("  CVINIT max pointer: {}", cv_max);
        }
    }

    #[test]
    fn test_chinit_id_name_map_resolves_yun_gasol_id() {
        let data = match load_test_bin("sys4/CHINIT.BIN") {
            Ok(d) => d,
            Err(_) => { println!("sys4/CHINIT.BIN not found, skipping"); return; }
        };
        let parsed = parse_bin(&data).unwrap();
        let map = extract_chinit_id_name_map(&parsed.instructions);
        println!("CHINIT ID → name map: {} entries", map.len());
        let mut keys: Vec<u32> = map.keys().copied().collect();
        keys.sort();
        for k in keys.iter().take(20) {
            println!("  id=0x{:X} ({}) = {:?}", k, k, map.get(k));
        }
        // ID 0x61 (97) should resolve to "ユン・ガソル兵" per user reverse-engineering.
        if let Some(name) = map.get(&0x61) {
            println!("\n0x61 -> {:?}", name);
            assert!(
                name.contains("ユン・ガソル") || name.contains("兵"),
                "Expected ユン・ガソル兵 family for ID 0x61, got {:?}", name
            );
        } else {
            panic!("ID 0x61 was not resolved by CHINIT ID-name map");
        }
        // Sanity: known main-cast IDs should also resolve.
        assert_eq!(map.get(&1).map(String::as_str), Some("ヴァイスハイト"));
        assert_eq!(map.get(&2).map(String::as_str), Some("リセル"));
    }

    #[test]
    fn test_extract_characters_from_cvinit() {
        let data = match load_test_bin("CVINIT.BIN") {
            Ok(d) => d,
            Err(_) => {
                println!("CVINIT.BIN not found, skipping");
                return;
            }
        };
        let parsed = parse_bin(&data).unwrap();
        let chars = extract_all_character_definitions(&parsed.instructions);

        assert!(
            chars.len() > 30,
            "CVINIT should have many character definitions, got {}",
            chars.len()
        );

        // All should have names
        for c in &chars {
            assert!(!c.name.is_empty(), "Character 0x{:X} has empty name", c.pointer);
        }
    }

    #[test]
    fn test_detect_speaker_register() {
        for file_name in &["SC1310.BIN", "SC2230.BIN"] {
            let data = match load_test_bin(file_name) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let parsed = parse_bin(&data).unwrap();
            let reg = detect_speaker_register(&parsed.instructions);
            assert!(reg.is_some(), "{} should have a speaker register", file_name);
        }
    }

    #[test]
    fn test_extract_referenced_speakers_finds_indices() {
        for file_name in &["SC0000.BIN", "SC1310.BIN"] {
            let data = match load_test_bin(file_name) {
                Ok(d) => d,
                Err(_) => { println!("{} SKIPPED", file_name); continue; }
            };
            let parsed = parse_bin(&data).unwrap();

            // Try SYS4-style first, then SYS5 lookup-array, then legacy fallback
            let speakers = extract_referenced_speakers(&parsed.instructions);
            if speakers.len() > 2 {
                println!("{}: found {} speaker refs (SYS4)", file_name, speakers.len());
            } else if let Some((array_addr, _)) = detect_speaker_lookup_array(&parsed.instructions) {
                let sys5_speakers = extract_referenced_speakers_sys5(&parsed.instructions, array_addr);
                println!("{}: found {} speaker refs (SYS5 lookup-array)", file_name, sys5_speakers.len());
                assert!(sys5_speakers.len() > 2, "{} should have more than 2 speaker refs, got {}", file_name, sys5_speakers.len());
            } else if let Some(id_var) = detect_speaker_id_variable(&parsed.instructions) {
                let sys5_speakers = extract_referenced_speakers_sys5_legacy(&parsed.instructions, id_var);
                println!("{}: found {} speaker refs (SYS5 legacy)", file_name, sys5_speakers.len());
                assert!(sys5_speakers.len() > 2, "{} should have more than 2 speaker refs, got {}", file_name, sys5_speakers.len());
            } else {
                panic!("{} should have more than 2 speaker refs, got {}", file_name, speakers.len());
            }
        }
    }

    #[test]
    fn test_row_speakers_all_resolved() {
        let data = match load_test_bin("SC1310.BIN") {
            Ok(d) => d,
            Err(_) => return,
        };
        let parsed = parse_bin(&data).unwrap();
        let speaker_reg = detect_speaker_register(&parsed.instructions).unwrap();
        let row_speakers = extract_row_speakers(&parsed.instructions, speaker_reg);

        // Every show-text should have a speaker entry (narrator excluded with ID 0)
        assert!(
            row_speakers.len() > 0,
            "Should have at least some speaker entries"
        );
        // All source_raw_data should be valid character IDs (> 0)
        for (idx, speaker) in &row_speakers {
            assert!(speaker.source_raw_data > 0,
                "Speaker at [{}] has ID 0 (should be excluded)", idx);
        }
    }
    #[test]
    fn test_sys3_speaker_detection() {
        let data = load_test_bin("sys3/SC060.BIN").unwrap();
        let parsed = parse_bin(&data).unwrap();

        let speakers = extract_referenced_speakers_sys3(&parsed.instructions);
        assert!(speakers.len() >= 3, "Should find at least 3 speakers, got {}", speakers.len());
        // Should contain real names, not furigana continuation fragments
        let names: Vec<&str> = speakers.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"ティリ"), "Should find ティリ");
        assert!(names.contains(&"イグナート"), "Should find イグナート");
        // Furigana continuations should be filtered out
        assert!(!names.contains(&"も"), "Should not detect furigana continuation 'も'");
        assert!(!names.contains(&"しの間"), "Should not detect furigana continuation 'しの間'");

        let row_speakers = extract_row_speakers_sys3(&parsed.instructions);
        assert!(row_speakers.len() >= 20, "Should have many speaker-tagged rows, got {}", row_speakers.len());
    }

}
