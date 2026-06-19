//! English word-wrap for auto-textbox splitting.
//!
//! Port of Eushlator's `correct_en_box`: greedy word-wrap that never breaks
//! mid-word and splits text into multiple "boxes" when it overflows
//! `lines_per_box`.

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

/// Greedy word-wrap, ported from Eushlator's `correct_en_box`.
///
/// Splits `text` into boxes of at most `lines_per_box` lines, each line at
/// most `line_char_cap` characters wide. Words are never broken; an
/// over-width word is placed on its own line as-is and recorded in
/// `overflow_words`.
///
/// `「」` brackets are stripped before measuring and re-applied to the
/// first/last box boundaries (matching the engine's dialogue rendering).
pub fn wrap_english(text: &str, line_char_cap: u32, lines_per_box: u32) -> WrapOutput {
    if line_char_cap == 0 || lines_per_box == 0 {
        return WrapOutput {
            boxes: vec![vec![text.to_string()]],
            overflow_words: Vec::new(),
        };
    }

    // Strip CJK quote brackets for measurement; we'll re-apply later.
    let stripped = text
        .trim_start_matches('「')
        .trim_end_matches('」');
    let had_open = text.starts_with('「');
    let had_close = text.ends_with('」');

    let words: Vec<&str> = stripped.split(' ').filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return WrapOutput {
            boxes: vec![vec![text.to_string()]],
            overflow_words: Vec::new(),
        };
    }

    let mut boxes: Vec<Vec<String>> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut overflow_words: Vec<String> = Vec::new();
    let lines_per_box = lines_per_box as usize;

    for word in &words {
        if measure_width(word) > line_char_cap {
            overflow_words.push(word.to_string());
        }

        if lines.is_empty() {
            lines.push(word.to_string());
            continue;
        }

        let candidate = format!("{} {}", lines.last().unwrap(), word);
        if measure_width(&candidate) <= line_char_cap {
            *lines.last_mut().unwrap() = candidate;
        } else if lines.len() < lines_per_box {
            lines.push(word.to_string());
        } else {
            boxes.push(std::mem::take(&mut lines));
            lines.push(word.to_string());
        }
    }
    if !lines.is_empty() {
        boxes.push(lines);
    }

    // Re-apply brackets to the first word of the first box and the last word
    // of the last box.
    if had_open {
        if let Some(first_box) = boxes.first_mut() {
            if let Some(first_line) = first_box.first_mut() {
                *first_line = format!("「{}", first_line);
            }
        }
    }
    if had_close {
        if let Some(last_box) = boxes.last_mut() {
            if let Some(last_line) = last_box.last_mut() {
                *last_line = format!("{}」", last_line);
            }
        }
    }

    WrapOutput {
        boxes,
        overflow_words,
    }
}

/// Join a box's lines into a single newline-separated string, matching
/// Eushlator's output format where each box is `lines.join("\n")`.
pub fn box_to_string(lines: &[String]) -> String {
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // The word still gets placed, just overflows
        assert!(out.boxes.iter().any(|b| b.iter().any(|l| l.contains("reallyreallylongword"))));
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
