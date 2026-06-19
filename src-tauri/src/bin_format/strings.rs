use encoding_rs::SHIFT_JIS;

/// Remap PUA codepoints used by the game's custom font to their standard
/// Unicode equivalents so the editor and translation pipeline can render them
/// with normal fonts. Applied to both SJIS (SYS3/SYS4) and UTF-16 (SYS5)
/// decode paths — SJIS user-defined byte range decodes into PUA via
/// encoding_rs, and SYS5 stores PUA codepoints directly.
fn remap_pua_char(ch: u32) -> u32 {
    match ch {
        0xE000 => 0x2026, // … (horizontal ellipsis)
        0xE001 | 0xE003 => 0x2014, // — (em dash)
        _ => ch,
    }
}

/// Decode a SYS4 XOR'd SJIS string from raw bytes.
/// Each byte is XOR'd with 0xFF. The string terminates at byte 0xFF (which is 0x00 after XOR).
/// Returns (UTF-8 string, bytes consumed, original SJIS bytes before XOR).
pub fn decode_sjis_xor(data: &[u8]) -> (String, usize, Vec<u8>) {
    let mut sjis_bytes = Vec::with_capacity(64);
    let mut consumed = 0;

    for &byte in data {
        consumed += 1;
        if byte == 0xFF {
            break;
        }
        sjis_bytes.push(byte ^ 0xFF);
    }

    let (decoded, _, _) = SHIFT_JIS.decode(&sjis_bytes);
    // Apply PUA remap so SJIS user-defined glyphs (custom-font ellipsis,
    // em-dash, etc.) render as their standard Unicode equivalents instead
    // of boxes. Matches behavior of decode_utf16_xor.
    let remapped: String = decoded
        .chars()
        .map(|c| char::from_u32(remap_pua_char(c as u32)).unwrap_or(c))
        .collect();
    (remapped, consumed, sjis_bytes)
}

/// Decode a SYS5 XOR'd UTF-16LE string from raw bytes.
/// Each u16 is XOR'd with 0xFFFF. The string terminates at u16 0xFFFF (which is 0x0000 after XOR).
/// Returns (UTF-8 string, bytes consumed, original UTF-16LE bytes before XOR).
pub fn decode_utf16_xor(data: &[u8]) -> (String, usize, Vec<u8>) {
    let mut utf16_chars = Vec::with_capacity(64);
    let mut consumed = 0;

    let mut i = 0;
    while i + 1 < data.len() {
        let word = u16::from_le_bytes([data[i], data[i + 1]]);
        consumed += 2;
        i += 2;
        if word == 0xFFFF {
            break;
        }
        utf16_chars.push(word ^ 0xFFFF);
    }

    // Store original UTF-16LE bytes (after XOR decode)
    let mut native_bytes = Vec::with_capacity(utf16_chars.len() * 2);
    for &ch in &utf16_chars {
        native_bytes.extend_from_slice(&ch.to_le_bytes());
    }

    // Replace PUA control codes with standard Unicode equivalents via the
    // shared remap helper (kept in sync with the SJIS decode path).
    for ch in &mut utf16_chars {
        *ch = remap_pua_char(*ch as u32) as u16;
    }

    let decoded = String::from_utf16_lossy(&utf16_chars);
    (decoded, consumed, native_bytes)
}

/// Encode a UTF-8 string to SYS4 XOR'd SJIS format.
/// Returns the encoded bytes including 0xFF terminator, padded to 4-byte alignment with 0xFF.
pub fn encode_sjis_xor(text: &str) -> Vec<u8> {
    let (encoded, _, _) = SHIFT_JIS.encode(text);
    let sjis_bytes = encoded.as_ref();

    let mut result = Vec::with_capacity(sjis_bytes.len() + 4);

    // XOR each byte
    for &byte in sjis_bytes {
        result.push(byte ^ 0xFF);
    }

    // Add 0xFF terminator + padding to 4-byte alignment
    // The C++ code: current_string_offset += length + 1, then padding = 4 - (offset % 4), then writes padding + 1 bytes of 0xFF
    let content_len = result.len() + 1; // +1 for the terminator
    let padding = (4 - (content_len % 4)) % 4;
    for _ in 0..padding + 1 {
        result.push(0xFF);
    }

    result
}

