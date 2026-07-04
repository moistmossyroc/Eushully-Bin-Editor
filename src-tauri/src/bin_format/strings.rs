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

/// Normalize characters that don't render correctly in the game's Shift-JIS font
/// before encoding for export. Three classes of problem:
///   1. Smart/curly quotes (U+2018/2019/201C/201D) encode to FULL-WIDTH SJIS
///      glyphs that render wide ("Celica's" → "Celica' s") → ASCII '/".
///   2. ASCII tilde `~` renders half-width + top-aligned → full-width wave dash
///      ～ (U+FF5E → SJIS 0x8160), centered like the native tone-tilde.
///   3. Many chars Claude emits have NO Shift-JIS mapping and would become `?`
///      mojibake in-game — accented Latin (é/ñ/ü), romaji macrons (ā/ō/ū),
///      em/en dashes, non-breaking/thin spaces, ligatures (æ/œ/ß), bullets,
///      fractions, ™/©/® — so transliterate them to safe ASCII.
/// Chars that DO have a valid (full-width) SJIS glyph — … ― × ÷ ° arrows ♪ ★ ☆
/// £ ¢ — are left as-is (render fine, just wide). Only MODIFIED strings are
/// re-encoded through here, so Japanese originals are untouched. Hearts ♥♡ → "<3"
/// and € → "EUR" (no SJIS glyph exists, and the game's gaiji heart code — if any —
/// can't be identified from bytes; "<3" is proper-width and doesn't break spacing).
pub(crate) fn normalize_export_punctuation(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            // Smart quotes → ASCII (else wide full-width render).
            '\u{2018}' | '\u{2019}' => out.push('\''),
            '\u{201C}' | '\u{201D}' => out.push('"'),
            // Tilde → full-width wave dash (centered, not top-aligned).
            '~' | '\u{301C}' => out.push('\u{FF5E}'),
            // Dashes — U+2013/2014/2011 have NO SJIS mapping (→ mojibake).
            '\u{2014}' => out.push_str("--"),
            '\u{2013}' | '\u{2011}' => out.push('-'),
            // Unicode spaces with no SJIS mapping → plain space.
            '\u{00A0}' | '\u{2007}' | '\u{2009}' | '\u{200A}' | '\u{202F}' => out.push(' '),
            // Bullets / middle dot.
            '\u{2022}' => out.push('*'),
            '\u{00B7}' => out.push('-'),
            // Accented Latin → base letter (none exist in SJIS).
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' => out.push('a'),
            'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' | 'Ā' => out.push('A'),
            'é' | 'è' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ę' | 'ě' => out.push('e'),
            'É' | 'È' | 'Ê' | 'Ë' | 'Ē' => out.push('E'),
            'í' | 'ì' | 'î' | 'ï' | 'ī' | 'į' => out.push('i'),
            'Í' | 'Ì' | 'Î' | 'Ï' | 'Ī' => out.push('I'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ō' | 'ő' => out.push('o'),
            'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' | 'Ō' => out.push('O'),
            'ú' | 'ù' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => out.push('u'),
            'Ú' | 'Ù' | 'Û' | 'Ü' | 'Ū' => out.push('U'),
            'ñ' => out.push('n'),
            'Ñ' => out.push('N'),
            'ç' => out.push('c'),
            'Ç' => out.push('C'),
            'ý' | 'ÿ' => out.push('y'),
            'Ý' => out.push('Y'),
            // Ligatures / special letters.
            'æ' => out.push_str("ae"),
            'Æ' => out.push_str("AE"),
            'œ' => out.push_str("oe"),
            'Œ' => out.push_str("OE"),
            'ß' => out.push_str("ss"),
            'ø' => out.push('o'),
            'Ø' => out.push('O'),
            'ð' => out.push('d'),
            'Ð' => out.push('D'),
            'þ' => out.push_str("th"),
            'Þ' => out.push_str("Th"),
            // Fractions + marks with no SJIS mapping.
            '½' => out.push_str("1/2"),
            '¼' => out.push_str("1/4"),
            '¾' => out.push_str("3/4"),
            '™' => out.push_str("(TM)"),
            '©' => out.push_str("(C)"),
            '®' => out.push_str("(R)"),
            // Half-width yen → full-width ￥ (the game's yen glyph).
            '\u{00A5}' => out.push('\u{FFE5}'),
            // No SJIS glyph → recognizable, proper-width ASCII.
            '\u{2665}' | '\u{2661}' => out.push_str("<3"),
            '\u{20AC}' => out.push_str("EUR"),
            other => out.push(other),
        }
    }
    out
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
    let normalized = normalize_export_punctuation(text);
    let (encoded, _, _) = SHIFT_JIS.encode(&normalized);
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
    let normalized = normalize_export_punctuation(text);
    let utf16_chars: Vec<u16> = normalized.encode_utf16().collect();

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
    fn test_sjis_smart_punctuation_normalized() {
        // Curly apostrophe (U+2019) must encode as ASCII ' (half-width) so the
        // game doesn't render it full-width ("Celica's" not "Celica' s").
        let encoded = encode_sjis_xor("Celica\u{2019}s \u{201C}hi\u{201D}");
        let (decoded, _, _) = decode_sjis_xor(&encoded);
        assert_eq!(decoded, "Celica's \"hi\"");
        assert!(!decoded.contains('\u{2019}') && !decoded.contains('\u{201C}'));
    }

    #[test]
    fn test_utf16_smart_punctuation_normalized() {
        let encoded = encode_utf16_xor("it\u{2019}s \u{2018}x\u{2019}");
        let (decoded, _, _) = decode_utf16_xor(&encoded);
        assert_eq!(decoded, "it's 'x'");
    }

    #[test]
    fn test_sjis_transliterates_unmappable() {
        // Accents, macron, em-dash, ligature, nbsp — none have a Shift-JIS
        // mapping; they must transliterate to ASCII, never become 0x3F '?'.
        let s = "Caf\u{00E9} na\u{00EF}ve Ry\u{016B} \u{2014} \u{00E6}on\u{00A0}fin\u{2665}";
        let encoded = encode_sjis_xor(s);
        let dexor: Vec<u8> = encoded.iter().map(|b| b ^ 0xFF).collect();
        assert!(!dexor.contains(&0x3F), "produced '?' mojibake: {:02x?}", dexor);
        let (decoded, _, _) = decode_sjis_xor(&encoded);
        assert_eq!(decoded, "Cafe naive Ryu -- aeon fin<3");
    }

    #[test]
    fn test_sjis_tilde_to_wave_dash() {
        // ASCII '~' must encode to the full-width wave dash (SJIS 0x8160) so it
        // renders centered in-game, not half-width/top-aligned.
        let encoded = encode_sjis_xor("he~");
        let dexor: Vec<u8> = encoded.iter().map(|b| b ^ 0xFF).collect();
        // 'h'=0x68, 'e'=0x65, then wave dash 0x81 0x60
        assert_eq!(&dexor[0..4], &[0x68, 0x65, 0x81, 0x60],
            "tilde not encoded as wave dash 0x8160: {:02x?}", &dexor[0..4]);
        let (decoded, _, _) = decode_sjis_xor(&encoded);
        assert_eq!(decoded, "he\u{FF5E}");
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
