use crate::editor::extract::DialogueEntry;
use crate::project::types::GlossaryEntry;

/// Options that influence how Claude translates.
#[derive(Debug, Clone, Default)]
pub struct TranslationOptions {
    /// Block character cap (0 = disabled)
    pub block_char_cap: u32,
    /// Per-line character cap (0 = disabled)
    pub line_char_cap: u32,
    /// Whether to enforce limits (true) or just flag violations (false)
    pub enforce_char_limits: bool,
    /// Cross-bin context summary (narrative context from sibling BINs)
    pub cross_bin_context: Option<String>,
    /// When true, skip detailed notes and glossary_updates to reduce token usage
    pub minimal_notes: bool,
    /// When true, skip the previous-dialogue context window for faster, cheaper translations
    pub fast_mode: bool,
    /// When true, group dialog rows into blocks (bounded by wait-for-input) and
    /// translate each block in a single LLM call.
    pub block_mode: bool,
    /// Number of previous blocks to include as context. 0 = none. Forced to 0 in fast mode.
    pub context_blocks: u32,
    /// When true, only glossary entries whose Japanese term appears as a substring
    /// of the current entry/block text are included in the system prompt. Cuts
    /// per-call tokens when prompt caching is unavailable (non-Claude providers).
    pub scoped_glossary: bool,
}

/// Filter a glossary to only entries whose `japanese` field appears as a
/// substring in `text`. Used by scoped-glossary mode to keep per-call token
/// cost minimal on providers without prompt caching.
pub fn scope_glossary<'a>(glossary: &'a [GlossaryEntry], text: &str) -> Vec<&'a GlossaryEntry> {
    glossary
        .iter()
        .filter(|g| !g.japanese.is_empty() && text.contains(&g.japanese))
        .collect()
}

