//! English word-wrap for auto-textbox splitting.
//!
//! Port of Eushlator's `correct_en_box`: greedy word-wrap that splits text into
//! multiple "boxes" when it overflows `lines_per_box`. Breaks at spaces; a word wider
//! than its row is hard-broken at the row's cap (the engine itself wraps per character
//! — docs/engine/age-sys.md §7b). Optional bottom-row cap per box ([`RowCaps`]).

use serde::{Deserialize, Serialize};

/// Result of wrapping a single English string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrapOutput {
    /// Outer = boxes, inner = lines within a box. Each line is a substring
    /// of the original text (words joined by spaces).
    pub boxes: Vec<Vec<String>>,
    /// Words that individually exceed `line_char_cap`. The wrap still places
    /// them on their own line — this list is purely for UI warnings.
    pub overflow_words: Vec<String>,
}

/// Measure display width. For v1 we use `chars().count()` — almost all
/// translated English is ASCII. Upgrade to wide-char measurement if a real
/// scene exposes the limitation.
fn measure_width(s: &str) -> u32 {
    s.chars().count() as u32
}

/// Per-row width limits for a wrap: every row holds `line` characters, except the
/// bottom row of each box (row `rows_per_box - 1`, counted from the box top) which holds
/// `bottom` when set — the slot some games share with UI art (Atelial's buttons).
#[derive(Debug, Clone, Copy)]
pub struct RowCaps {
    pub line: u32,
    /// 0 = no separate bottom-row limit.
    pub bottom: u32,
    pub rows_per_box: u32,
}

impl RowCaps {
    fn for_row(&self, row: usize) -> u32 {
        let is_bottom = self.rows_per_box > 0 && row % self.rows_per_box as usize == self.rows_per_box as usize - 1;
        if is_bottom && self.bottom > 0 { self.bottom.min(self.line) } else { self.line }
    }
}

/// Greedy word-wrap into display rows, each within its row cap (see [`RowCaps`]).
/// Breaks at spaces; a word wider than the row it starts on is cut at that row's cap
/// and continues on the next row (the engine itself wraps per character —
/// docs/engine/age-sys.md §7b). Words wider than `caps.line` are reported.
///
/// `「」` brackets are stripped before measuring and re-applied to the first/last row.
pub fn wrap_rows(text: &str, caps: RowCaps) -> WrapRows {
    if caps.line == 0 {
        return WrapRows { rows: vec![text.to_string()], overflow_words: Vec::new() };
    }
    let stripped = text.trim_start_matches('「').trim_end_matches('」');
    let words: Vec<&str> = stripped.split(' ').filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return WrapRows { rows: vec![text.to_string()], overflow_words: Vec::new() };
    }
    let overflow_words = words.iter().filter(|w| measure_width(w) > caps.line).map(|w| w.to_string()).collect();

    let mut rows: Vec<String> = Vec::new();
    for word in words {
        let joined = rows.last().map(|row| format!("{row} {word}"));
        if let Some(candidate) = joined.filter(|c| measure_width(c) <= caps.for_row(rows.len() - 1)) {
            *rows.last_mut().unwrap() = candidate;
            continue;
        }
        // Fresh row(s): cut the word at each row's cap until the rest fits.
        let mut rest: Vec<char> = word.chars().collect();
        loop {
            let cap = caps.for_row(rows.len()) as usize;
            if rest.len() <= cap {
                rows.push(rest.iter().collect());
                break;
            }
            rows.push(rest[..cap].iter().collect());
            rest.drain(..cap);
        }
    }

    if text.starts_with('「') {
        rows[0] = format!("「{}", rows[0]);
    }
    if text.ends_with('」') {
        let last = rows.len() - 1;
        rows[last] = format!("{}」", rows[last]);
    }
    WrapRows { rows, overflow_words }
}

