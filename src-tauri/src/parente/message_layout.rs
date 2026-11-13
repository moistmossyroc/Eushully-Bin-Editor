//! Parente message-window layout model (decompiled 2026-11-12, parente/PARENTE_ENGINE.md §12):
//! how `UI_MessageWindow.SetMessageBase` lays a `text()` string into rows, so the editor can
//! predict overflow and pre-break English at word boundaries (the engine breaks mid-word).
//!
//! Engine rules (defaults from the constructor; nothing overrides them):
//! - the string is first split at `\n` (`Division_NewLine`); each part starts a row;
//! - glyph advance = `GetFontWidth(c, MsgWinFontWidth = 42)`: half (21 px) when
//!   `StringEx.IsHankaku(c)` — UTF-16 high byte 0, i.e. `c <= U+00FF` — else 42 px. So ASCII
//!   is half-width but curly quotes, dashes and `…` are FULL-width;
//! - before placing a glyph, when the row is already at the limit or past it, a new row starts
//!   — unless the glyph is in `INDENTED` (closing brackets, `！？、。`), which hangs past the
//!   edge instead. MEASURED in-game (2026-11-13): a row holds 52 half-width or 26 full-width
//!   glyphs, i.e. a 1092 px limit (26 × 42), not the decompiled `TextW` 28 × 42;
//! - the message's first glyph, when in `INDENTST` (opening brackets), is shifted left by its
//!   own width (hanging indent), giving that row one extra cell;
//! - `TextH` (3) is never read: a 4th row is drawn below the window, off-screen. The box
//!   shows 3 rows ([`NATIVE_ROWS`], in-game verified 2026-11-13);
//! - half-width `(` … `)` is an inline COMMAND: `(explicit newline)` logged
//!   `error!!:command:explicit newline`, then `AddRuby` threw and the message hung (in-game
//!   2026-11-13). English must not contain ASCII parentheses.
//!
//! Not modelled: per-glyph `fontScale` from inline commands (`AddScaling`; plain text = 1.0).

/// Row limit in pixels, measured in-game (2026-11-13): 26 full-width / 52 half-width glyphs.
pub const ROW_LIMIT_PX: u32 = 26 * FULL_WIDTH_PX;
/// Full-width glyph advance (`MsgWinFontWidth`).
pub const FULL_WIDTH_PX: u32 = 42;
/// Half-width glyph advance (`GetFontWidth` × 0.5).
pub const HALF_WIDTH_PX: u32 = FULL_WIDTH_PX / 2;
/// Rows the message box shows (in-game verified 2026-11-13; a 4th row is off-screen).
pub const NATIVE_ROWS: usize = 3;

/// `UI_MessageWindow.INDENTED`: glyphs that hang past the row edge instead of wrapping.
const INDENT_ED: &str = "｝）〕〉》」』】〙〗〟’”｠！？、。";
/// `UI_MessageWindow.INDENTST`: a message starting with one of these hangs it left.
const INDENT_ST: &str = "（｛〔〈《「『【〘〖〝‘“｟";

/// Advance of one glyph in pixels (`GetFontWidth` with the default 42 px cell).
pub fn glyph_width(c: char) -> u32 {
    if (c as u32) <= 0xFF { HALF_WIDTH_PX } else { FULL_WIDTH_PX }
}