/// Build the system prompt for translation, including the glossary.
pub fn build_system_prompt(glossary: &[GlossaryEntry], options: &TranslationOptions) -> String {
    let mut prompt = String::from(
        r#"You are an expert translator specializing in Japanese visual novel scripts from the Eushully AGE engine. You translate Japanese dialogue and text to natural, fluent English.

IMPORTANT RULES:
- Maintain consistent character voice and tone throughout the script
- Preserve the emotional nuance and subtext of the original Japanese
- Handle Japanese-specific wordplay, puns, and cultural references thoughtfully
- When a show-text line includes a trailing `[reading: ...]` annotation, the reading is furigana folded in from a following display-furigana row — translate the base text naturally and do NOT repeat the reading in the output
- Keep honorifics only when they carry meaningful nuance (e.g., -sama for reverence)
- Translate names consistently using the glossary below
- For wordplay or jokes that don't translate directly, adapt them to work in English and explain the original in your notes
- The line is shown to you as `[label] Japanese text` — the `[label]` is an opcode hint (e.g. `[show-text]`, `[set-string]`). Do NOT include the `[label]` in your translation output.
- If the original Japanese text starts with `「` and/or ends with `」`, your translation MUST preserve those exact CJK quote brackets in the same positions. Do not substitute English quotes ("..."), curly quotes (“...”), or omit them.
- Use ASCII hyphens (`-`) only. Do NOT use em-dashes (—, U+2014) or en-dashes (–, U+2013) — the in-game renderer does not display them correctly. For dialogue trail-off or interruption use `--` or `...`.

You MUST respond with ONLY a JSON object in this exact format:
"#,
    );

    if options.minimal_notes {
        prompt.push_str(
            r#"{
  "translation": "The translated English text"
}

Do NOT include notes or glossary_updates — just the translation.

"#,
        );
    } else {
        prompt.push_str(
            r#"{
  "translation": "The translated English text",
  "notes": "Any translator notes about nuance, wordplay, or context (empty string if none)",
  "glossary_updates": []
}

The glossary_updates array should contain any new terms you encounter that should be tracked for consistency. Each entry should be:
{
  "japanese": "原文",
  "english": "Translation",
  "category": "character|place|term|nuance|joke",
  "notes": "Why this should be tracked"
}

"#,
        );
    }

    // Character limit instructions
    let has_limits = options.block_char_cap > 0 || options.line_char_cap > 0;
    if has_limits {
        if options.enforce_char_limits {
            prompt.push_str("CHARACTER LIMITS (you MUST stay within these):\n");
            if options.line_char_cap > 0 {
                prompt.push_str(&format!(
                    "- Each individual line must be at most {} characters.\n",
                    options.line_char_cap
                ));
            }
            if options.block_char_cap > 0 {
                prompt.push_str(&format!(
                    "- The total characters across all lines in a dialogue block must not exceed {}.\n",
                    options.block_char_cap
                ));
            }
            prompt.push_str("- If you cannot fit the full meaning within the limit, prioritize the most important information and note what was condensed.\n");
            prompt.push_str("- 「」 brackets do not count toward the character limit.\n\n");
        } else {
            prompt.push_str("CHARACTER LIMITS (for reference only — prioritize translation accuracy):\n");
            if options.line_char_cap > 0 {
                prompt.push_str(&format!(
                    "- Suggested per-line limit: {} characters.\n",
                    options.line_char_cap
                ));
            }
            if options.block_char_cap > 0 {
                prompt.push_str(&format!(
                    "- Suggested block limit: {} characters.\n",
                    options.block_char_cap
                ));
            }
            prompt.push_str("- Translate for accuracy and naturalness first. If your translation exceeds a limit, mention it in your notes (e.g., \"Exceeds line cap by 5 chars\").\n");
            prompt.push_str("- 「」 brackets do not count toward the character limit.\n\n");
        }
    }

    // Cross-bin narrative context
    if let Some(ref context) = options.cross_bin_context {
        prompt.push_str("NARRATIVE CONTEXT FROM OTHER SCENE FILES:\n");
        prompt.push_str(context);
        prompt.push_str("\n\n");
    }

    if !glossary.is_empty() {
        prompt.push_str("GLOSSARY (use these translations consistently):\n");
        for entry in glossary {
            prompt.push_str(&format!(
                "- {} → {} [{}]{}\n",
                entry.japanese,
                entry.english,
                entry.category,
                if entry.notes.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", entry.notes)
                }
            ));
        }
        prompt.push('\n');
    }

    prompt
}

/// Build the system prompt for optimization — rephrasing translations to fit character limits.
pub fn build_optimization_prompt(glossary: &[GlossaryEntry], minimal_notes: bool) -> String {
    let mut prompt = String::from(
        r#"You are an expert English editor specializing in visual novel translations. Your task is to rephrase existing English translations to fit within strict character limits while preserving as much meaning, tone, and nuance as possible.

IMPORTANT RULES:
- You MUST keep the translation within the specified character limit
- Preserve the speaker's voice and emotional tone
- Prioritize the most important information when condensing
- If meaning must be sacrificed for brevity, clearly document what was lost
- 「」 brackets do not count toward the character limit
- Do not change the translation if it already fits within the limit
- Maintain consistency with the glossary below

You MUST respond with ONLY a JSON object in this exact format:
"#,
    );

    if minimal_notes {
        prompt.push_str(
            r#"{
  "translation": "The optimized English text"
}

Do NOT include notes or glossary_updates — just the translation.

"#,
        );
    } else {
        prompt.push_str(
            r#"{
  "translation": "The optimized English text",
  "notes": "What was changed and any meaning/nuance that was condensed or lost",
  "glossary_updates": []
}

"#,
        );
    }

    if !glossary.is_empty() {
        prompt.push_str("GLOSSARY (use these translations consistently):\n");
        for entry in glossary {
            prompt.push_str(&format!(
                "- {} → {} [{}]{}\n",
                entry.japanese,
                entry.english,
                entry.category,
                if entry.notes.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", entry.notes)
                }
            ));
        }
        prompt.push('\n');
    }

    prompt
}

