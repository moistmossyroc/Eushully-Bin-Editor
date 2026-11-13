// file_magic.rs — Content sniffing by magic bytes.
//
// Used where a file's name doesn't reflect its content: Parente stores PNGs
// under `.bytes` names and FMOD FSB5 audio under `.ogg` names. Only strong
// signatures (>= 4 bytes, or a fixed-offset box tag) are listed so plain text
// and AGE-native formats (AGF, BIN, …) are never misclassified.

/// (signature, byte offset where it must appear, canonical extension)
const SIGNATURES: &[(&[u8], usize, &str)] = &[
    (b"\x89PNG\r\n\x1a\n", 0, "png"),
    (b"\xFF\xD8\xFF",      0, "jpg"),
    (b"OggS",              0, "ogg"),
    (b"FSB5",              0, "fsb"),
    (b"UnityFS\0",         0, "bundle"),
    (b"ftyp",              4, "mp4"),
    // MPEG program stream pack header. AGE ships its movies under .AGF names.
    (&[0x00, 0x00, 0x01, 0xBA], 0, "mpg"),
];

/// RIFF sub-formats, keyed on the form type at offset 8.
const RIFF_FORMS: &[(&[u8], &str)] = &[
    (b"WAVE", "wav"),
    (b"XWMA", "xwm"),
];

fn has_signature_at(data: &[u8], signature: &[u8], offset: usize) -> bool {
    data.get(offset..offset + signature.len()) == Some(signature)
}

/// Return the canonical extension for `data` if it starts with a known
/// signature, otherwise `None` (caller should fall back to the file name).
pub fn detect_extension(data: &[u8]) -> Option<&'static str> {
    if let Some(&(_, _, ext)) = SIGNATURES
        .iter()
        .find(|(signature, offset, _)| has_signature_at(data, signature, *offset))
    {
        return Some(ext);
    }
    if has_signature_at(data, b"RIFF", 0) {
        return RIFF_FORMS
            .iter()
            .find(|(form, _)| has_signature_at(data, form, 8))
            .map(|&(_, ext)| ext);
    }
    None
}

/// Lower-cased extension of a file name (text after the last '.' of the last
/// path component), or "" when there is none.
pub fn name_extension(file_name: &str) -> String {
    let last_component = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    match last_component.rsplit_once('.') {
        Some((_, ext)) => ext.to_lowercase(),
        None => String::new(),
    }
}

/// Spellings that name the same format as a sniffed canonical extension.
const EXTENSION_ALIASES: &[(&str, &str)] = &[("jpg", "jpeg"), ("xwm", "xwma")];

/// True when a file-name extension already denotes the sniffed format.
pub fn extension_matches(sniffed: &str, name_ext: &str) -> bool {
    sniffed == name_ext
        || EXTENSION_ALIASES
            .iter()
            .any(|&(canonical, alias)| canonical == sniffed && alias == name_ext)
}

/// Extension to use for `data`: sniffed content type first, file name second.
pub fn effective_extension(file_name: &str, data: &[u8]) -> String {
    detect_extension(data)
        .map(str::to_string)
        .unwrap_or_else(|| name_extension(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_png_regardless_of_name() {
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        assert_eq!(effective_extension("img/ev/ev157bd.bytes", png), "png");
    }

    #[test]
    fn detects_fsb_and_mp4_and_riff_forms() {
        assert_eq!(detect_extension(b"FSB5\x01\0\0\0"), Some("fsb"));
        assert_eq!(detect_extension(b"\0\0\0\x18ftypmp42"), Some("mp4"));
        assert_eq!(detect_extension(b"RIFF\0\0\0\0WAVEfmt "), Some("wav"));
        assert_eq!(detect_extension(b"RIFF\0\0\0\0XWMAfmt "), Some("xwm"));
        assert_eq!(detect_extension(b"RIFF\0\0\0\0AVI LIST"), None);
        assert_eq!(detect_extension(&[0x00, 0x00, 0x01, 0xBA, 0x21]), Some("mpg"));
    }

    #[test]
    fn aliases_count_as_matching() {
        assert!(extension_matches("jpg", "jpeg"));
        assert!(extension_matches("png", "png"));
        assert!(!extension_matches("png", "bytes"));
    }

    #[test]
    fn falls_back_to_name_for_unknown_content() {
        assert_eq!(effective_extension("src/game/sc0001.TXT", b"-- Liwei --"), "txt");
        assert_eq!(effective_extension("pfb/ui/ui_title", b"Manifest"), "");
        assert_eq!(effective_extension("dir.with.dot/file", b"xx"), "");
    }
}