/// [`wrap_rows`] laid out as BALANCED boxes: the text gets the fewest boxes the greedy
/// wrap needs, but each box takes an even share of the words (no box ending on a lone
/// word or line). A box's last row may be short; boxes are separated implicitly — a box
/// with fewer than `rows_per_box` rows is followed by a full-width row the bottom slot
/// can't take, so pagination (boxPlan.ts) recovers the same boxes. Falls back to the
/// greedy wrap when balancing can't fit (a word wider than a bottom row, or rounding).
pub fn wrap_rows_balanced(text: &str, caps: RowCaps) -> WrapRows {
    let greedy = wrap_rows(text, caps);
    let per_box = caps.rows_per_box as usize;
    if per_box == 0 || greedy.rows.len() <= per_box {
        return greedy;
    }
    let boxes = greedy.rows.len().div_ceil(per_box);
    let stripped = text.trim_start_matches('「').trim_end_matches('」');
    let words: Vec<&str> = stripped.split(' ').filter(|w| !w.is_empty()).collect();
    let narrowest_row = (0..per_box).map(|row| caps.for_row(row)).min().unwrap_or(caps.line);
    if words.iter().any(|w| measure_width(w) > narrowest_row) {
        return greedy;
    }

    let mut remaining: u32 = words.iter().map(|w| measure_width(w)).sum();
    let mut rows: Vec<String> = Vec::new();
    let mut next = 0usize;
    for box_index in 0..boxes {
        let last_box = box_index == boxes - 1;
        let budget = remaining.div_ceil((boxes - box_index) as u32);
        let mut box_rows: Vec<String> = Vec::new();
        let mut box_chars = 0u32;
        while next < words.len() {
            let word = words[next];
            let width = measure_width(word);
            // Stop this box once it holds its share (round to the nearest word).
            if !last_box && box_chars > 0 && box_chars + width > budget + width / 2 {
                break;
            }
            let fits_current = box_rows
                .last()
                .is_some_and(|row| measure_width(row) + 1 + width <= caps.for_row(box_rows.len() - 1));
            if fits_current {
                let row = box_rows.last_mut().unwrap();
                row.push(' ');
                row.push_str(word);
            } else if box_rows.len() < per_box {
                box_rows.push(word.to_string());
            } else {
                break; // box full
            }
            box_chars += width;
            next += 1;
        }
        remaining -= box_chars;
        rows.extend(box_rows);
    }
    if next < words.len() || rows.len() > boxes * per_box {
        return greedy;
    }

    if text.starts_with('「') {
        rows[0] = format!("「{}", rows[0]);
    }
    if text.ends_with('」') {
        let last = rows.len() - 1;
        rows[last] = format!("{}」", rows[last]);
    }
    WrapRows { rows, overflow_words: greedy.overflow_words }
}

/// Rows of [`wrap_rows`] plus the over-wide words (UI warnings only).
#[derive(Debug, Clone)]
pub struct WrapRows {
    pub rows: Vec<String>,
    pub overflow_words: Vec<String>,
}

/// Greedy word-wrap, ported from Eushlator's `correct_en_box`: [`wrap_rows`] at a
/// uniform `line_char_cap`, grouped into boxes of `lines_per_box` rows.
pub fn wrap_english(text: &str, line_char_cap: u32, lines_per_box: u32) -> WrapOutput {
    if line_char_cap == 0 || lines_per_box == 0 {
        return WrapOutput { boxes: vec![vec![text.to_string()]], overflow_words: Vec::new() };
    }
    let wrapped = wrap_rows(text, RowCaps { line: line_char_cap, bottom: 0, rows_per_box: lines_per_box });
    WrapOutput {
        boxes: wrapped.rows.chunks(lines_per_box as usize).map(|rows| rows.to_vec()).collect(),
        overflow_words: wrapped.overflow_words,
    }
}

/// The engine's own row breaking (docs/engine/age-sys.md §7b), from the metrics the
/// loader reports: each glyph advances `advance_*`; a row breaks before a glyph when
/// `cursor + check_*` would pass `row_right`. With `word_wrap` (the loader hook) an
/// ASCII word that would not fit starts the next row, and a space never starts one.
#[derive(Debug, Clone, Copy)]
pub struct EngineWrap<'a> {
    pub metrics: &'a crate::engine::text_metrics::EngineTextMetrics,
    pub word_wrap: bool,
}

/// Half-width glyph (1 cell): ASCII or half-width katakana; everything else is 2 cells.
pub fn is_half_width(c: char) -> bool {
    (c as u32) < 0x80 || ('\u{FF61}'..='\u{FF9F}').contains(&c)
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_graphic()
}