/// Build the user message for optimizing a single line to fit within character limits.
pub fn build_optimization_message(
    current_entry: &DialogueEntry,
    context_entries: &[DialogueEntry],
    line_char_cap: u32,
    block_char_cap: u32,
    current_line_length: u32,
    current_block_length: u32,
) -> String {
    let mut message = String::new();

    // Include context for understanding
    if !context_entries.is_empty() {
        message.push_str("SURROUNDING CONTEXT (for reference):\n");
        for entry in context_entries {
            let translated = if entry.translated_text.is_empty() {
                &entry.original_text
            } else {
                &entry.translated_text
            };
            message.push_str(&format!(
                "[{}] JP: {} → EN: {}\n",
                entry.label, entry.original_text, translated
            ));
        }
        message.push('\n');
    }

    message.push_str("OPTIMIZE THIS TRANSLATION:\n");
    message.push_str(&format!("Original Japanese: {}\n", current_entry.original_text));
    message.push_str(&format!("Current English: {}\n", current_entry.translated_text));
    message.push_str(&format!("Current character count: {}\n", current_line_length));

    if line_char_cap > 0 {
        message.push_str(&format!("Line character limit: {} (over by {})\n", line_char_cap, current_line_length.saturating_sub(line_char_cap)));
    }
    if block_char_cap > 0 && current_block_length > block_char_cap {
        message.push_str(&format!("Block character limit: {} (block total is {}, over by {})\n", block_char_cap, current_block_length, current_block_length.saturating_sub(block_char_cap)));
    }

    message.push_str("\nRephrase the English translation to fit within the character limit(s). Preserve as much meaning and tone as possible.\n");

    message
}

