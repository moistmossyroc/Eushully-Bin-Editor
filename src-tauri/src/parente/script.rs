//! Parser for the Lua-style scene scripts used by the Unity-era Eushully engine.
//!
//! Translatable content is every double-quoted string literal containing
//! non-ASCII (Japanese) bytes — `text(...)` dialogue and name-table strings —
//! while skipping ASCII asset refs (`"BGM004.ogg"`).
//!
//! The engine has built-in multi-language support: `text(jp,en,..)` and
//! `LangArray(jp,en,..)` take the Japanese in arg 1 and other languages in
//! later slots. We translate **non-destructively** by filling slot 2 (English)
//! and leaving the Japanese in slot 1, paired with patched `text`/`LangArray`
//! functions that prefer slot 2 when present (see `english_patch_*` in mod.rs).
//! Strings that aren't slot-1 of such a call (e.g. data tables) have no slot 2,
//! so they're translated in place.

/// Functions whose first string arg is Japanese and second is the English slot.
const LANG_FUNCS: [&str; 2] = ["text", "LangArray"];

/// One translatable string literal found in a script.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ParenteString {
    /// Sequential index across the whole file (stable id for round-trip).
    pub ordinal: usize,
    /// Inner (between-the-quotes) content of slot 1 (the Japanese / original).
    pub text: String,
    /// Existing translation read from slot 2 (English), empty if none.
    pub translated: String,
    /// Enclosing function name, e.g. `text`, or empty if none.
    pub label: String,
    /// Nearest preceding `::block_NNNNN::` label, if any — maps the row to the
    /// in-game text order so the editor list matches what the player sees.
    pub block: Option<String>,
    /// Speaker name from the nearest preceding `-- Name --` comment, if any.
    pub speaker: Option<String>,
}

/// Internal span used for replacement.
struct Span {
    /// Inner-content byte range of slot 1 (the original string).
    start: usize,
    len: usize,
    /// Inner-content byte range of slot 2 (the English slot), if the string is
    /// the first arg of a lang-capable call. When present, translations are
    /// written here (non-destructive); otherwise slot 1 is replaced in place.
    slot2: Option<(usize, usize)>,
    /// A `text()` message (rendered by the message window, which parses `<…>` commands).
    message: bool,
}

