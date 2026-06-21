//! Synthetic, engine-specific round-trip tests for the BIN pipeline.
//!
//! Each engine is its own operator: SYS3/SYS4 store strings as SJIS, SYS5 as
//! UTF-16LE, both XOR'd. These tests build a minimal in-memory BIN per version
//! and assert parse↔write is lossless — opcodes, instruction count, and the
//! (engine-encoded) string args all survive.
//!
//! Unlike `roundtrip.rs` (which needs a real Sankai BIN and skips when absent),
//! these need NO game data, so they run everywhere — CI, clean clones, any box.
//! `write_bin` recomputes the footer-table offsets from the instructions, so a
//! header with 0-placeholder offsets is sufficient.

use eushully_bin_editor_lib::bin_format::header::{BinaryHeader, Header};
use eushully_bin_editor_lib::bin_format::instruction::{Argument, Instruction};
use eushully_bin_editor_lib::bin_format::opcodes::definition_for_opcode;
use eushully_bin_editor_lib::bin_format::parser::parse_bin;
use eushully_bin_editor_lib::bin_format::writer::write_bin;
use eushully_bin_editor_lib::editor::apply::apply_changes;
use eushully_bin_editor_lib::project::types::{InsertedInstruction, SerializedArg};

/// Minimal valid header for `sys_version`. Table offsets/lengths are recomputed
/// by `write_bin`, so the 0 placeholders here are fine.
fn synthetic_header(sys_version: u8) -> Header {
    let is_ver5 = sys_version == 5;
    let signature = [b'S', b'Y', b'S', b'0' + sys_version, 0, 0, 0, 0];
    Header {
        binary: BinaryHeader {
            signature,
            local_integer_1: 0,
            local_floats: 0,
            local_strings_1: 0,
            local_integer_2: 0,
            unknown_data: 0,
            local_strings_2: 0,
            sub_header_length: 0x1C,
            table_1_length: 0,
            table_1_offset: 0,
            table_2_length: 0,
            table_2_offset: 0,
            table_3_length: 0,
            table_3_offset: 0,
        },
        length: if is_ver5 { 0x44 } else { 0x3C },
        is_ver5,
        sys_version,
    }
}

/// Build an instruction with the correct argument count for its opcode.
/// `string_args` maps an argument index -> string content (those become
/// arg_type 2); every other argument is an arg_type 0 literal. `raw_data` stays
/// 0 — `write_bin` fills the real string offsets itself.
fn make_instr(opcode: u32, string_args: &[(usize, &str)], offset: u64) -> Instruction {
    let def = definition_for_opcode(opcode).expect("opcode in table");
    let arguments = (0..def.argument_count as usize)
        .map(|i| match string_args.iter().find(|(idx, _)| *idx == i) {
            Some((_, text)) => Argument {
                arg_type: 2,
                raw_data: 0,
                decoded_string: Some((*text).to_string()),
                original_encoded_bytes: None,
                data_array: None,
            },
            None => Argument {
                arg_type: 0,
                raw_data: 0,
                decoded_string: None,
                original_encoded_bytes: None,
                data_array: None,
            },
        })
        .collect();
    Instruction {
        opcode,
        label: def.label.to_string(),
        argument_count: def.argument_count,
        arguments,
        offset,
    }
}

/// A short dialogue: wait-for-input, then a 2-line textbox. The strings mix JP,
/// ASCII, and full-width brackets — all representable in both SJIS and UTF-16,
/// so the one fixture validates every engine's codec path through a full BIN.
fn dialogue_stream() -> Vec<Instruction> {
    vec![
        make_instr(0x72, &[], 0),                                   // wait-for-input
        make_instr(0x6E, &[(1, "「こんにちは、Hello 世界」")], 1),   // show-text, line 1
        make_instr(0x6F, &[], 2),                                   // end-text-line
        make_instr(0x6E, &[(1, "二行目です。Second line.")], 3),     // show-text, line 2
    ]
}

fn assert_roundtrips(sys_version: u8) {
    let header = synthetic_header(sys_version);
    let instrs = dialogue_stream();

    let bytes = write_bin(&header, &instrs).expect("write_bin");
    let parsed = parse_bin(&bytes).expect("re-parse synthetic BIN");

    assert_eq!(parsed.header.sys_version, sys_version, "sys_version lost");
    assert_eq!(parsed.header.is_ver5, sys_version == 5, "is_ver5 wrong");
    assert_eq!(
        parsed.instructions.len(),
        instrs.len(),
        "instruction count changed (SYS{sys_version})"
    );

    for (i, (a, b)) in instrs.iter().zip(parsed.instructions.iter()).enumerate() {
        assert_eq!(a.opcode, b.opcode, "opcode differs at {i} (SYS{sys_version})");
        let want: Vec<&str> = a.arguments.iter().filter_map(|x| x.decoded_string.as_deref()).collect();
        let got: Vec<&str> = b.arguments.iter().filter_map(|x| x.decoded_string.as_deref()).collect();
        assert_eq!(want, got, "string args corrupted at {i} (SYS{sys_version})");
    }
}

#[test]
fn synthetic_roundtrip_sys3() {
    assert_roundtrips(3);
}

#[test]
fn synthetic_roundtrip_sys4() {
    assert_roundtrips(4);
}

#[test]
fn synthetic_roundtrip_sys5() {
    assert_roundtrips(5);
}

/// The insertion-ordering contract (the class of bug behind the in-game line
/// swap), validated on a synthetic BIN so it runs without game data. Two inserts
/// after the SAME index, sent in reverse-visual order [show-text, end-text-line],
/// must come out [end-text-line, show-text] and survive the byte round-trip.
#[test]
fn synthetic_insertion_ordering_contract() {
    let header = synthetic_header(4);
    let instrs = dialogue_stream();
    let x = instrs.iter().position(|i| i.opcode == 0x6E).expect("a show-text exists");

    let inserts = vec![
        InsertedInstruction {
            after_index: x,
            opcode: 0x6E,
            label: "show-text".into(),
            arguments: vec![
                SerializedArg { arg_type: 0, raw_value: 0, string_value: None },
                SerializedArg { arg_type: 2, raw_value: 0, string_value: Some("CONT".into()) },
            ],
        },
        InsertedInstruction {
            after_index: x,
            opcode: 0x6F,
            label: "end-text-line".into(),
            arguments: vec![SerializedArg { arg_type: 0, raw_value: 0, string_value: None }],
        },
    ];

    let result = apply_changes(&instrs, &[], &inserts);
    assert_eq!(result.len(), instrs.len() + 2);
    assert_eq!(result[x + 1].opcode, 0x6F, "end-text-line must come first");
    assert_eq!(result[x + 2].opcode, 0x6E, "continuation show-text must come second");

    let bytes = write_bin(&header, &result).expect("write_bin");
    let reparsed = parse_bin(&bytes).expect("re-parse");
    assert_eq!(reparsed.instructions[x + 1].opcode, 0x6F);
    assert_eq!(reparsed.instructions[x + 2].opcode, 0x6E);
    let cont = reparsed.instructions[x + 2]
        .arguments
        .iter()
        .find(|a| a.arg_type == 2)
        .and_then(|a| a.decoded_string.as_deref());
    assert_eq!(cont, Some("CONT"), "continuation text corrupted in round-trip");
}