/// Build the system prompt for block-mode translation. Like the per-row prompt
/// but expects a JSON array of N strings (one per slot) instead of a single string.
pub fn build_block_system_prompt(
    glossary: &[GlossaryEntry],
    options: &TranslationOptions,
) -> String {
    let mut prompt = String::from(
        r#"You are an expert translator specializing in Japanese visual novel scripts from the Eushully AGE engine. You translate Japanese dialogue and text into natural, fluent English. You will be given a single dialogue block split into N numbered slots. You translate the WHOLE BLOCK in one pass, returning one English string per slot in order.

IMPORTANT RULES:
- Maintain consistent character voice and tone across the block
- Preserve emotional nuance, subtext, and cultural references
- display-furigana slots are pre-filtered out before you see this block; when a show-text slot has a trailing `[reading: ...]` annotation, that reading came from a folded furigana row — translate the base text naturally and do NOT repeat the reading in the output
- Translate names consistently using the glossary
- Return EXACTLY N strings, in slot order
- Do NOT merge or reorder slots — slot 0 → translation 0, slot 1 → translation 1, etc.
- Do NOT include the slot's `[label]` prefix (e.g. `[set-string]`, `[show-text]`) in the translation output. The label is provided as a hint only — your translation must contain ONLY the English text.
- If a slot's Japanese starts with `「` and/or ends with `」`, the corresponding translation MUST preserve those exact CJK quote brackets in the same positions. Do not substitute English quotes or curly quotes.
- Use ASCII hyphens (`-`) only. Do NOT use em-dashes (—) or en-dashes (–); the in-game renderer does not display them correctly. For trail-off or interruption use `--` or `...`.

You MUST respond with ONLY a JSON object in this exact format:
"#,
    );

    if options.minimal_notes {
        prompt.push_str(
            r#"{
  "translations": ["slot 0 english", "slot 1 english", "..."]
}

Do NOT include notes or glossary_updates — just the translations array.

"#,
        );
    } else {
        prompt.push_str(
            r#"{
  "translations": ["slot 0 english", "slot 1 english", "..."],
  "notes": "Translator notes about wordplay or nuance for the whole block (empty string if none)",
  "glossary_updates": []
}

The glossary_updates array should contain any new terms encountered. Each entry:
{
  "japanese": "原文",
  "english": "Translation",
  "category": "character|place|term|nuance|joke",
  "notes": "Why this should be tracked"
}

"#,
        );
    }

    // Character limit instructions
    let has_limits = options.block_char_cap > 0 || options.line_char_cap > 0;
    if has_limits {
        if options.enforce_char_limits {
            prompt.push_str("CHARACTER LIMITS (you MUST stay within these):\n");
            if options.line_char_cap > 0 {
                prompt.push_str(&format!(
                    "- Each individual slot must be at most {} characters.\n",
                    options.line_char_cap
                ));
            }
            if options.block_char_cap > 0 {
                prompt.push_str(&format!(
                    "- The total characters across all slots must not exceed {}.\n",
                    options.block_char_cap
                ));
            }
            prompt.push_str("- 「」 brackets do not count toward the character limit.\n\n");
        } else {
            prompt.push_str("CHARACTER LIMITS (for reference only — prioritize accuracy):\n");
            if options.line_char_cap > 0 {
                prompt.push_str(&format!("- Suggested per-slot: {} chars.\n", options.line_char_cap));
            }
            if options.block_char_cap > 0 {
                prompt.push_str(&format!("- Suggested total: {} chars.\n", options.block_char_cap));
            }
            prompt.push_str("- 「」 brackets do not count toward the character limit.\n\n");
        }
    }

    if let Some(ref context) = options.cross_bin_context {
        prompt.push_str("NARRATIVE CONTEXT FROM OTHER SCENE FILES:\n");
        prompt.push_str(context);
        prompt.push_str("\n\n");
    }

    if !glossary.is_empty() {
        prompt.push_str("GLOSSARY (use these translations consistently):\n");
        for entry in glossary {
            prompt.push_str(&format!(
                "- {} → {} [{}]{}\n",
                entry.japanese,
                entry.english,
                entry.category,
                if entry.notes.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", entry.notes)
                }
            ));
        }
        prompt.push('\n');
    }

    prompt
}

/// Build the user message for block translation: lists slots and (optionally) prior blocks for context.
pub fn build_block_translation_message(
    slots: &[DialogueEntry],
    prior_blocks: &[Vec<DialogueEntry>],
) -> String {
    let mut message = String::new();

    if !prior_blocks.is_empty() {
        message.push_str("PREVIOUS BLOCKS (for context, do not translate these):\n");
        for (bi, block) in prior_blocks.iter().enumerate() {
            message.push_str(&format!("--- Block -{} ---\n", prior_blocks.len() - bi));
            for e in block {
                let translated = if e.translated_text.is_empty() {
                    &e.original_text
                } else {
                    &e.translated_text
                };
                message.push_str(&format!(
                    "  [{}] JP: {} → EN: {}\n",
                    e.label, e.original_text, translated
                ));
            }
        }
        message.push('\n');
    }

    message.push_str("TRANSLATE THIS BLOCK:\n\n");
    for (i, e) in slots.iter().enumerate() {
        message.push_str(&format!(
            "Slot {} [{}]\n  JP: {}\n",
            i, e.label, e.original_text
        ));
    }
    message.push_str(&format!(
        "\nReturn a JSON object with a \"translations\" array of exactly {} English strings, one per slot in order.\n",
        slots.len()
    ));

    message
}

