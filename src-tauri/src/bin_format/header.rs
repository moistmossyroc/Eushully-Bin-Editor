use serde::{Deserialize, Serialize};
use std::io;
use thiserror::Error;

/// Raw binary header fields shared between SYS4 and SYS5.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryHeader {
    pub signature: [u8; 8],
    pub local_integer_1: u32,
    pub local_floats: u32,
    pub local_strings_1: u32,
    pub local_integer_2: u32,
    pub unknown_data: u32,
    pub local_strings_2: u32,
    pub sub_header_length: u32, // always 0x1C
    pub table_1_length: u32,
    pub table_1_offset: u32,
    pub table_2_length: u32,
    pub table_2_offset: u32,
    pub table_3_length: u32,
    pub table_3_offset: u32,
}

/// Parsed header with version metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub binary: BinaryHeader,
    /// Total header length in bytes (0x3C for SYS3/SYS4, 0x44 for SYS5)
    pub length: u32,
    /// True for SYS5 (UTF-16 strings), false for SYS3/SYS4 (SJIS strings)
    pub is_ver5: bool,
    /// Engine version number extracted from signature (3, 4, or 5)
    pub sys_version: u8,
    /// Full AGE engine build version from the signature digits after "SYS<n>"
    /// (e.g. `SYS4443` → 443). Distinguishes games/builds within one sys_version:
    /// 415 = Ikusa Megami Zero, 443 = Madou Koukaku, 448 = Sankai no Yubiwa. Used
    /// to select per-build handling (speaker/bridge mechanisms differ per build).
    pub engine_version: u16,
}

/// Parse the build version from the signature: the run of ASCII digits starting
/// right after "SYS<n>" (index 4). `SYS4443 ` → 443.
fn parse_engine_version(signature: &[u8; 8]) -> u16 {
    let mut v: u16 = 0;
    for &b in &signature[4..] {
        if b.is_ascii_digit() {
            v = v.wrapping_mul(10).wrapping_add((b - b'0') as u16);
        } else {
            break;
        }
    }
    v
}

#[derive(Error, Debug)]
pub enum HeaderError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Could not determine header version from signature")]
    UnknownSignature,
}

impl Header {
    /// Parse the header from raw BIN file bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, HeaderError> {
        if data.len() < 4 {
            return Err(HeaderError::UnknownSignature);
        }

        // Check for SYS5: UTF-16LE encoded "SYS5" = [0x53, 0x00, 0x59, 0x00]
        if data.len() >= 0x44
            && data[0] == 0x53
            && data[1] == 0x00
            && data[2] == 0x59
            && data[3] == 0x00
        {
            return Self::parse_sys5(data);
        }

        // Check for SYS3 or SYS4: ASCII "SYS3" / "SYS4"
        if data.len() >= 0x3C
            && data[0] == b'S'
            && data[1] == b'Y'
            && data[2] == b'S'
            && (data[3] == b'3' || data[3] == b'4')
        {
            return Self::parse_sys3_or_sys4(data);
        }

        Err(HeaderError::UnknownSignature)
    }

    fn parse_sys3_or_sys4(data: &[u8]) -> Result<Self, HeaderError> {
        let mut signature = [0u8; 8];
        signature.copy_from_slice(&data[0..8]);
        let sys_version = data[3] - b'0'; // '3' -> 3, '4' -> 4
        let engine_version = parse_engine_version(&signature);

        Ok(Header {
            engine_version,
            binary: BinaryHeader {
                signature,
                local_integer_1: read_u32(data, 0x08),
                local_floats: read_u32(data, 0x0C),
                local_strings_1: read_u32(data, 0x10),
                local_integer_2: read_u32(data, 0x14),
                unknown_data: read_u32(data, 0x18),
                local_strings_2: read_u32(data, 0x1C),
                sub_header_length: read_u32(data, 0x20),
                table_1_length: read_u32(data, 0x24),
                table_1_offset: read_u32(data, 0x28),
                table_2_length: read_u32(data, 0x2C),
                table_2_offset: read_u32(data, 0x30),
                table_3_length: read_u32(data, 0x34),
                table_3_offset: read_u32(data, 0x38),
            },
            length: 0x3C,
            is_ver5: false,
            sys_version,
        })
    }

    fn parse_sys5(data: &[u8]) -> Result<Self, HeaderError> {
        // SYS5 signature is 16 bytes of UTF-16LE, but we store 8 bytes in our struct
        // The C++ code converts the UTF-16 sig to UTF-8 for storage
        let mut signature = [0u8; 8];
        // Extract ASCII chars from the UTF-16LE signature (every other byte)
        for i in 0..8 {
            signature[i] = data[i * 2];
        }
        let engine_version = parse_engine_version(&signature);

        // Fields start at offset 16 (after the 16-byte UTF-16 signature)
        let base = 16;

        Ok(Header {
            engine_version,
            binary: BinaryHeader {
                signature,
                local_integer_1: read_u32(data, base),
                local_floats: read_u32(data, base + 4),
                local_strings_1: read_u32(data, base + 8),
                local_integer_2: read_u32(data, base + 12),
                unknown_data: read_u32(data, base + 16),
                local_strings_2: read_u32(data, base + 20),
                sub_header_length: read_u32(data, base + 24),
                table_1_length: read_u32(data, base + 28),
                table_1_offset: read_u32(data, base + 32),
                table_2_length: read_u32(data, base + 36),
                table_2_offset: read_u32(data, base + 40),
                table_3_length: read_u32(data, base + 44),
                table_3_offset: read_u32(data, base + 48),
            },
            length: 0x44,
            is_ver5: true,
            sys_version: 5,
        })
    }

    /// Serialize the header back to bytes for writing.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.length as usize);

        if self.is_ver5 {
            // Write UTF-16LE signature (8 ASCII chars -> 16 bytes)
            for &byte in &self.binary.signature {
                buf.push(byte);
                buf.push(0x00);
            }
        } else {
            buf.extend_from_slice(&self.binary.signature);
        }

        let h = &self.binary;
        for val in [
            h.local_integer_1,
            h.local_floats,
            h.local_strings_1,
            h.local_integer_2,
            h.unknown_data,
            h.local_strings_2,
            h.sub_header_length,
            h.table_1_length,
            h.table_1_offset,
            h.table_2_length,
            h.table_2_offset,
            h.table_3_length,
            h.table_3_offset,
        ] {
            buf.extend_from_slice(&val.to_le_bytes());
        }

        buf
    }
}

/// Read a little-endian u32 from a byte slice at the given offset.
fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sys4_header_roundtrip() {
        let mut data = vec![0u8; 0x3C];
        // SYS4 signature
        data[0..8].copy_from_slice(b"SYS4472 ");
        // sub_header_length at offset 0x20
        data[0x20..0x24].copy_from_slice(&0x1Cu32.to_le_bytes());

        let header = Header::from_bytes(&data).unwrap();
        assert!(!header.is_ver5);
        assert_eq!(header.sys_version, 4);
        assert_eq!(header.length, 0x3C);
        assert_eq!(header.binary.sub_header_length, 0x1C);

        let roundtrip = header.to_bytes();
        assert_eq!(roundtrip, data);
    }
}
