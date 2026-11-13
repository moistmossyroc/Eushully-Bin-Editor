//! The engine's dialogue text metrics as reported by the fanpatch loader
//! (`fanpatch_layout.txt` in the game folder, written by its `english_word_wrap` hook;
//! docs/engine/age-sys.md §7b). With these the editor reproduces the engine's row
//! breaking exactly: each glyph advances `advance_*` px and starts a new row when
//! `cursor + check_*` would pass `row_right`; rows start at `row_left`.

use std::path::Path;

/// File the loader writes next to the game EXE.
pub const LAYOUT_FILE: &str = "fanpatch_layout.txt";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineTextMetrics {
    pub face: String,
    pub font_height: u32,
    pub font_width: u32,
    pub weight: u32,
    /// Cursor advance of a half-width / full-width glyph (px).
    pub advance_half: u32,
    pub advance_full: u32,
    /// Width the engine's wrap check adds to the cursor for each kind of glyph (px).
    pub check_half: u32,
    pub check_full: u32,
    /// Row extent in the message window's coordinates (same space as the text origin).
    pub row_left: i32,
    pub row_right: i32,
}

/// Parse the loader's `key=value` lines (Shift-JIS face name). `None` when a required
/// key is missing or malformed.
pub fn parse(bytes: &[u8]) -> Option<EngineTextMetrics> {
    let (text, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
    let value = |key: &str| -> Option<&str> {
        text.lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .find_map(|line| line.split_once('=').filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim()))
    };
    let number = |key: &str| -> Option<u32> { value(key)?.parse().ok() };
    let signed = |key: &str| -> Option<i32> { value(key)?.parse().ok() };
    let metrics = EngineTextMetrics {
        face: value("face").unwrap_or_default().to_string(),
        font_height: number("font_height")?,
        font_width: number("font_width").unwrap_or(0),
        weight: number("weight").unwrap_or(400),
        advance_half: number("advance_half")?,
        advance_full: number("advance_full").filter(|v| *v > 0).unwrap_or(number("font_height")?),
        check_half: number("check_half")?,
        check_full: number("check_full").filter(|v| *v > 0).unwrap_or(number("font_height")?),
        row_left: signed("row_left")?,
        row_right: signed("row_right")?,
    };
    (metrics.advance_half > 0 && metrics.row_right > metrics.row_left).then_some(metrics)
}

/// Metrics the loader reported for an installed game, if it has run with the hook.
pub fn read(game_dir: &Path) -> Option<EngineTextMetrics> {
    parse(&std::fs::read(game_dir.join(LAYOUT_FILE)).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATELIAL: &[u8] = b"# comment\r\nface=\x82\x6c\x82\x72 \x83\x53\x83\x56\x83\x62\x83\x4e\r\nfont_height=22\r\nfont_width=11\r\nweight=700\r\nadvance_half=12\r\nadvance_full=22\r\ncheck_half=11\r\ncheck_full=22\r\nrow_left=78\r\nrow_right=788\r\n";

    #[test]
    fn parses_the_atelial_report() {
        let metrics = parse(ATELIAL).unwrap();
        assert_eq!(metrics.face, "ＭＳ ゴシック");
        assert_eq!((metrics.advance_half, metrics.advance_full, metrics.check_half, metrics.check_full), (12, 22, 11, 22));
        assert_eq!((metrics.row_left, metrics.row_right, metrics.weight), (78, 788, 700));
    }

    #[test]
    fn missing_full_width_falls_back_to_the_font_height() {
        let text = b"font_height=22\nadvance_half=12\nadvance_full=0\ncheck_half=11\ncheck_full=0\nrow_left=78\nrow_right=788\n";
        let metrics = parse(text).unwrap();
        assert_eq!((metrics.advance_full, metrics.check_full), (22, 22));
    }

    #[test]
    fn rejects_incomplete_reports() {
        assert!(parse(b"font_height=22\nadvance_half=12\n").is_none());
        assert!(parse(b"font_height=22\nadvance_half=12\ncheck_half=11\nrow_left=500\nrow_right=100\n").is_none());
    }
}
