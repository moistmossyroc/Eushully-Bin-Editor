//! CSV/JSON export-import for offline translation via Claude Code.
//!
//! Lets the user round-trip their dialogue through Claude Code (or any other
//! tool that can edit JSON files) instead of going through the paid API.
//!
//! Flow:
//! 1. Editor calls `export_for_translation` → writes `translation.json` and
//!    `PROMPT.md` to a chosen directory.
//! 2. User runs Claude Code in that directory with the PROMPT.md instructions.
//!    Claude Code edits `translation.json` in place, filling `translated` fields.
//! 3. Editor calls `import_translations` → reads the edited JSON, returns
//!    the merged entries + a stats summary.

use serde::{Deserialize, Serialize};

fn is_zero_u32(n: &u32) -> bool { *n == 0 }

/// One row of dialogue in the exchange file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationRow {
    /// Stable identifier — matches DialogueEntry.instruction_index.
    /// For grouped rows, this is the FIRST member's id (canonical anchor).
    /// Negative values mean visually-inserted rows.
    pub id: i64,
    /// Opcode label (e.g. "show-text", "display-furigana")
    pub label: String,
    /// Speaker name if resolvable, else null
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    /// Original Japanese text. For grouped rows, this is the concatenated
    /// JP across all line-group members (between end-text-line opcodes).
    pub original: String,
    /// Translated English text — empty on export, filled on import.
    /// For grouped rows, this is the translation of the WHOLE line group;
    /// import distributes it to the first member and empties the rest.
    pub translated: String,
    /// Optional translator notes (Claude can fill these). Omitted when empty
    /// to save tokens — `t.entries(data)[i].get("notes", "")` works fine.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// When the row represents a multi-entry line group, this lists every
    /// member's instruction_index in visual order. Empty/absent for
    /// single-entry rows (legacy + non-grouped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_member_ids: Vec<i64>,
    /// Sequential dialogue block this row belongs to. Bounded by
    /// `wait-for-input` opcodes in the source BIN. Rows in the same block
    /// share `block_id` so translation can be batched and conversational
    /// context (character voice, mood) is preserved per block. Omitted when
    /// zero (the default) to trim noise.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub block_id: u32,
    /// When set, this row's `original` was identical to another row's
    /// `original` (the canonical row at `duplicate_of`). The translation
    /// only needs to happen once at the canonical; on import the result
    /// is copied to every duplicate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<i64>,
    /// Up to N previous translatable lines as quick context, so each row is
    /// self-contained enough for parallel translation sessions (where no
    /// global conversation context is available). Each snippet is
    /// `[speaker] original` or `[speaker] translated` (translated preferred
    /// when available, original as fallback).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_before: Vec<String>,
    /// Cross-folder duplicate marker. When the multi-tab export sees the
    /// same `original` text in two different sibling folders, the second
    /// occurrence gets this field pointing back at the canonical folder/id.
    /// `translate.py` skips these (no Claude tokens spent); the editor's
    /// import step fills the translation by reading the canonical folder's
    /// mapping.json. None for canonical rows and for in-folder duplicates
    /// (those use `duplicate_of` and stay within one translation.json).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_duplicate_of: Option<ExternalDuplicateRef>,
}

/// Pointer to the canonical row in a sibling export folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalDuplicateRef {
    /// Subfolder name (basename) containing the canonical row.
    pub folder: String,
    /// `id` of the canonical row inside that folder's translation.json.
    pub id: i64,
}

/// Character limit metadata embedded in the export file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportCharLimits {
    /// Per-line character cap (0 = none)
    pub line: u32,
    /// Per-block character cap (0 = none)
    pub block: u32,
}

/// Box-budget model for the PROMPT, derived from the loaded BIN's split
/// capability (whether the engine script can grow into extra text boxes and
/// whether it can break lines within a box).
///
/// The point: the per-line cap is the *wrap width*, NOT the limit the
/// translator must squeeze a whole box into. When the BIN can add boxes, a long
/// translation is free — the tool wraps it and inserts continuation boxes. Only
/// when boxes can't be added does a hard total cap apply.
#[derive(Debug, Clone, Copy)]
pub struct PromptBudget {
    /// Per-line wrap width — the tool wraps to this. 0 = no wrapping / no limits
    /// section (translate freely).
    pub wrap_width: u32,
    /// Max display lines per box.
    pub max_lines: u32,
    /// True if the BIN can grow into additional text boxes (bridge available).
    pub can_add_boxes: bool,
    /// Hard total-character cap per entry. `None` = unlimited (boxes can be
    /// added). `Some(n)` = the entry must fit within `n` characters total
    /// (one box when line-breaks are available, otherwise one physical line).
    pub box_total: Option<u32>,
}

