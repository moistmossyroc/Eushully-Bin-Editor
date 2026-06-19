use crate::claude::client::{ClaudeClient, ClaudeError, Message};
use crate::claude::prompt::{
    build_system_prompt, build_translation_message,
    build_optimization_prompt, build_optimization_message,
    build_block_system_prompt, build_block_translation_message,
    TranslationOptions,
};
use crate::editor::extract::DialogueEntry;
use crate::project::types::GlossaryEntry;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

/// Translation response parsed from Claude's JSON output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationResult {
    pub translation: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub glossary_updates: Vec<GlossaryEntry>,
}

/// Block-mode translation result: N strings for N input slots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockTranslationResult {
    pub translations: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub glossary_updates: Vec<GlossaryEntry>,
}

/// Current state of the translation session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Idle,
    Running,
    Paused,
    Stopped,
}

/// Progress information for the translation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationProgress {
    pub status: SessionStatus,
    pub current_index: usize,
    pub total_entries: usize,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub last_error: Option<String>,
}

/// Manages the state of an ongoing translation session.
pub struct TranslationSession {
    client: ClaudeClient,
    status: SessionStatus,
    current_index: usize,
    total_input_tokens: u64,
    total_output_tokens: u64,
    /// Number of previous entries to include as context
    context_window_size: usize,
    /// Channel to signal pause/resume/stop from the UI
    control_rx: watch::Receiver<SessionControl>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionControl {
    Continue,
    Pause,
    Stop,
}

/// Clean Claude's response text for JSON parsing.
/// Strips markdown code fences, leading/trailing whitespace, and other common wrapping.
fn clean_json_response(text: &str) -> &str {
    let trimmed = text.trim();

    // Strip ```json ... ``` or ``` ... ```
    if let Some(rest) = trimmed.strip_prefix("```json") {
        if let Some(inner) = rest.strip_suffix("```") {
            return inner.trim();
        }
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        if let Some(inner) = rest.strip_suffix("```") {
            return inner.trim();
        }
    }

    trimmed
}

/// Strip a leading `[label]` prefix that the LLM may have copied from the
/// prompt format `Slot N [set-string]\n  JP: ...`. The prompt uses square-bracket
/// labels purely as hints, but some models echo them into the output.
fn strip_leading_label(text: &str) -> String {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            // Only strip if the bracketed content looks like an opcode label —
            // short, no whitespace, kebab/snake-case identifier. Avoids eating
            // legitimate translations that happen to start with [text].
            let label = &rest[..end];
            if !label.is_empty()
                && label.len() <= 32
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return rest[end + 1..].trim_start().to_string();
            }
        }
    }
    text.to_string()
}

/// Replace em-dash (—, U+2014) and en-dash (–, U+2013) with ASCII hyphens.
/// The in-game AGE renderer doesn't support the wider dash glyphs.
fn normalize_dashes(text: &str) -> String {
    text.replace('\u{2014}', "-").replace('\u{2013}', "-")
}

/// Preserve Japanese quote brackets `「` `」` from the original onto the
/// translation if the LLM dropped or substituted them. The brackets carry
/// dialogue framing and need to round-trip into the BIN output.
fn preserve_quote_brackets(original: &str, translated: &str) -> String {
    let mut out = translated.trim().to_string();

    // Normalize common substitutions the LLM might emit (English/CJK quotes).
    if original.starts_with('「') && !out.starts_with('「') {
        // Strip a leading English/curly quote if the LLM swapped one in.
        let stripped = out
            .strip_prefix('"')
            .or_else(|| out.strip_prefix('\u{201C}'))
            .or_else(|| out.strip_prefix('\u{2018}'))
            .unwrap_or(&out)
            .trim_start();
        out = format!("「{}", stripped);
    }
    if original.ends_with('」') && !out.ends_with('」') {
        let stripped = out
            .strip_suffix('"')
            .or_else(|| out.strip_suffix('\u{201D}'))
            .or_else(|| out.strip_suffix('\u{2019}'))
            .unwrap_or(&out)
            .trim_end();
        out = format!("{}」", stripped);
    }
    out
}

impl TranslationSession {
    pub fn new(
        client: ClaudeClient,
        control_rx: watch::Receiver<SessionControl>,
    ) -> Self {
        Self {
            client,
            status: SessionStatus::Idle,
            current_index: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            context_window_size: 30,
            control_rx,
        }
    }

    /// Build a tag like `[LLM: claude-sonnet-4-6]` or `[LLM: gpt-4o]` for notes.
    pub fn model_tag(&self) -> String {
        format!("[LLM: {}]", self.client.model())
    }

