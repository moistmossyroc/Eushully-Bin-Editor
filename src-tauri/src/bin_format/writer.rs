use crate::bin_format::header::Header;
use crate::bin_format::instruction::Instruction;
use crate::bin_format::opcodes::{is_control_flow, is_label_argument};
use encoding_rs::SHIFT_JIS;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WriteError {
    #[error("Instruction at index {0} has no definition")]
    MissingDefinition(usize),
    #[error("Label target 0x{target:X} not found in offset map")]
    LabelNotFound { target: u32 },
}

/// Reassemble a list of instructions back into a valid BIN file.
///
/// This is a full rewrite — all offsets (strings, arrays, labels, footer tables)
/// are recomputed from scratch. Matches the C++ reassembler's byte-exact behavior.
pub fn write_bin(header: &Header, instructions: &[Instruction]) -> Result<Vec<u8>, WriteError> {
    let header_len = header.length as u64;
    let is_ver5 = header.is_ver5;

    // Phase 1: Compute byte offsets for each instruction
    let mut instruction_byte_offsets: Vec<u64> = Vec::with_capacity(instructions.len());
    let mut current_offset = header_len;

    for instr in instructions {
        instruction_byte_offsets.push(current_offset);
        current_offset += instr.byte_size();
    }

    let data_array_end = current_offset;

    // Phase 2: Build old-offset -> new-offset map for label fixup.
    // Skip instructions with offset u64::MAX (sentinel for inserted entries
    // that have no original offset — they aren't referenced by any label
    // and would collide with each other / corrupt the map).
    let mut old_to_new_offset: std::collections::HashMap<u64, u64> =
        std::collections::HashMap::new();
    for (i, instr) in instructions.iter().enumerate() {
        if instr.offset == u64::MAX {
            continue;
        }
        old_to_new_offset.insert(instr.offset, instruction_byte_offsets[i]);
    }

    // Phase 3: Write string data — matches C++ reassembler exactly.
    // The C++ tracks current_string_offset as an absolute file offset,
    // and padding is computed from that offset for 4-byte alignment.
    let mut string_data: Vec<u8> = Vec::with_capacity(4096);
    let mut string_offsets: Vec<(usize, usize, u32)> = Vec::new();
    let mut current_string_offset = data_array_end;

    for (instr_idx, instr) in instructions.iter().enumerate() {
        for (arg_idx, arg) in instr.arguments.iter().enumerate() {
            if arg.arg_type != 2 {
                continue;
            }
            if arg.decoded_string.is_none() {
                continue;
            }

            // Record the offset for this string argument (in u32 units)
            let raw_offset = ((current_string_offset - header_len) >> 2) as u32;
            string_offsets.push((instr_idx, arg_idx, raw_offset));

            // If we have original encoded bytes (unmodified string), XOR them back
            // and use the exact original length for perfect round-trip.
            if let Some(ref native_bytes) = arg.original_encoded_bytes {
                let str_len = native_bytes.len();

                if is_ver5 {
                    // XOR each byte with 0xFF
                    for &b in native_bytes {
                        string_data.push(b ^ 0xFF);
                    }
                    // UTF-16: length is in chars, native_bytes.len() is chars*2
                    let char_len = str_len / 2;
                    current_string_offset += ((char_len + 1) * 2) as u64;
                    let padding = 4 - (current_string_offset % 4) as usize;
                    for _ in 0..padding + 2 {
                        string_data.push(0xFF);
                    }
                    current_string_offset += padding as u64;
                } else {
                    // XOR each byte with 0xFF
                    for &b in native_bytes {
                        string_data.push(b ^ 0xFF);
                    }
                    current_string_offset += (str_len + 1) as u64;
                    let padding = 4 - (current_string_offset % 4) as usize;
                    for _ in 0..padding + 1 {
                        string_data.push(0xFF);
                    }
                    current_string_offset += padding as u64;
                }
            } else if is_ver5 {
                // Normalize smart/curly quotes → ASCII (see strings.rs) so they
                // don't encode to wide full-width glyphs in-game. Only MODIFIED
                // strings reach here (unchanged reuse original_encoded_bytes above).
                let text = crate::bin_format::strings::normalize_export_punctuation(
                    arg.decoded_string.as_ref().unwrap(),
                );
                // SYS5: UTF-16LE encoding, each byte XOR'd with 0xFF
                let utf16_chars: Vec<u16> = text.encode_utf16().collect();
                let str_len = utf16_chars.len();

                // Write XOR'd UTF-16LE bytes
                for &ch in &utf16_chars {
                    let bytes = ch.to_le_bytes();
                    string_data.push(bytes[0] ^ 0xFF);
                    string_data.push(bytes[1] ^ 0xFF);
                }

                // C++ logic: current_string_offset += (length + 1) * 2
                current_string_offset += ((str_len + 1) * 2) as u64;

                // C++ logic: padding = 4 - (current_string_offset % 4), write padding + 2 bytes of 0xFF
                let padding = 4 - (current_string_offset % 4) as usize;
                for _ in 0..padding + 2 {
                    string_data.push(0xFF);
                }
                current_string_offset += padding as u64;
            } else {
                // Normalize smart/curly quotes → ASCII so they don't encode to
                // wide full-width SJIS glyphs (e.g. "Celica's" → "Celica' s").
                let text = crate::bin_format::strings::normalize_export_punctuation(
                    arg.decoded_string.as_ref().unwrap(),
                );
                // SYS4: SJIS encoding, each byte XOR'd with 0xFF
                let (encoded, _, _) = SHIFT_JIS.encode(&text);
                let sjis_bytes = encoded.as_ref();
                let str_len = sjis_bytes.len();

                // Write XOR'd SJIS bytes
                for &byte in sjis_bytes {
                    string_data.push(byte ^ 0xFF);
                }

                // C++ logic: current_string_offset += length + 1
                current_string_offset += (str_len + 1) as u64;

                // C++ logic: padding = 4 - (current_string_offset % 4), write padding + 1 bytes of 0xFF
                let padding = 4 - (current_string_offset % 4) as usize;
                for _ in 0..padding + 1 {
                    string_data.push(0xFF);
                }
                current_string_offset += padding as u64;
            }
        }
    }

    // Phase 4: Collect array arguments and compute footer arrays
    let mut footer_data: Vec<u32> = Vec::with_capacity(1024);
    let mut array_offsets: Vec<(usize, usize, u32)> = Vec::new();

    let mut current_array_offset = ((current_string_offset - header_len) >> 2) as u32;
    for (instr_idx, instr) in instructions.iter().enumerate() {
        if instr.opcode == 0x64 {
            for (arg_idx, arg) in instr.arguments.iter().enumerate() {
                if let Some(ref da) = arg.data_array {
                    array_offsets.push((instr_idx, arg_idx, current_array_offset));
                    footer_data.push(da.length);
                    footer_data.extend(&da.data);
                    current_array_offset += da.length + 1;
                }
            }
        }
    }

    // Phase 5: Build the three footer index tables
    let mut table_1: Vec<u32> = Vec::new();
    let mut table_2: Vec<u32> = Vec::new();
    let mut table_3: Vec<u32> = Vec::new();

    for (i, instr) in instructions.iter().enumerate() {
        let offset_in_u32_units = ((instruction_byte_offsets[i] - header_len) >> 2) as u32;
        match instr.opcode {
            0x71 => table_1.push(offset_in_u32_units),
            0x03 => table_2.push(offset_in_u32_units),
            0x8F => table_3.push(offset_in_u32_units),
            _ => {}
        }
    }

    table_1.sort();
    table_2.sort();
    table_3.sort();

    // Phase 6: Build the updated header
    let mut out_header = header.clone();
    out_header.binary.table_1_length = table_1.len() as u32;
    out_header.binary.table_1_offset = current_array_offset;
    out_header.binary.table_2_length = table_2.len() as u32;
    out_header.binary.table_2_offset = current_array_offset + table_1.len() as u32;
    out_header.binary.table_3_length = table_3.len() as u32;
    out_header.binary.table_3_offset =
        current_array_offset + table_1.len() as u32 + table_2.len() as u32;

    // Phase 7: Write the output
    let estimated_size = header_len as usize
        + instructions
            .iter()
            .map(|i| i.byte_size() as usize)
            .sum::<usize>()
        + string_data.len()
        + (footer_data.len() + table_1.len() + table_2.len() + table_3.len()) * 4;
    let mut output = Vec::with_capacity(estimated_size);

    // Write header
    output.extend(out_header.to_bytes());

    // Write instructions
    for (instr_idx, instr) in instructions.iter().enumerate() {
        output.extend_from_slice(&instr.opcode.to_le_bytes());

        for (arg_idx, arg) in instr.arguments.iter().enumerate() {
            output.extend_from_slice(&arg.arg_type.to_le_bytes());

            let raw_value = if arg.arg_type == 2 {
                // String: use the computed string offset
                string_offsets
                    .iter()
                    .find(|(ii, ai, _)| *ii == instr_idx && *ai == arg_idx)
                    .map(|(_, _, offset)| *offset)
                    .unwrap_or(arg.raw_data)
            } else if instr.opcode == 0x64 && arg.data_array.is_some() {
                // Array: use the computed array offset
                array_offsets
                    .iter()
                    .find(|(ii, ai, _)| *ii == instr_idx && *ai == arg_idx)
                    .map(|(_, _, offset)| *offset)
                    .unwrap_or(arg.raw_data)
            } else if is_control_flow(instr.opcode)
                && is_label_argument(instr.opcode, arg_idx, arg.raw_data)
            {
                // Label: convert old offset to new offset
                if let Some(&new_byte_offset) = old_to_new_offset.get(&(arg.raw_data as u64)) {
                    ((new_byte_offset - header_len) >> 2) as u32
                } else {
                    arg.raw_data
                }
            } else {
                arg.raw_data
            };

            output.extend_from_slice(&raw_value.to_le_bytes());
        }
    }

    // Write string data
    output.extend(&string_data);

    // Write footer arrays
    for val in &footer_data {
        output.extend_from_slice(&val.to_le_bytes());
    }

    // Write the three index tables
    for val in &table_1 {
        output.extend_from_slice(&val.to_le_bytes());
    }
    for val in &table_2 {
        output.extend_from_slice(&val.to_le_bytes());
    }
    for val in &table_3 {
        output.extend_from_slice(&val.to_le_bytes());
    }

    // Patch the header in the output with final table offsets
    let header_bytes = out_header.to_bytes();
    output[..header_bytes.len()].copy_from_slice(&header_bytes);

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bin_format::parser::parse_bin;
    use std::path::Path;

    /// Load a real game BIN sitting next to the project, or `None` if absent.
    /// Game-data tests skip cleanly when missing so `cargo test` is green on
    /// clean clones / CI (the engine-specific coverage lives in the synthetic
    /// `tests/engine_roundtrip.rs`, which needs no game data).
    fn load_test_bin(name: &str) -> Option<Vec<u8>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(name);
        std::fs::read(&path).ok()
    }

    #[test]
    fn find_bracket_strings_in_bin() {
        for name in &["SC1310.BIN", "CVINIT.BIN", "CIINIT.BIN"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(name);
            let Ok(data) = std::fs::read(&path) else { continue };
            let Ok(parsed) = parse_bin(&data) else { continue };
            println!("\n=== {} ===", name);
            let mut count = 0;
            for (i, instr) in parsed.instructions.iter().enumerate() {
                for (ai, arg) in instr.arguments.iter().enumerate() {
                    if let Some(ref s) = arg.decoded_string {
                        if s.contains('【') || s.contains('】') {
                            println!("[{}] op=0x{:X} arg[{}] = {:?}", i, instr.opcode, ai, s);
                            count += 1;
                            if count >= 10 { return; }
                        }
                    }
                }
            }
            println!("(no 【】 strings found in {})", name);
        }
    }

    #[test]
    fn dump_show_text_arg0_distribution() {
        let Some(data) = load_test_bin("SC1310.BIN") else { return };
        let parsed = parse_bin(&data).unwrap();
        let mut counts: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
        for i in &parsed.instructions {
            if i.opcode == 0x6E && !i.arguments.is_empty() {
                *counts.entry(i.arguments[0].raw_data).or_insert(0) += 1;
            }
        }
        for (k, v) in &counts {
            println!("show-text arg[0]=0x{:X}: {} instances", k, v);
        }
    }

    #[test]
    fn dump_context_around_show_text_sys4() {
        use crate::bin_format::opcodes::definition_for_opcode;
        let Some(data) = load_test_bin("SC1310.BIN") else { return };
        let parsed = parse_bin(&data).unwrap();
        // Find first show-text and dump 10 instructions before it
        let st_idx = parsed.instructions.iter().position(|i| i.opcode == 0x6E && !i.arguments.is_empty() && i.arguments.iter().any(|a| a.decoded_string.is_some())).unwrap();
        let start = st_idx.saturating_sub(15);
        for (i, instr) in parsed.instructions[start..=st_idx].iter().enumerate() {
            let label = definition_for_opcode(instr.opcode).map(|d| d.label).unwrap_or("?");
            println!("[{}] op=0x{:X} ({}) args={}",
                start + i, instr.opcode, label,
                instr.arguments.iter().map(|a| format!("({} 0x{:X}{})", a.arg_type, a.raw_data,
                    if let Some(ref s) = a.decoded_string { format!(" \"{}\"", s) } else { String::new() }
                )).collect::<Vec<_>>().join(" ")
            );
        }
    }

    #[test]
    fn dump_show_text_args_sys4() {
        let Some(data) = load_test_bin("SC1310.BIN") else { return };
        let parsed = parse_bin(&data).unwrap();
        let shows: Vec<_> = parsed.instructions.iter().filter(|i| i.opcode == 0x6E).take(5).collect();
        for (i, st) in shows.iter().enumerate() {
            println!("show-text #{}: arg_count={}", i, st.argument_count);
            for (ai, a) in st.arguments.iter().enumerate() {
                println!("  arg[{}]: type={} raw_data=0x{:X} string={:?}",
                    ai, a.arg_type, a.raw_data, a.decoded_string);
            }
        }
    }

    /// Insert a cloned bridge mid-stream and verify the exported BIN re-parses
    /// cleanly. Regression test for the bug where apply.rs Phase 4 overwrote
    /// `instr.offset`, breaking label resolution for control-flow opcodes
    /// inside the bridge (call/jmp pointing to stale positions).
    #[test]
    fn test_insert_bridge_clone_roundtrip_sys4() {
        use crate::editor::bridge::{detect_bridge_template, clone_bridge, TemplateQuality};
        let Some(data) = load_test_bin("SC1310.BIN") else { return };
        let mut parsed = parse_bin(&data).unwrap();
        let template = detect_bridge_template(&parsed.instructions, 4);
        if !matches!(template.quality, TemplateQuality::Clean | TemplateQuality::Degraded) {
            return; // No template available, skip
        }
        // Mark cloned instructions with the synthetic sentinel offset so the
        // writer's label map skips them (matching apply.rs behaviour).
        let cloned: Vec<crate::bin_format::instruction::Instruction> =
            clone_bridge(&template, 0, None)
                .into_iter()
                .map(|mut i| { i.offset = u64::MAX; i })
                .collect();
        for (offset, instr) in cloned.into_iter().enumerate() {
            parsed.instructions.insert(100 + offset, instr);
        }
        let written = write_bin(&parsed.header, &parsed.instructions).expect("write failed");
        parse_bin(&written).expect("re-parse failed — corrupt BIN");
    }

    #[test]
    fn test_roundtrip_sys4() {
        let Some(original) = load_test_bin("SC1310.BIN") else { return };
        let parsed = parse_bin(&original).expect("Failed to parse");
        let written = write_bin(&parsed.header, &parsed.instructions).expect("Failed to write");
        assert_eq!(
            original.len(),
            written.len(),
            "Output size mismatch: original={}, written={}",
            original.len(),
            written.len()
        );
        assert_eq!(original, written, "Round-trip produced different output");
    }

    #[test]
    fn test_roundtrip_sys5() {
        let Some(original) = load_test_bin("SC2230.BIN") else { return };
        let parsed = parse_bin(&original).expect("Failed to parse");
        let written = write_bin(&parsed.header, &parsed.instructions).expect("Failed to write");
        assert_eq!(
            original.len(),
            written.len(),
            "Output size mismatch: original={}, written={}",
            original.len(),
            written.len()
        );
        assert_eq!(original, written, "Round-trip produced different output");
    }
}