/// Top-level structure of `translation.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationExportFile {
    pub version: u32,
    pub source_bin: String,
    pub source_bin_sha256: String,
    pub char_limits: ExportCharLimits,
    /// Glossary is no longer written into translation.json (the canonical
    /// copy lives in PROMPT.md, rendered for Claude to read directly).
    /// Field kept for backward compat with old JSON files; serialized only
    /// when non-empty (which the export command now leaves empty by default).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<crate::project::types::GlossaryEntry>,
    pub entries: Vec<TranslationRow>,
}

/// Result of merging an imported translation file back into the editor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportMergeResult {
    /// Updated DialogueEntries with `translated_text` and `notes` filled in.
    pub entries: Vec<crate::editor::extract::DialogueEntry>,
    /// Number of entries that received a non-empty translation.
    pub applied: u32,
    /// Number of imported rows whose `id` had no matching entry in the project.
    pub unknown_ids: u32,
    /// Number of project entries that were left unchanged (no row in the import, or import had empty `translated`).
    pub unchanged: u32,
    /// Number of matched entries skipped because they already had a translation
    /// (only counted in fill_missing mode).
    pub skipped_existing: u32,
    /// True if the source BIN hash in the import file does not match the loaded BIN.
    pub hash_mismatch: bool,
    /// Source BIN filename recorded in the import file (used by the UI to show context in warnings).
    pub source_bin: String,
}