/// The text drawn in the row: ruby commands `<base,reading>` keep only `base` (the
/// reading is drawn above the row by `AddRuby` and takes no row width).
pub fn visible_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        match tail.find('>').map(|close| &tail[..close]).and_then(|inner| inner.split_once(',').map(|(base, _)| (base, inner.len()))) {
            Some((base, inner_len)) => {
                out.push_str(base);
                rest = &tail[inner_len + 1..];
            }
            None => {
                out.push('<');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Makes a translated `text()` string safe for the message window (in-game 2026-11-13).
/// `UI_MessageWindow.GetCommand` treats every `<…>` as an inline command: with a `,` it is
/// ruby, a few keywords are scale/size commands, anything else logs `error!!:command:` and
/// leaves a half-built entry that makes `AddRuby` throw — the message HANGS. ASCII `( … )`
/// becomes `< … >` first (`NormalizeCommand`), so English parentheses hang it too. There is
/// no escape. So: `(` `)` → `[` `]` (half-width, reads naturally; `[ ]` and `{ }` verified),
/// and a `<` / `>` that is not part of a ruby `<base,reading>` → full-width `＜` `＞`.
pub fn message_safe_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    // Indices of `<` `>` belonging to a ruby command `<…,…>` (no nested `<`).
    let mut ruby = vec![false; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            if let Some(close) = chars[i + 1..].iter().position(|&c| c == '>' || c == '<').map(|p| p + i + 1) {
                if chars[close] == '>' && chars[i + 1..close].contains(&',') {
                    ruby[i] = true;
                    ruby[close] = true;
                    i = close + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    chars
        .iter()
        .enumerate()
        .map(|(k, &c)| match c {
            '(' => '[',
            ')' => ']',
            '<' if !ruby[k] => '＜',
            '>' if !ruby[k] => '＞',
            _ => c,
        })
        .collect()
}

/// The rows the engine draws for `text` (a `text()` string as the game receives it;
/// ruby markup is reduced to its base text first).
pub fn layout_rows(text: &str) -> Vec<String> {
    let text = visible_text(text);
    let mut rows = Vec::new();
    let mut first_glyph = true;
    for part in text.split('\n') {
        let mut row = String::new();
        let mut x: i64 = 0;
        for c in part.chars() {
            if x >= ROW_LIMIT_PX as i64 && !INDENT_ED.contains(c) {
                rows.push(std::mem::take(&mut row));
                x = 0;
            }
            if first_glyph && INDENT_ST.contains(c) {
                x -= glyph_width(c) as i64;
            }
            first_glyph = false;
            row.push(c);
            x += glyph_width(c) as i64;
        }
        rows.push(row);
    }
    rows
}

/// True when `text` needs more rows than the box shows.
pub fn overflows(text: &str, rows_per_box: usize) -> bool {
    layout_rows(text).len() > rows_per_box
}

/// Pixel width of `text` laid on one row (no wrapping).
fn width(text: &str) -> u32 {
    text.chars().map(glyph_width).sum()
}

/// English pre-break: inserts `\n` at word boundaries so no row exceeds the engine's limit
/// (the engine itself would break mid-word). Existing `\n` are kept. A word longer than a
/// whole row is left for the engine to break.
pub fn break_at_words(text: &str) -> String {
    text.split('\n')
        .map(|paragraph| {
            let mut lines: Vec<String> = Vec::new();
            let mut line = String::new();
            for word in paragraph.split(' ') {
                let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if !line.is_empty() && !fits_one_row(&candidate) {
                    lines.push(std::mem::take(&mut line));
                    line = word.to_string();
                } else {
                    line = candidate;
                }
            }
            lines.push(line);
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The engine keeps `row` on one row: every glyph starts below the limit (hanging glyphs
/// may start at or past it).
fn fits_one_row(row: &str) -> bool {
    layout_rows(row).len() == 1
}

/// Split `text` into box-sized pieces of at most `rows_per_box` engine rows each, breaking
/// between words (after [`break_at_words`]). Each piece is one `text()` of a continuation box.
pub fn split_into_boxes(text: &str, rows_per_box: usize) -> Vec<String> {
    let rows: Vec<String> = break_at_words(text).split('\n').flat_map(layout_rows).collect();
    rows.chunks(rows_per_box.max(1)).map(|chunk| chunk.join("\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_safe_text_removes_command_brackets() {
        assert_eq!(message_safe_text("buff stats with BP (Bonus Points)."), "buff stats with BP [Bonus Points].");
        assert_eq!(message_safe_text("5 < 6 and 7 > 3"), "5 ＜ 6 and 7 ＞ 3");
        // Real ruby survives; a stray bracket next to it is still neutralised.
        assert_eq!(message_safe_text("<軍神,マーズテリア> (god)"), "<軍神,マーズテリア> [god]");
        assert_eq!(message_safe_text("a < b <c,d>"), "a ＜ b <c,d>");
        assert_eq!(message_safe_text("plain"), "plain");
    }

    #[test]
    fn widths_follow_is_hankaku() {
        assert_eq!(glyph_width('a'), 21);
        assert_eq!(glyph_width('é'), 21); // <= U+00FF
        assert_eq!(glyph_width('’'), 42); // curly quote: full width in Parente
        assert_eq!(glyph_width('…'), 42);
        assert_eq!(glyph_width('あ'), 42);
    }

    #[test]
    fn row_holds_52_half_or_26_full_width() {
        assert_eq!(layout_rows(&"a".repeat(52)).len(), 1);
        assert_eq!(layout_rows(&"a".repeat(53)), ["a".repeat(52), "a".to_string()]);
        assert_eq!(layout_rows(&"あ".repeat(26)).len(), 1);
        assert_eq!(layout_rows(&"あ".repeat(27)).len(), 2);
        // In-game calibration box 2: 60 full-width digits → 26 / 26 / 8.
        let rows = layout_rows(&"１".repeat(60));
        assert_eq!(rows.iter().map(|r| r.chars().count()).collect::<Vec<_>>(), [26, 26, 8]);
    }

    #[test]
    fn closing_punctuation_hangs_instead_of_wrapping() {
        let text = format!("{}。", "あ".repeat(26));
        assert_eq!(layout_rows(&text).len(), 1);
        let two = format!("{}」。", "あ".repeat(26)); // consecutive hanging glyphs stay too
        assert_eq!(layout_rows(&two).len(), 1);
    }

    #[test]
    fn opening_bracket_at_message_start_hangs_left() {
        // 「 + 26 glyphs fit one row; without the bracket 27 glyphs would wrap.
        let text = format!("「{}", "あ".repeat(26));
        assert_eq!(layout_rows(&text).len(), 1);
        assert_eq!(layout_rows(&format!("あ{}", "あ".repeat(26))).len(), 2);
    }

    #[test]
    fn ruby_takes_no_row_width() {
        assert_eq!(visible_text("<軍神,マーズテリア>に<寵,ちょう>"), "軍神に寵");
        assert_eq!(visible_text("a < b and c > d"), "a < b and c > d"); // not ruby
        assert_eq!(visible_text("broken <tag"), "broken <tag");
        let text = format!("{}<神格者,しんかくしゃ>", "あ".repeat(23));
        assert_eq!(layout_rows(&text).len(), 1); // 26 visible glyphs
    }

    #[test]
    fn newlines_start_rows() {
        assert_eq!(layout_rows("one\ntwo\nthree\nfour"), ["one", "two", "three", "four"]);
        assert!(overflows("one\ntwo\nthree\nfour", NATIVE_ROWS));
        assert!(!overflows("one\ntwo\nthree", NATIVE_ROWS));
    }

    #[test]
    fn english_breaks_at_words() {
        let text = "The quick brown fox jumps over the lazy dog while the cat watches from the old stone wall nearby.";
        let broken = break_at_words(text);
        for row in broken.split('\n') {
            assert!(width(row) <= ROW_LIMIT_PX, "{row:?} too wide");
            assert!(!row.starts_with(' ') && !row.ends_with(' '));
        }
        assert_eq!(broken.replace('\n', " "), text);
        assert_eq!(layout_rows(&broken).len(), broken.split('\n').count()); // engine adds no breaks
    }

    #[test]
    fn long_text_splits_into_three_row_boxes() {
        let text = "word ".repeat(80).trim_end().to_string(); // 399 half-width glyphs ≈ 8 rows
        let boxes = split_into_boxes(&text, NATIVE_ROWS);
        assert!(boxes.len() >= 3);
        for piece in &boxes {
            assert!(!overflows(piece, NATIVE_ROWS), "{piece:?}");
        }
        assert_eq!(boxes.join("\n").replace('\n', " "), text);
    }
}
