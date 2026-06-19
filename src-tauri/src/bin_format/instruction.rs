use serde::{Deserialize, Serialize};

/// A single argument to an instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Argument {
    /// Argument type (0 = literal, 1 = float, 2 = string, 3+ = various register types)
    pub arg_type: u32,
    /// Raw u32 value — for strings this is the offset into string data area
    pub raw_data: u32,
    /// Decoded string content (UTF-8), present only when arg_type == 2
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded_string: Option<String>,
    /// Original native-encoded string bytes (SJIS for SYS4, UTF-16LE for SYS5).
    /// Stored AFTER XOR decoding (i.e., the real encoded bytes, NOT XOR'd).
    /// Used by the writer to avoid encoding round-trip issues (SJIS→UTF8→SJIS).
    /// Cleared when the string is modified, forcing re-encoding from decoded_string.
    #[serde(skip)]
    pub original_encoded_bytes: Option<Vec<u8>>,
    /// Array data, present only for copy-local-array (opcode 0x64) second argument
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_array: Option<DataArray>,
}

/// An array embedded in the BIN file footer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataArray {
    pub length: u32,
    pub data: Vec<u32>,
}

/// A fully parsed instruction from the BIN file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instruction {
    /// Opcode value
    pub opcode: u32,
    /// Human-readable label from the opcode table
    pub label: String,
    /// Expected argument count from the definition
    pub argument_count: u32,
    /// Parsed arguments
    pub arguments: Vec<Argument>,
    /// Offset of this instruction in the instruction area (in u32 units, relative to header end)
    pub offset: u64,
}

impl Instruction {
    /// Returns the byte size of this instruction in the binary file.
    /// opcode (4 bytes) + argument_count * 2 * 4 bytes (type + value per arg)
    pub fn byte_size(&self) -> u64 {
        4 + (self.argument_count as u64) * 8
    }

    /// Returns all string arguments as a collected vector of (arg_index, string_ref).
    pub fn string_arguments(&self) -> Vec<(usize, &str)> {
        self.arguments
            .iter()
            .enumerate()
            .filter_map(|(i, arg)| {
                if arg.arg_type == 2 {
                    arg.decoded_string.as_deref().map(|s| (i, s))
                } else {
                    None
                }
            })
            .collect()
    }
}