/// Render the PROMPT.md file that ships alongside translation.json.
///
/// The glossary is NOT embedded here anymore — it lives in `glossary.json` and
/// is auto-injected into each `block N` output by `translate.py` so only the
/// terms relevant to the current block enter Claude's context.
pub fn render_prompt(
    budget: &PromptBudget,
    glossary: &[crate::project::types::GlossaryEntry],
) -> String {
    let has_glossary = !glossary.is_empty();

    let mut s = String::from(
        r#"# Eushully BIN Translation — Instructions for Claude Code

You are translating a Japanese visual novel script from the Eushully AGE engine into natural, fluent English. The dialogue is in `translation.json` in this directory.

## Task

Open `translation.json` and fill in the `translated` field on every entry where it is currently the empty string. **Do not modify** any other fields (`id`, `label`, `speaker`, `original`, the structure itself). Edit the file in place.

You may optionally add a short translator note to the `notes` field for any entry where wordplay, cultural reference, or condensation needs explaining. Leave it empty otherwise.

## How rows are structured

Each entry in the `entries` array represents one **rendered line** in the game. In the source BIN, a single line can be composed of multiple opcodes (e.g. `show-text` + `display-furigana` + `show-text`) — they're concatenated here into one row whose `original` field holds the full Japanese for that line.

When a row has a `group_member_ids` field, that's the list of underlying entries the line spans. The editor will distribute your translation back: the first member gets the full English text, the rest are emptied automatically. **You do not need to do anything special** — just translate the row's `original` as one piece of natural English. Do NOT try to split it across multiple rows or duplicate fragments.

## Translation rules

- Maintain consistent character voice and tone throughout.
- Preserve emotional nuance and subtext from the Japanese.
- Handle wordplay, puns, and cultural references thoughtfully — adapt them to work in English and document the original in `notes` if needed.
- Keep honorifics (e.g. `-sama`, `-chan`) only when they carry meaningful nuance.
- Within a line's `original`, kanji that had furigana ruby readings in the original BIN are written as `kanji〔reading〕` (e.g. `燻〔くすぶ〕`). Treat these as kanji with a pronunciation hint — translate the kanji's meaning into English; the `〔reading〕` portion is just there to disambiguate readings for proper names or unusual kanji and should NOT be repeated in your translation.
- If the original starts with `「` and/or ends with `」`, your translation MUST preserve those exact CJK quote brackets in the same positions. Do not substitute English quotes, curly quotes, or omit them.
- Use ASCII hyphens (`-`) only. Do NOT use em-dashes (—) or en-dashes (–) — the in-game renderer does not display them correctly. For dialogue trail-off or interruption use `--` or `...`.
- Use the `speaker` field when present to inform voice — different characters speak differently.
"#,
    );

    if has_glossary {
        s.push_str("- A glossary of established term translations lives in `glossary.json`. **You don't need to load it manually** — `translate.py block N` automatically scans each block's text and prepends any glossary entries whose Japanese substring appears in the block, under a `\"glossary\"` field at the top of the output. Use those translations exactly as given for character names, place names, and recurring terms. Run `python translate.py glossary [search]` to inspect the full glossary or filter by substring if you ever need to.\n");
    } else {
        s.push_str("- No glossary is provided for this BIN. Be consistent with name romanizations and terminology choices you make as you go.\n");
    }

    s.push('\n');

    if budget.wrap_width > 0 {
        s.push_str("## Length & line breaks (must respect)\n\n");
        s.push_str(&format!(
            "- **Do NOT insert line breaks or newlines.** Write each entry as one continuous line. The tool automatically wraps your text to **{}** characters per line and lays it out in the game's text box (up to **{}** line(s) per box).\n",
            budget.wrap_width, budget.max_lines
        ));
        match budget.box_total {
            None => {
                // Bridge available — the tool adds continuation boxes for long text.
                s.push_str(
                    "- **No length limit.** Translate fully and naturally — if the English runs long, the tool automatically adds extra text boxes to fit it. Do NOT compress, truncate, or drop meaning to save space.\n",
                );
            }
            Some(total) if total > budget.wrap_width => {
                // No bridge, but lines can wrap → must fit one multi-line box.
                s.push_str(&format!(
                    "- **This entry cannot expand into extra text boxes.** It must fit a single box: at most **{}** characters total (about {} line(s) of {}). If the full meaning won't fit, prioritize what matters most and note what you condensed in `notes`.\n",
                    total, budget.max_lines, budget.wrap_width
                ));
            }
            Some(total) => {
                // No bridge and no line-break support → one physical line.
                s.push_str(&format!(
                    "- **This entry cannot wrap or expand.** Keep the translation to at most **{}** characters on a single line. Condense if needed and note it in `notes`.\n",
                    total
                ));
            }
        }
        s.push_str("- `「」` brackets do not count toward the limit.\n\n");
    }

    s.push_str(
        r#"## Workflow — STRICTLY one block per turn

Use `translate.py` (ships in this folder). The ONLY supported cycle, repeated per block:

1. **GET**: `python translate.py block N`  (one block only)
2. **TRANSLATE**: produce `{id: "english", ...}` for THIS block's entries
3. **WRITE**: write that mapping to `tl-out.json` (overwrite each block)
4. **APPLY**: `python translate.py apply tl-out.json`
5. Increment N, repeat from step 1.

### Things you MUST NOT do (each one causes failures)

- ❌ **NO batching `block` calls** — no `block 0 && block 1 && ...`, no `for i in 0 1 2; do ... done`, no dumping multiple blocks "to plan." Multi-block dumps crash on the 32K output token cap.
- ❌ **NO `python -c "..."` with Japanese text** — Windows cp932 console crashes. Use file I/O via the CLI commands.
- ❌ **NO wrapper scripts** like `dump_block.py` or `apply_translations.py`. The CLI commands below ARE the wrappers.
- ❌ **NO reading the whole `translation.json`** to "understand structure." The iterator gives you exactly what's needed.
- ❌ **NO translating `duplicate_of` rows** — their translation is copied from the canonical at import time. `untranslated_blocks()` already skips them.
- ❌ **NO PowerShell cmdlets in Bash** — your shell on Windows is Git Bash, NOT PowerShell. `Get-ChildItem`, `Select-Object`, `Where-Object`, etc. produce `command not found`. Use POSIX commands (`ls`, `find`, `grep`) — or better, don't enumerate files at all and trust `translate.py` to know what's there.
- ❌ **NO walking the filesystem** — `find`, `ls -R`, `Glob` over the project. The only files you should touch are `translate.py`, `translation.json`, `tl-out.json`, and PROMPT.md. `translate.py` is the canonical entry point; if you're tempted to list files, you're solving the wrong problem.

### Why this matters

- Block dump = small. Combined blocks = exceed 32K output cap.
- Per-block save = crash-safe; resume after rate limits anywhere.
- Small focused turns = better translation decisions.
- **Rare 30+ entry block**: split your output into ~20-entry chunks, applying between chunks.

### Correct sequence

```
> python translate.py blocks-pending
  block 0: 8 pending
  block 1: 1 pending
  ...

> python translate.py block 0
  {"glossary":[{"ja":"リゼル","en":"Liselle"}],"entries":[{"id":143,...}, ...]}

> Write tl-out.json with: {"143":"...","410":"...",...}

> python translate.py apply tl-out.json
  applied 8 translation(s); 8/622 translated (1%), 614 to translate

> python translate.py block 1
  ...repeat until status shows N/N (100%).
```

### CLI reference

- `python translate.py status` — progress summary
- `python translate.py blocks-pending` — list of `block_id: N pending` lines
- `python translate.py block <N>` — dump block N's entries (with auto-scoped glossary)
- `python translate.py apply <mapping.json>` — apply `{id: text}` mapping and save
- `python translate.py list-pending [N]` — show first N untranslated entries
- `python translate.py glossary [search]` — dump glossary, optionally filtered

### Field encoding & invariants

- `block_id` absent = 0. `notes` absent = `""`. `context_before` only in parallel-batch exports.
- **Never modify** `id`, `label`, `speaker`, `original`, `group_member_ids`, `block_id`, `duplicate_of`, or the JSON shape — only `translated` (and optionally `notes`).
"#,
    );

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_fixed_box_budget_and_glossary_reference() {
        use crate::project::types::GlossaryEntry;
        // No bridge, lines wrap → one box, total = 50*3 = 150.
        let budget = PromptBudget { wrap_width: 50, max_lines: 3, can_add_boxes: false, box_total: Some(150) };
        let glossary = vec![GlossaryEntry {
            japanese: "リル".into(),
            english: "Lyle".into(),
            category: "character".into(),
            notes: "Protagonist".into(),
        }];
        let s = render_prompt(&budget, &glossary);
        assert!(s.contains("wraps your text to **50**"));
        assert!(s.contains("at most **150** characters total"));
        assert!(s.contains("Do NOT insert line breaks"));
        // Glossary is no longer embedded — only a pointer to glossary.json + auto-injection
        assert!(s.contains("glossary.json"));
        assert!(s.contains("translate.py block N"));
        assert!(!s.contains("## Glossary"));
        assert!(!s.contains("リル"));
        assert!(s.contains("## Workflow"));
    }

    #[test]
    fn test_prompt_unlimited_when_boxes_can_be_added() {
        let budget = PromptBudget { wrap_width: 50, max_lines: 3, can_add_boxes: true, box_total: None };
        let s = render_prompt(&budget, &[]);
        assert!(s.contains("No length limit"));
        assert!(s.contains("adds extra text boxes"));
        assert!(s.contains("Do NOT insert line breaks"));
        assert!(!s.contains("characters total"));
    }

    #[test]
    fn test_prompt_renders_without_limits_or_glossary() {
        let budget = PromptBudget { wrap_width: 0, max_lines: 0, can_add_boxes: false, box_total: None };
        let s = render_prompt(&budget, &[]);
        assert!(!s.contains("Length & line breaks"));
        assert!(!s.contains("glossary.json"));
        assert!(s.contains("No glossary is provided"));
        // Cross-bin context section was removed — low signal vs token cost.
        assert!(!s.contains("Narrative context"));
    }

    #[test]
    fn test_export_file_roundtrip() {
        let file = TranslationExportFile {
            version: 1,
            source_bin: "TEST.BIN".into(),
            source_bin_sha256: "deadbeef".into(),
            char_limits: ExportCharLimits { line: 25, block: 75 },
            glossary: vec![],
            entries: vec![TranslationRow {
                id: 42,
                label: "show-text".into(),
                speaker: Some("Lyle".into()),
                original: "こんにちは".into(),
                translated: "".into(),
                notes: "".into(),
                group_member_ids: vec![],
                block_id: 0,
                duplicate_of: None,
                context_before: vec![],
                external_duplicate_of: None,
            }],
        };
        let json = serde_json::to_string_pretty(&file).unwrap();
        let parsed: TranslationExportFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].id, 42);
        assert_eq!(parsed.entries[0].speaker.as_deref(), Some("Lyle"));
    }
}