fn is_ident_byte(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => { out.push('\\'); out.push(other); }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Returns true if a `-- ... --` comment body looks like a speaker tag, and the
/// trimmed name if so.
fn speaker_from_comment(body: &str) -> Option<String> {
    let t = body.trim();
    let name = t.strip_suffix("--")?.trim();
    if name.is_empty() || name.chars().count() > 20 {
        return None;
    }
    let first = name.chars().next().unwrap();
    if "▼$-#".contains(first) {
        return None;
    }
    if name.is_ascii() {
        return None;
    }
    Some(name.to_string())
}

/// Read a string literal's inner byte range starting at the opening quote `qpos`.
/// Returns (inner_start, inner_end, after_close). `after_close` is just past the
/// closing quote (or EOF).
fn read_string(d: &[u8], qpos: usize) -> (usize, usize, usize) {
    let n = d.len();
    let inner = qpos + 1;
    let mut j = inner;
    while j < n {
        if d[j] == b'\\' {
            j += 2;
            continue;
        }
        if d[j] == b'"' {
            break;
        }
        j += 1;
    }
    (inner, j, (j + 1).min(n))
}

/// Skip ASCII whitespace from `p`, returning the new index.
fn skip_ws(d: &[u8], mut p: usize) -> usize {
    while p < d.len() && d[p].is_ascii_whitespace() {
        p += 1;
    }
    p
}

/// After slot 1 closes at `after1`, look for the 2nd string argument (slot 2):
/// `, "..."`. Returns (slot2_inner_span, slot2_content, index_past_slot2) if
/// found, else (None, "", after1).
fn lookahead_slot2(d: &[u8], after1: usize) -> (Option<(usize, usize)>, String, usize) {
    let mut p = skip_ws(d, after1);
    if p >= d.len() || d[p] != b',' {
        return (None, String::new(), after1);
    }
    p = skip_ws(d, p + 1);
    if p >= d.len() || d[p] != b'"' {
        return (None, String::new(), after1);
    }
    let (s, e, after2) = read_string(d, p);
    let content = unescape(&String::from_utf8_lossy(&d[s..e]));
    (Some((s, e - s)), content, after2)
}

/// Scan a script, returning every translatable string with its span(s).
fn scan(text: &str) -> (Vec<ParenteString>, Vec<Span>) {
    let d = text.as_bytes();
    let n = d.len();
    let mut i = 0usize;
    let mut out = Vec::new();
    let mut spans = Vec::new();
    let mut ordinal = 0usize;

    // (function name, arg index seen so far). arg index = commas seen at this depth.
    let mut call_stack: Vec<(String, u32)> = Vec::new();
    let mut last_ident: Option<String> = None;
    let mut speaker: Option<String> = None;
    let mut block: Option<String> = None;

    while i < n {
        let c = d[i];

        // Lua goto label `::block_00001::`.
        if c == b':' && i + 1 < n && d[i + 1] == b':' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < n && !(d[j] == b':' && d[j + 1] == b':') {
                j += 1;
            }
            let name = String::from_utf8_lossy(&d[start..j]).trim().to_string();
            if !name.is_empty() {
                block = Some(name);
            }
            i = j + 2;
            last_ident = None;
            continue;
        }

        // Line comment `-- ... <eol>`.
        if c == b'-' && i + 1 < n && d[i + 1] == b'-' {
            let body_start = i + 2;
            let mut j = body_start;
            while j < n && d[j] != b'\n' {
                j += 1;
            }
            let body = String::from_utf8_lossy(&d[body_start..j]);
            if let Some(name) = speaker_from_comment(&body) {
                speaker = Some(name);
            }
            i = j;
            last_ident = None;
            continue;
        }

        // String literal.
        if c == b'"' {
            let (inner_start, j, after) = read_string(d, i);
            let mut has_non_ascii = false;
            let mut has_ascii_letter = false;
            for &b in &d[inner_start..j] {
                if b >= 0x80 {
                    has_non_ascii = true;
                } else if b.is_ascii_alphabetic() {
                    has_ascii_letter = true;
                }
            }
            let func = call_stack.last().map(|(s, _)| s.as_str()).unwrap_or("");
            let arg_index = call_stack.last().map(|(_, a)| *a).unwrap_or(0);
            let is_lang_slot1 = arg_index == 0 && LANG_FUNCS.contains(&func);
            let is_dialogue_call = func == "text";
            let capture = has_non_ascii || (is_dialogue_call && j > inner_start && has_ascii_letter);

            if is_lang_slot1 && capture {
                // Slot 1 of a lang call → translation goes in slot 2.
                let (slot2, prefill, after2) = lookahead_slot2(d, after);
                out.push(ParenteString {
                    ordinal,
                    text: String::from_utf8_lossy(&d[inner_start..j]).into_owned(),
                    translated: prefill,
                    label: func.to_string(),
                    block: block.clone(),
                    speaker: speaker.clone(),
                });
                spans.push(Span { start: inner_start, len: j - inner_start, slot2, message: is_dialogue_call });
                ordinal += 1;
                // Consume through slot 2 so it isn't captured as its own entry.
                i = after2;
                last_ident = None;
                continue;
            } else if capture {
                // Non-lang string (data table, etc.) → in-place replacement.
                out.push(ParenteString {
                    ordinal,
                    text: String::from_utf8_lossy(&d[inner_start..j]).into_owned(),
                    translated: String::new(),
                    label: func.to_string(),
                    block: block.clone(),
                    speaker: speaker.clone(),
                });
                spans.push(Span { start: inner_start, len: j - inner_start, slot2: None, message: is_dialogue_call });
                ordinal += 1;
            }
            i = after;
            last_ident = None;
            continue;
        }

        // Identifier.
        if is_ident_byte(c) && !c.is_ascii_digit() {
            let start = i;
            while i < n && is_ident_byte(d[i]) {
                i += 1;
            }
            last_ident = Some(String::from_utf8_lossy(&d[start..i]).into_owned());
            continue;
        }

        match c {
            b'(' => {
                call_stack.push((last_ident.take().unwrap_or_default(), 0));
            }
            b')' => {
                let popped = call_stack.pop();
                if popped.as_ref().map(|(s, _)| s.as_str()) == Some("noname") {
                    speaker = None;
                }
                last_ident = None;
            }
            b',' => {
                if let Some(top) = call_stack.last_mut() {
                    top.1 += 1;
                }
                last_ident = None;
            }
            b'\n' => {
                last_ident = None;
            }
            _ => {
                if !c.is_ascii_whitespace() {
                    last_ident = None;
                }
            }
        }
        i += 1;
    }

    (out, spans)
}

/// Extract all translatable strings from a script (with any existing slot-2
/// English prefilled into `translated`).
pub fn parse_script(text: &str) -> Vec<ParenteString> {
    scan(text).0
}