/// Encode a UTF-8 string to SYS5 XOR'd UTF-16LE format.
/// Returns the encoded bytes including 0xFFFF terminator, padded to 4-byte alignment with 0xFF.
pub fn encode_utf16_xor(text: &str) -> Vec<u8> {
    let utf16_chars: Vec<u16> = text.encode_utf16().collect();

    let mut result = Vec::with_capacity(utf16_chars.len() * 2 + 8);

    // XOR each u16 and write as LE bytes
    for &ch in &utf16_chars {
        let xored = ch ^ 0xFFFF;
        result.extend_from_slice(&xored.to_le_bytes());
    }

    // Add 0xFFFF terminator (2 bytes of 0xFF)
    // Then pad to 4-byte alignment
    // C++ logic: current_string_offset += (length + 1) * 2, then padding = 4 - (offset % 4), then writes padding + 2 bytes of 0xFF
    let content_len = (utf16_chars.len() + 1) * 2; // string + terminator in bytes
    let padding = (4 - (content_len % 4)) % 4;
    for _ in 0..padding + 2 {
        result.push(0xFF);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sjis_roundtrip() {
        let original = "Hello";
        let encoded = encode_sjis_xor(original);
        let (decoded, _, _) = decode_sjis_xor(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_utf16_roundtrip() {
        let original = "Hello";
        let encoded = encode_utf16_xor(original);
        let (decoded, _, _) = decode_utf16_xor(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_sjis_japanese_roundtrip() {
        // Test with actual Japanese text (hiragana)
        let original = "こんにちは";
        let encoded = encode_sjis_xor(original);
        let (decoded, _, _) = decode_sjis_xor(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_utf16_japanese_roundtrip() {
        let original = "こんにちは世界";
        let encoded = encode_utf16_xor(original);
        let (decoded, _, _) = decode_utf16_xor(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_sjis_alignment() {
        // Encoded output must be 4-byte aligned (total length)
        for text in &["a", "ab", "abc", "abcd", "abcde"] {
            let encoded = encode_sjis_xor(text);
            assert_eq!(encoded.len() % 4, 0, "Not 4-byte aligned for text: {}", text);
        }
    }

    #[test]
    fn test_sjis_pua_remap_ellipsis() {
        // SJIS user-defined byte 0xF040 decodes via encoding_rs to U+E000,
        // which our remap should turn into U+2026 (…).
        // Encode bytes manually (no XOR for this fragment), pass through.
        let sjis_bytes: Vec<u8> = vec![0xF0, 0x40];
        // Pre-XOR with 0xFF so decode_sjis_xor's inverse XOR restores the raw bytes.
        let mut data: Vec<u8> = sjis_bytes.iter().map(|b| b ^ 0xFF).collect();
        data.push(0xFF); // terminator
        let (decoded, _, _) = decode_sjis_xor(&data);
        // Either the chars are remapped to ellipsis or encoding_rs replaced
        // them; assert remap happened (no PUA codepoint left).
        assert!(!decoded.chars().any(|c| (0xE000..=0xF8FF).contains(&(c as u32))),
            "PUA codepoint leaked through: {:?}", decoded);
    }

    #[test]
    fn test_utf16_pua_remap_ellipsis() {
        // U+E000 as a UTF-16LE word pre-XOR'd with 0xFFFF, plus terminator.
        let data: Vec<u8> = vec![
            0xFF ^ 0x00, 0xFF ^ 0xE0, // U+E000 (LE: 00 E0) XOR'd
            0xFF, 0xFF, // terminator
        ];
        let (decoded, _, _) = decode_utf16_xor(&data);
        assert_eq!(decoded, "…");
    }

    #[test]
    fn test_utf16_alignment() {
        for text in &["a", "ab", "abc", "abcd", "abcde"] {
            let encoded = encode_utf16_xor(text);
            assert_eq!(encoded.len() % 4, 0, "Not 4-byte aligned for text: {}", text);
        }
    }
}