    /// Translate a single entry using the Claude API.
    pub async fn translate_single(
        &mut self,
        entry: &DialogueEntry,
        context: &[DialogueEntry],
        glossary: &[GlossaryEntry],
        options: &TranslationOptions,
    ) -> Result<TranslationResult, ClaudeError> {
        // Scoped glossary: filter to only entries whose JP appears in this row's
        // original text. Falls through to full glossary when scoping is off.
        let scoped: Vec<GlossaryEntry>;
        let active_glossary: &[GlossaryEntry] = if options.scoped_glossary {
            scoped = crate::claude::prompt::scope_glossary(glossary, &entry.original_text)
                .into_iter().cloned().collect();
            &scoped
        } else {
            glossary
        };
        let system_prompt = build_system_prompt(active_glossary, options);
        let user_message = build_translation_message(entry, context);

        let messages = vec![Message {
            role: "user".to_string(),
            content: user_message,
        }];

        let response = self.client.send_message(&system_prompt, messages, 1024).await?;

        self.total_input_tokens += response.usage.input_tokens;
        self.total_output_tokens += response.usage.output_tokens;

        let raw_text = ClaudeClient::extract_text(&response);
        let text = clean_json_response(&raw_text);

        // Parse the JSON response, falling back to raw text as translation
        let mut result: TranslationResult = match serde_json::from_str(text) {
            Ok(r) => r,
            Err(_) if !text.is_empty() => {
                TranslationResult {
                    translation: text.to_string(),
                    notes: format!("{} Response was not valid JSON — used raw text", self.model_tag()),
                    glossary_updates: vec![],
                }
            }
            Err(e) => return Err(ClaudeError::Parse(e)),
        };

        result.translation = strip_leading_label(&result.translation);
        result.translation = preserve_quote_brackets(&entry.original_text, &result.translation);
        result.translation = normalize_dashes(&result.translation);

        Ok(result)
    }