/// Escape a translated string for embedding inside a Lua double-quoted string
/// literal (`text("...", "<here>", ...)`). Three classes of hazard, all of which
/// otherwise abort the scene with a SyntaxError in-game:
///  - `\` and `"` — standard Lua string escaping.
///  - CR/LF — a raw newline makes Lua report `unfinished string` (the string runs
///    off the end of the source line). Some CC translations carry line breaks.
///  - `--` — the AGE engine's Lua treats `--` as a comment EVEN INSIDE a string,
///    swallowing the rest of the line incl. the closing quote → `unfinished string`
///    (observed: sc0001 `"...my father too--was I..."`). Convert each `--` to an
///    em-dash `—` (renders the same, isn't a comment). Iterates chars (not bytes)
///    so multibyte UTF-8 and the `--` look-ahead are handled correctly.
fn escape_into(out: &mut Vec<u8>, s: &str) {
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend_from_slice(b"\\\\"),
            '"' => out.extend_from_slice(b"\\\""),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '-' if chars.peek() == Some(&'-') => {
                chars.next(); // consume the paired '-'
                out.extend_from_slice("—".as_bytes());
            }
            _ => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
}

/// The translation as written into the script: message text is made safe for the message
/// window's command parser (`message_layout::message_safe_text`).
fn export_text(translation: &str, message: bool) -> std::borrow::Cow<'_, str> {
    if message {
        std::borrow::Cow::Owned(crate::parente::message_layout::message_safe_text(translation))
    } else {
        std::borrow::Cow::Borrowed(translation)
    }
}

/// Apply translations IN PLACE (replace slot 1). Destructive — used for the
/// legacy/simple export. `translations` maps ordinal → English.
pub fn apply_translations(text: &str, translations: &[(usize, String)]) -> String {
    use std::collections::HashMap;
    let map: HashMap<usize, &str> = translations.iter().map(|(o, s)| (*o, s.as_str())).collect();
    let (_strings, spans) = scan(text);
    let d = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(d.len());
    let mut cursor = 0usize;
    for (ordinal, span) in spans.iter().enumerate() {
        if let Some(repl) = map.get(&ordinal) {
            if !repl.is_empty() {
                out.extend_from_slice(&d[cursor..span.start]);
                escape_into(&mut out, &export_text(repl, span.message));
                cursor = span.start + span.len;
            }
        }
    }
    out.extend_from_slice(&d[cursor..]);
    String::from_utf8_lossy(&out).into_owned()
}

/// Apply translations the NON-DESTRUCTIVE way: write English into slot 2 of
/// lang-capable calls (preserving the Japanese in slot 1); for strings without
/// a slot 2 (data tables), replace in place. Pairs with the patched
/// `text`/`LangArray` functions that read slot 2.
pub fn apply_english(text: &str, translations: &[(usize, String)]) -> String {
    use std::collections::HashMap;
    let map: HashMap<usize, &str> = translations.iter().map(|(o, s)| (*o, s.as_str())).collect();
    let (_strings, spans) = scan(text);
    let d = text.as_bytes();

    // Build (start, end, replacement) edits, then splice in order.
    struct Edit { start: usize, end: usize, repl: Vec<u8> }
    let mut edits: Vec<Edit> = Vec::new();
    for (ordinal, span) in spans.iter().enumerate() {
        let Some(en) = map.get(&ordinal) else { continue };
        if en.is_empty() {
            continue;
        }
        let mut repl = Vec::new();
        escape_into(&mut repl, &export_text(en, span.message));
        if let Some((s2, l2)) = span.slot2 {
            // Fill slot 2 (replace its inner content, keeping slot 1 intact).
            edits.push(Edit { start: s2, end: s2 + l2, repl });
        } else {
            // No slot 2 → replace slot 1 in place.
            edits.push(Edit { start: span.start, end: span.start + span.len, repl });
        }
    }
    edits.sort_by_key(|e| e.start);

    let mut out: Vec<u8> = Vec::with_capacity(d.len() + 64);
    let mut cursor = 0usize;
    for e in edits {
        if e.start < cursor {
            continue; // overlapping safety (shouldn't happen)
        }
        out.extend_from_slice(&d[cursor..e.start]);
        out.extend_from_slice(&e.repl);
        cursor = e.end;
    }
    out.extend_from_slice(&d[cursor..]);
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_message_text_has_no_command_brackets() {
        let script = "text(\"テスト\", \"\", \"\", \"\")\r\nLangArray(\"名前\", \"\")\r\n";
        let strings = parse_script(script);
        assert_eq!(strings.len(), 2);
        let out = apply_english(script, &[(0, "BP (Bonus Points) < 5".into()), (1, "Name (UI)".into())]);
        assert!(out.contains("BP [Bonus Points] ＜ 5"), "{out}");
        assert!(out.contains("Name (UI)"), "non-message strings keep their parentheses: {out}");
    }
}
