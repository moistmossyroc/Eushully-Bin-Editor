use crate::bin_format::header::Header;
use crate::bin_format::instruction::{Argument, DataArray, Instruction};
use crate::bin_format::opcodes::definition_for_opcode;
use crate::bin_format::strings::{decode_sjis_xor, decode_utf16_xor};
use thiserror::Error;

/// Complete parsed representation of a BIN file.
#[derive(Debug, Clone)]
pub struct ParsedBin {
    pub header: Header,
    pub instructions: Vec<Instruction>,
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Header error: {0}")]
    Header(#[from] crate::bin_format::header::HeaderError),
    #[error("Unknown opcode 0x{opcode:X} at offset 0x{offset:X}")]
    UnknownOpcode { opcode: u32, offset: u64 },
    #[error("Unexpected end of data at offset 0x{0:X}")]
    UnexpectedEof(u64),
    #[error("Zero opcode at offset 0x{0:X}")]
    ZeroOpcode(u64),
}

/// Read a little-endian u32 from data at the given byte offset.
fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Parse a BIN file from raw bytes into a structured representation.
pub fn parse_bin(data: &[u8]) -> Result<ParsedBin, ParseError> {
    let header = Header::from_bytes(data)?;
    let header_len = header.length as u64;

    // Compute the end of the instruction data area.
    // It's bounded by the earliest of the three footer table offsets.
    let min_table_offset = header
        .binary
        .table_1_offset
        .min(header.binary.table_2_offset)
        .min(header.binary.table_3_offset);
    let mut data_array_end = header_len + ((min_table_offset as u64) << 2);

    let mut instructions = Vec::with_capacity(5000);
    let mut pos = header_len as usize;

    while (pos as u64) < data_array_end {
        if pos + 4 > data.len() {
            return Err(ParseError::UnexpectedEof(pos as u64));
        }

        let instruction_offset = pos as u64;
        let opcode = read_u32(data, pos);
        pos += 4;

        if opcode == 0 {
            return Err(ParseError::ZeroOpcode(instruction_offset));
        }

        let def = definition_for_opcode(opcode).ok_or(ParseError::UnknownOpcode {
            opcode,
            offset: instruction_offset,
        })?;

        let mut arguments = Vec::with_capacity(def.argument_count as usize);

        for arg_idx in 0..def.argument_count {
            if pos + 8 > data.len() {
                return Err(ParseError::UnexpectedEof(pos as u64));
            }

            let arg_type = read_u32(data, pos);
            let raw_data = read_u32(data, pos + 4);
            pos += 8;

            let mut decoded_string = None;
            let mut original_encoded_bytes = None;
            let mut data_array = None;

            // String argument: decode from the string data area
            if arg_type == 2 {
                let string_offset = header_len as usize + ((raw_data as usize) << 2);
                // Update data_array_end to exclude string data from instruction area
                data_array_end = data_array_end.min(string_offset as u64);

                if string_offset < data.len() {
                    let string_data = &data[string_offset..];
                    let (decoded, _, native_bytes) = if header.is_ver5 {
                        decode_utf16_xor(string_data)
                    } else {
                        decode_sjis_xor(string_data)
                    };
                    decoded_string = Some(decoded);
                    original_encoded_bytes = Some(native_bytes);
                }
            }
            // Array argument for copy-local-array (opcode 0x64, second argument)
            else if def.opcode == 0x64 && arg_idx == 1 {
                let array_offset = header_len as usize + ((raw_data as usize) << 2);
                data_array_end = data_array_end.min(array_offset as u64);

                if array_offset + 4 <= data.len() {
                    let array_len = read_u32(data, array_offset) as usize;
                    let mut array_data = Vec::with_capacity(array_len);
                    for i in 0..array_len {
                        let val_offset = array_offset + 4 + (i * 4);
                        if val_offset + 4 <= data.len() {
                            array_data.push(read_u32(data, val_offset));
                        }
                    }
                    data_array = Some(DataArray {
                        length: array_len as u32,
                        data: array_data,
                    });
                }
            }

            arguments.push(Argument {
                arg_type,
                raw_data,
                decoded_string,
                original_encoded_bytes,
                data_array,
            });
        }

        // Store offset in u32 units relative to header end (matching C++ convention)
        let relative_offset = (instruction_offset - header_len) >> 2;

        instructions.push(Instruction {
            opcode,
            label: def.label.to_string(),
            argument_count: def.argument_count,
            arguments,
            offset: relative_offset,
        });
    }

    Ok(ParsedBin {
        header,
        instructions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn test_parse_sys4_bin() {
        let Some(data) = load_test_bin("SC1310.BIN") else { return };
        let parsed = parse_bin(&data).expect("Failed to parse SYS4 BIN");

        assert!(!parsed.header.is_ver5);
        assert_eq!(parsed.header.length, 0x3C);
        assert!(!parsed.instructions.is_empty(), "No instructions parsed");

        // Verify we have some show-text instructions
        let show_text_count = parsed
            .instructions
            .iter()
            .filter(|i| i.opcode == 0x6E)
            .count();
        assert!(
            show_text_count > 0,
            "Expected show-text instructions in SC1310.BIN"
        );

        // Verify strings were decoded
        for instr in &parsed.instructions {
            for arg in &instr.arguments {
                if arg.arg_type == 2 {
                    assert!(
                        arg.decoded_string.is_some(),
                        "String argument not decoded for {} at offset {}",
                        instr.label,
                        instr.offset
                    );
                }
            }
        }
    }

    #[test]
    fn test_parse_sys5_bin() {
        let Some(data) = load_test_bin("SC2230.BIN") else { return };
        let parsed = parse_bin(&data).expect("Failed to parse SYS5 BIN");

        assert!(parsed.header.is_ver5);
        assert_eq!(parsed.header.length, 0x44);
        assert!(!parsed.instructions.is_empty(), "No instructions parsed");

        let show_text_count = parsed
            .instructions
            .iter()
            .filter(|i| i.opcode == 0x6E)
            .count();
        assert!(
            show_text_count > 0,
            "Expected show-text instructions in SC2230.BIN"
        );
    }

}