/// Rows the engine lays `text` out in.
pub fn engine_rows(text: &str, wrap: EngineWrap) -> Vec<String> {
    let m = wrap.metrics;
    let advance = |c: char| if is_half_width(c) { m.advance_half as i32 } else { m.advance_full as i32 };
    let check = |c: char| if is_half_width(c) { m.check_half as i32 } else { m.check_full as i32 };
    let slack = m.advance_half as i32 - m.check_half as i32;
    let chars: Vec<char> = text.chars().collect();
    let mut rows: Vec<String> = vec![String::new()];
    let mut x = m.row_left;
    for (i, &c) in chars.iter().enumerate() {
        let row_has_text = !rows.last().unwrap().is_empty();
        let new_row = if wrap.word_wrap && c == ' ' {
            false
        } else if wrap.word_wrap && is_word_char(c) && (i == 0 || chars[i - 1] == ' ') && x > m.row_left {
            let word: i32 = chars[i..].iter().take_while(|w| is_word_char(**w)).map(|w| advance(*w)).sum();
            x + word - slack > m.row_right
        } else {
            row_has_text && x + check(c) > m.row_right
        };
        if new_row {
            rows.push(String::new());
            x = m.row_left;
        }
        rows.last_mut().unwrap().push(c);
        x += advance(c);
    }
    rows
}

/// Width of a row in half-width cells (half = 1, full = 2), trailing spaces ignored.
pub fn row_cells(row: &str) -> u32 {
    row.trim_end().chars().map(|c| if is_half_width(c) { 1 } else { 2 }).sum()
}

/// Whether `text` fits one box the engine wraps itself: at most `rows_per_box` rows, and
/// a full box's bottom row within `bottom_cap` cells (0 = no cap; the UI-art slot).
pub fn engine_box_fits(text: &str, wrap: EngineWrap, rows_per_box: usize, bottom_cap: u32) -> bool {
    let rows = engine_rows(text, wrap);
    rows.len() <= rows_per_box && (bottom_cap == 0 || rows.len() < rows_per_box || row_cells(&rows[rows_per_box - 1]) <= bottom_cap)
}

/// Split `text` at word boundaries into the fewest boxes the engine can show (each box
/// wrapped by the engine itself), balanced so every box takes an even share of the
/// words — no box ends on a lone word. Falls back to the greedy fill when balancing can't
/// fit; a single word too long for a box becomes its own box.
pub fn engine_boxes(text: &str, wrap: EngineWrap, rows_per_box: usize, bottom_cap: u32) -> Vec<String> {
    let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
    if words.is_empty() || rows_per_box == 0 || engine_box_fits(text, wrap, rows_per_box, bottom_cap) {
        return vec![text.to_string()];
    }
    let fits = |words: &[&str]| engine_box_fits(&words.join(" "), wrap, rows_per_box, bottom_cap);
    // Greedy: as many words per box as fit.
    let greedy = {
        let mut boxes: Vec<String> = Vec::new();
        let mut start = 0;
        while start < words.len() {
            let mut end = start + 1;
            while end < words.len() && fits(&words[start..=end]) {
                end += 1;
            }
            boxes.push(words[start..end].join(" "));
            start = end;
        }
        boxes
    };
    // Balanced: each box takes about remaining/boxes_left characters (nearest word).
    let count = greedy.len();
    let mut boxes: Vec<String> = Vec::new();
    let mut start = 0;
    for index in 0..count {
        let remaining: usize = words[start..].iter().map(|w| w.chars().count() + 1).sum();
        let budget = remaining.div_ceil(count - index);
        let mut end = start + 1;
        let mut used = words[start].chars().count() + 1;
        while end < words.len() && index + 1 < count {
            let next = words[end].chars().count() + 1;
            if used + next > budget + next / 2 || !fits(&words[start..=end]) {
                break;
            }
            used += next;
            end += 1;
        }
        if index + 1 == count {
            end = words.len();
        }
        if !fits(&words[start..end]) && end - start > 1 {
            return greedy;
        }
        boxes.push(words[start..end].join(" "));
        start = end;
        if start >= words.len() {
            break;
        }
    }
    if start < words.len() || boxes.len() > count {
        return greedy;
    }
    boxes
}