    /// Translate an entire dialog block in one call. Returns N strings for the N input slots.
    /// `prior_blocks` may contain a few preceding blocks for context (each as a Vec of entries).
    pub async fn translate_block(
        &mut self,
        slots: &[DialogueEntry],
        prior_blocks: &[Vec<DialogueEntry>],
        glossary: &[GlossaryEntry],
        options: &TranslationOptions,
    ) -> Result<BlockTranslationResult, ClaudeError> {
        // Scoped glossary: concatenate the block's originals and filter glossary
        // to only entries whose JP appears somewhere in that combined text.
        let scoped: Vec<GlossaryEntry>;
        let active_glossary: &[GlossaryEntry] = if options.scoped_glossary {
            let combined: String = slots.iter()
                .map(|e| e.original_text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            scoped = crate::claude::prompt::scope_glossary(glossary, &combined)
                .into_iter().cloned().collect();
            &scoped
        } else {
            glossary
        };
        let system_prompt = build_block_system_prompt(active_glossary, options);
        let user_message = build_block_translation_message(slots, prior_blocks);

        let messages = vec![Message {
            role: "user".to_string(),
            content: user_message,
        }];

        // Block calls return more output, so allow more tokens.
        let response = self.client.send_message(&system_prompt, messages, 4096).await?;

        self.total_input_tokens += response.usage.input_tokens;
        self.total_output_tokens += response.usage.output_tokens;

        let raw_text = ClaudeClient::extract_text(&response);
        let text = clean_json_response(&raw_text);

        let mut result: BlockTranslationResult = serde_json::from_str(text)
            .map_err(ClaudeError::Parse)?;

        for (i, s) in result.translations.iter_mut().enumerate() {
            *s = strip_leading_label(s);
            if let Some(slot) = slots.get(i) {
                *s = preserve_quote_brackets(&slot.original_text, s);
            }
            *s = normalize_dashes(s);
        }

        if result.translations.len() != slots.len() {
            return Err(ClaudeError::Api {
                status: 0,
                message: format!(
                    "Block translation slot count mismatch: got {}, expected {}",
                    result.translations.len(),
                    slots.len()
                ),
                retry_after_secs: None,
            });
        }

        Ok(result)
    }

    /// Optimize a single entry to fit within character limits.
    pub async fn optimize_single(
        &mut self,
        entry: &DialogueEntry,
        context: &[DialogueEntry],
        glossary: &[GlossaryEntry],
        line_char_cap: u32,
        block_char_cap: u32,
        current_line_length: u32,
        current_block_length: u32,
        minimal_notes: bool,
    ) -> Result<TranslationResult, ClaudeError> {
        let system_prompt = build_optimization_prompt(glossary, minimal_notes);
        let user_message = build_optimization_message(
            entry, context, line_char_cap, block_char_cap,
            current_line_length, current_block_length,
        );

        let messages = vec![Message {
            role: "user".to_string(),
            content: user_message,
        }];

        let response = self.client.send_message(&system_prompt, messages, 1024).await?;

        self.total_input_tokens += response.usage.input_tokens;
        self.total_output_tokens += response.usage.output_tokens;

        let raw_text = ClaudeClient::extract_text(&response);
        let text = clean_json_response(&raw_text);

        let mut result: TranslationResult = match serde_json::from_str(text) {
            Ok(r) => r,
            Err(_) if !text.is_empty() => {
                TranslationResult {
                    translation: text.to_string(),
                    notes: format!("{} Response was not valid JSON — used raw text", self.model_tag()),
                    glossary_updates: vec![],
                }
            }
            Err(e) => return Err(ClaudeError::Parse(e)),
        };

        result.translation = strip_leading_label(&result.translation);
        result.translation = preserve_quote_brackets(&entry.original_text, &result.translation);
        result.translation = normalize_dashes(&result.translation);

        Ok(result)
    }

    /// Run the translation workflow over all untranslated entries.
    pub async fn run(
        &mut self,
        entries: &mut [DialogueEntry],
        glossary: &mut Vec<GlossaryEntry>,
    ) -> Result<(), ClaudeError> {
        self.status = SessionStatus::Running;

        let translatable_indices: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_context_row && !e.deleted && e.translated_text.is_empty())
            .map(|(i, _)| i)
            .collect();

        for &entry_idx in &translatable_indices {
            // Check control signal (scope the borrow so it's dropped before changed())
            let control = { *self.control_rx.borrow() };
            match control {
                SessionControl::Stop => {
                    self.status = SessionStatus::Stopped;
                    return Ok(());
                }
                SessionControl::Pause => {
                    self.status = SessionStatus::Paused;
                    // Wait for resume or stop
                    loop {
                        if self.control_rx.changed().await.is_err() {
                            self.status = SessionStatus::Stopped;
                            return Ok(());
                        }
                        let ctrl = { *self.control_rx.borrow() };
                        match ctrl {
                            SessionControl::Continue => break,
                            SessionControl::Stop => {
                                self.status = SessionStatus::Stopped;
                                return Ok(());
                            }
                            SessionControl::Pause => continue,
                        }
                    }
                    self.status = SessionStatus::Running;
                }
                SessionControl::Continue => {}
            }

            // Build context window from previous entries
            let context_start = entry_idx.saturating_sub(self.context_window_size);
            let context: Vec<DialogueEntry> = entries[context_start..entry_idx]
                .iter()
                .filter(|e| !e.is_context_row && !e.deleted)
                .cloned()
                .collect();

            let options = TranslationOptions::default();
            let result = self.translate_single(&entries[entry_idx], &context, glossary, &options).await?;

            // Apply the translation
            entries[entry_idx].translated_text = result.translation;
            // Tag with model attribution, append any translator notes
            let tag = self.model_tag();
            entries[entry_idx].notes = if result.notes.is_empty() {
                format!("{} Translated automatically", tag)
            } else {
                format!("{} {}", tag, result.notes)
            };

            // Merge glossary updates
            for update in result.glossary_updates {
                if !glossary.iter().any(|g| g.japanese == update.japanese) {
                    glossary.push(update);
                }
            }

            self.current_index = entry_idx;
        }

        self.status = SessionStatus::Idle;
        Ok(())
    }

    pub fn progress(&self, total_entries: usize) -> TranslationProgress {
        TranslationProgress {
            status: self.status,
            current_index: self.current_index,
            total_entries,
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
            last_error: None,
        }
    }

    /// Build progress with explicit current/total and status string for event emission.
    pub fn progress_with(
        &self,
        _total_entries: usize,
        current: usize,
        total: usize,
        status: &str,
    ) -> TranslationProgress {
        TranslationProgress {
            status: match status {
                "Running" => SessionStatus::Running,
                "Paused" => SessionStatus::Paused,
                "Stopped" => SessionStatus::Stopped,
                _ => SessionStatus::Idle,
            },
            current_index: current,
            total_entries: total,
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
            last_error: None,
        }
    }

    /// Access the control receiver for checking signals.
    pub fn control_rx(&self) -> &watch::Receiver<SessionControl> {
        &self.control_rx
    }

    /// Mutable access to the control receiver for awaiting changes.
    pub fn control_rx_mut(&mut self) -> &mut watch::Receiver<SessionControl> {
        &mut self.control_rx
    }

    pub fn input_tokens(&self) -> u64 {
        self.total_input_tokens
    }

    pub fn output_tokens(&self) -> u64 {
        self.total_output_tokens
    }
}