/// Build a proofreading system prompt — focused on correcting an existing
/// English translation block rather than producing a new one.
pub fn build_proofread_prompt(
    glossary: &[GlossaryEntry],
    line_char_cap: u32,
    block_char_cap: u32,
    enforce_char_limits: bool,
) -> String {
    let mut prompt = String::from(
        r#"You are an expert proofreader for English visual novel translations from Japanese (Eushully AGE engine scripts). You will be given the original Japanese text and the current English translation for a single dialogue block, split into N slots. Your job is to correct grammar, spelling, awkward phrasing, and inconsistencies — NOT to retranslate from scratch. Preserve the speaker's voice and the existing translator's stylistic choices when they are correct.

RULES:
- Return EXACTLY N strings, one per slot, in the same order. Empty slots stay empty.
- If a slot is already correct, return it unchanged.
- Only fix actual problems: typos, grammar, punctuation, awkward English, glossary inconsistency.
- Do NOT change meaning unless the current translation is actually wrong relative to the Japanese.
- Do NOT add or remove dialogue lines.
- 「」 brackets do not count toward character limits.
- Use ASCII hyphens (`-`) only. Replace any em-dashes (—) or en-dashes (–) in the current translation with `-` or `--` — the in-game renderer does not display the wider dash glyphs correctly.

You MUST respond with ONLY a JSON object in this exact format:
{
  "translations": ["slot 0 corrected", "slot 1 corrected", "..."],
  "notes": "What you changed and why (empty string if nothing changed)"
}
"#,
    );

    let has_limits = block_char_cap > 0 || line_char_cap > 0;
    if has_limits {
        if enforce_char_limits {
            prompt.push_str("\nCHARACTER LIMITS (corrected text MUST stay within these):\n");
            if line_char_cap > 0 {
                prompt.push_str(&format!("- Each slot at most {} characters.\n", line_char_cap));
            }
            if block_char_cap > 0 {
                prompt.push_str(&format!("- Total across all slots at most {} characters.\n", block_char_cap));
            }
        } else {
            prompt.push_str("\nCHARACTER LIMITS (reference only — accuracy first):\n");
            if line_char_cap > 0 {
                prompt.push_str(&format!("- Suggested per-slot: {} chars.\n", line_char_cap));
            }
            if block_char_cap > 0 {
                prompt.push_str(&format!("- Suggested total: {} chars.\n", block_char_cap));
            }
        }
    }

    if !glossary.is_empty() {
        prompt.push_str("\nGLOSSARY (enforce consistency with these):\n");
        for entry in glossary {
            prompt.push_str(&format!(
                "- {} → {} [{}]\n",
                entry.japanese, entry.english, entry.category
            ));
        }
    }

    prompt
}

/// Build the user message listing the N slots to proofread.
pub fn build_proofread_message(entries: &[DialogueEntry]) -> String {
    let mut message = String::from("PROOFREAD THIS DIALOGUE BLOCK:\n\n");
    for (i, e) in entries.iter().enumerate() {
        message.push_str(&format!(
            "Slot {} [{}]\n  JP: {}\n  EN: {}\n\n",
            i, e.label, e.original_text, e.translated_text
        ));
    }
    message.push_str(&format!(
        "Return a JSON object with a \"translations\" array of exactly {} strings, one per slot in order.\n",
        entries.len()
    ));
    message
}

/// Build the user message for translating a single line, including context.
pub fn build_translation_message(
    current_entry: &DialogueEntry,
    context_entries: &[DialogueEntry],
) -> String {
    let mut message = String::new();

    // Include previous context for narrative continuity
    if !context_entries.is_empty() {
        message.push_str("PREVIOUS CONTEXT (for reference, do not translate these):\n");
        for entry in context_entries {
            let translated = if entry.translated_text.is_empty() {
                &entry.original_text
            } else {
                &entry.translated_text
            };
            message.push_str(&format!(
                "[{}] JP: {} → EN: {}\n",
                entry.label, entry.original_text, translated
            ));
        }
        message.push('\n');
    }

    // The line to translate
    message.push_str(&format!(
        "TRANSLATE THIS LINE:\n[{}] {}\n",
        current_entry.label, current_entry.original_text
    ));

    message
}