/// Join a box's lines into a single newline-separated string, matching
/// Eushlator's output format where each box is `lines.join("\n")`.
pub fn box_to_string(lines: &[String]) -> String {
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atelial_metrics() -> crate::engine::text_metrics::EngineTextMetrics {
        crate::engine::text_metrics::EngineTextMetrics {
            face: "MS Gothic".into(),
            font_height: 22,
            font_width: 11,
            weight: 700,
            advance_half: 12,
            advance_full: 22,
            check_half: 11,
            check_full: 22,
            row_left: 78,
            row_right: 788,
        }
    }

    #[test]
    fn engine_rows_match_the_game() {
        let metrics = atelial_metrics();
        let native = EngineWrap { metrics: &metrics, word_wrap: false };
        let word = EngineWrap { metrics: &metrics, word_wrap: true };
        // In-game 2026-11-12 (#54 native, #60 word wrap).
        let text = "The wind carried a faint scent of something familiar, though I couldn't quite place where I had smelled it before. Strange.";
        assert_eq!(engine_rows(text, native)[0], "The wind carried a faint scent of something familiar, thoug");
        let rows = engine_rows(text, word);
        assert_eq!(rows, vec![
            "The wind carried a faint scent of something familiar, ",
            "though I couldn't quite place where I had smelled it ",
            "before. Strange.",
        ]);
        let text = "I stepped forward carefully, keeping my eyes on the shimmering boundary that separated the workshop from the rest of the world.";
        assert_eq!(engine_rows(text, word), vec![
            "I stepped forward carefully, keeping my eyes on the ",
            "shimmering boundary that separated the workshop from the ",
            "rest of the world.",
        ]);
        // Over-long word: per-character break, then words again (#63).
        let text = "Supercalifragilisticexpialidociousandthensomewordsthatkeepgoingwithoutanyspaces at all, followed by normal words.";
        assert_eq!(engine_rows(text, word)[0], "Supercalifragilisticexpialidociousandthensomewordsthatkeepg");
        // Japanese: 32 full-width per row (#64).
        let text = "「何故こうもクリエイターが集まって……異変に呼び寄せられたとでもいうの……！？」";
        assert_eq!(engine_rows(text, word)[0].chars().count(), 32);
    }

    #[test]
    fn engine_boxes_are_balanced_and_fit() {
        let metrics = atelial_metrics();
        let word = EngineWrap { metrics: &metrics, word_wrap: true };
        let text = "The wind carried a faint scent of something familiar, though I couldn't quite place where I had smelled it before. ".repeat(3);
        let boxes = engine_boxes(text.trim(), word, 3, 35);
        assert!(boxes.len() >= 2);
        assert_eq!(boxes.join(" "), text.trim(), "words preserved in order");
        for b in &boxes {
            assert!(engine_box_fits(b, word, 3, 35), "box fits: {b}");
        }
        let lengths: Vec<usize> = boxes.iter().map(|b| b.len()).collect();
        let (min, max) = (lengths.iter().min().unwrap(), lengths.iter().max().unwrap());
        assert!(max - min < 60, "balanced: {lengths:?}");
        // A box that fits stays whole.
        assert_eq!(engine_boxes("Short line.", word, 3, 35), vec!["Short line."]);
    }

    #[test]
    fn test_overlong_word_hard_breaks() {
        // The user's in-game test: 168 chars without spaces at 50/line.
        let text = "test".repeat(42);
        let out = wrap_english(&text, 50, 3);
        let widths: Vec<Vec<usize>> = out.boxes.iter().map(|b| b.iter().map(|l| l.len()).collect()).collect();
        assert_eq!(widths, vec![vec![50, 50, 50], vec![18]]);
        assert_eq!(out.boxes.concat().concat(), text, "no characters added or lost");
        assert_eq!(out.overflow_words, vec![text.clone()]);
        // A long word between normal words: no space inserted inside it.
        let out = wrap_english("go abcdefghijkl no", 5, 9);
        assert_eq!(out.boxes[0], vec!["go", "abcde", "fghij", "kl no"]);
    }

    #[test]
    fn test_bottom_row_cap() {
        // The user's re-flow: 205 chars, 63/row, Atelial bottom row 35 → box 1 = 63/63/35.
        let text = "test".repeat(51) + "t";
        let rows = wrap_rows(&text, RowCaps { line: 63, bottom: 35, rows_per_box: 3 }).rows;
        assert_eq!(rows.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![63, 63, 35, 44]);
        assert_eq!(rows.concat(), text);
        // Words: the bottom row only takes what fits in 35.
        let words = "aaaa ".repeat(40);
        let rows = wrap_rows(words.trim(), RowCaps { line: 20, bottom: 10, rows_per_box: 3 }).rows;
        assert!(rows.iter().enumerate().all(|(i, r)| r.len() <= if i % 3 == 2 { 10 } else { 20 }));
    }

    #[test]
    fn test_balanced_boxes() {
        let caps = RowCaps { line: 63, bottom: 35, rows_per_box: 3 };
        // ~205 chars of words: greedy would be 63/63/35 | short tail; balanced = two even boxes.
        let text = "test ".repeat(41);
        let rows = wrap_rows_balanced(text.trim(), caps).rows;
        let widths: Vec<usize> = rows.iter().map(|r| r.len()).collect();
        assert_eq!(rows.join(" "), text.trim(), "words preserved in order");
        // Two boxes: box 1 = rows until the first row that opens box 2.
        assert_eq!(widths, vec![59, 44, 59, 39]);
        // Bottom rows never exceed 35 (none used here) and no row exceeds 63.
        assert!(widths.iter().all(|w| *w <= 63));
        // Fits one box → unchanged greedy rows.
        let short = "word ".repeat(20);
        assert_eq!(wrap_rows_balanced(short.trim(), caps).rows, wrap_rows(short.trim(), caps).rows);
    }

    #[test]
    fn test_single_box_no_split() {
        let out = wrap_english("Hello world", 30, 3);
        assert_eq!(out.boxes.len(), 1);
        assert_eq!(out.boxes[0], vec!["Hello world"]);
        assert!(out.overflow_words.is_empty());
    }

    #[test]
    fn test_line_wrap_within_box() {
        let out = wrap_english("Hello my baby hello my honey", 15, 3);
        assert_eq!(out.boxes.len(), 1);
        assert_eq!(out.boxes[0].len(), 2);
        assert_eq!(out.boxes[0][0], "Hello my baby");
        assert_eq!(out.boxes[0][1], "hello my honey");
    }

    #[test]
    fn test_multi_box_split() {
        // 10 chars per line, 2 lines per box
        let out = wrap_english("Hello my baby hello my honey", 10, 2);
        // "Hello my" (8) → line 1
        // "baby hello" (10) → line 2 → box full
        // "my honey" (8) → box 2, line 1
        assert_eq!(out.boxes.len(), 2);
        assert_eq!(out.boxes[0], vec!["Hello my", "baby hello"]);
        assert_eq!(out.boxes[1], vec!["my honey"]);
    }

    #[test]
    fn test_overflow_word_recorded() {
        let out = wrap_english("short reallyreallylongword end", 10, 3);
        assert_eq!(out.overflow_words, vec!["reallyreallylongword"]);
        // The word is hard-broken at the cap; its characters all survive, in order.
        assert_eq!(out.boxes.concat(), vec!["short", "reallyreal", "lylongword", "end"]);
    }

    #[test]
    fn test_brackets_preserved() {
        let out = wrap_english("「Hello my baby hello my honey」", 10, 2);
        let first_line = &out.boxes[0][0];
        assert!(first_line.starts_with('「'), "first line should start with 「");
        let last_box = out.boxes.last().unwrap();
        let last_line = last_box.last().unwrap();
        assert!(last_line.ends_with('」'), "last line should end with 」");
    }

    #[test]
    fn test_eushlator_example() {
        // From Eushlator test: "Hello my baby, hello my honey."
        // max_chars_per_line=10, max_lines=2
        let out = wrap_english("Hello my baby, hello my honey.", 10, 2);
        assert!(out.boxes.len() >= 2, "Should split into at least 2 boxes");
        // Each box's lines should be ≤10 chars (except overflow words)
        for b in &out.boxes {
            for line in b {
                // Skip bracket-carrying lines for width check
                let stripped = line.trim_start_matches('「').trim_end_matches('」');
                if !out.overflow_words.iter().any(|w| stripped.contains(w.as_str())) {
                    assert!(
                        measure_width(stripped) <= 10,
                        "Line '{}' exceeds cap", line
                    );
                }
            }
        }
    }

    #[test]
    fn test_empty_text() {
        let out = wrap_english("", 30, 3);
        assert_eq!(out.boxes.len(), 1);
    }

    #[test]
    fn test_zero_caps_passthrough() {
        let out = wrap_english("Some text", 0, 0);
        assert_eq!(out.boxes.len(), 1);
        assert_eq!(out.boxes[0][0], "Some text");
    }
}
