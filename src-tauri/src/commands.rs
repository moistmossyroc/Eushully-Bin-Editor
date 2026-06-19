use crate::bin_format::parser::parse_bin;
use crate::bin_format::writer::write_bin;
use crate::claude::client::{ApiProvider, ClaudeClient};
use crate::claude::prompt::TranslationOptions;
use crate::claude::session::{SessionControl, TranslationSession};
use crate::editor::apply::apply_changes;
use crate::editor::characters::{extract_all_character_definitions, extract_ciinit_character_names, extract_referenced_speakers, detect_speaker_register, extract_row_speakers, detect_speaker_lookup_array, detect_speaker_id_variable, extract_referenced_speakers_sys5, extract_row_speakers_sys5, extract_referenced_speakers_sys5_legacy, extract_row_speakers_sys5_legacy, extract_referenced_speakers_sys3, extract_row_speakers_sys3};
use crate::editor::extract::{extract_dialogue, DialogueEntry};
use crate::project::io::{load_project as load_project_file, save_project as save_project_file};
use crate::project::types::{CharacterEntry, GlossaryEntry, InsertedInstruction, ProjectFile};
use crate::state::AppState;
use sha2::{Digest, Sha256};
use std::path::Path;
use tauri::{Emitter, Manager, State};
use tokio::sync::watch;

/// Sleep for `total_secs` while emitting periodic countdown progress events.
/// Returns false if the user requested Stop during the wait.
async fn wait_with_progress(
    app: &tauri::AppHandle,
    tab_id: &str,
    session: &TranslationSession,
    progress_idx: usize,
    total_to_translate: usize,
    total_secs: u64,
    message_template: &str,
) -> bool {
    let mut remaining = total_secs;
    loop {
        let _ = app.emit("translation-progress", serde_json::json!({
            "tab_id": tab_id,
            "status": "Running",
            "current_index": progress_idx,
            "total_entries": total_to_translate,
            "total_input_tokens": session.input_tokens(),
            "total_output_tokens": session.output_tokens(),
            "last_error": message_template.replace("{}", &remaining.to_string()),
        }));

        if remaining == 0 {
            return true;
        }
        let step = remaining.min(2);
        tokio::time::sleep(std::time::Duration::from_secs(step)).await;
        remaining = remaining.saturating_sub(step);

        // Check for stop during the wait
        let ctrl = { *session.control_rx().borrow() };
        if ctrl == SessionControl::Stop {
            return false;
        }
    }
}

/// Create a ClaudeClient from the current AppState provider configuration.
fn create_client(state: &AppState, model: String) -> Result<ClaudeClient, String> {
    let provider = state.api_provider.lock().unwrap().clone();
    let endpoint = state.custom_endpoint.lock().unwrap().clone();

    let api_key = state.api_key.lock().unwrap().clone().unwrap_or_default();

    // Anthropic requires a key; OpenAI-compatible providers may not (e.g. Ollama)
    if api_key.is_empty() && provider == ApiProvider::Anthropic {
        return Err("No API key configured".to_string());
    }

    Ok(ClaudeClient::from_config(api_key, model, provider, endpoint))
}

/// Result from loading a BIN file — entries plus metadata.
#[derive(serde::Serialize)]
pub struct LoadBinResult {
    pub entries: Vec<DialogueEntry>,
    pub sys_version: u8,
}

/// Load a BIN file and extract dialogue entries.
#[tauri::command]
pub async fn load_bin(
    tab_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<LoadBinResult, String> {
    let data = std::fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;

    let parsed = parse_bin(&data).map_err(|e| format!("Failed to parse BIN: {}", e))?;

    let entries = extract_dialogue(&parsed.instructions);
    let sys_version = parsed.header.sys_version;

    let bridge = crate::editor::bridge::detect_bridge_template(
        &parsed.instructions,
        sys_version as u32,
    );

    // Store in tab state
    state.ensure_tab(&tab_id);
    let mut tabs = state.tabs.lock().unwrap();
    let tab = tabs.get_mut(&tab_id).unwrap();
    tab.bin_path = Some(path);
    tab.raw_bin_data = Some(data);
    tab.bridge_template = Some(bridge);
    tab.parsed_bin = Some(parsed);

    Ok(LoadBinResult { entries, sys_version })
}

/// Load a project JSON file. Returns the project data.
/// The caller must also provide the BIN path via load_bin_for_project.
#[tauri::command]
pub async fn load_project(path: String) -> Result<ProjectFile, String> {
    let project =
        load_project_file(Path::new(&path)).map_err(|e| format!("Failed to load project: {}", e))?;
    Ok(project)
}

/// Load the BIN file associated with a project and verify its hash.
#[tauri::command]
pub async fn load_bin_for_project(
    tab_id: String,
    bin_path: String,
    expected_hash: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let data =
        std::fs::read(&bin_path).map_err(|e| format!("Failed to read BIN file: {}", e))?;

    // Verify SHA-256 hash
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let actual_hash = hex::encode(hasher.finalize());

    if actual_hash != expected_hash {
        return Ok(false);
    }

    let parsed = parse_bin(&data).map_err(|e| format!("Failed to parse BIN: {}", e))?;

    let bridge = crate::editor::bridge::detect_bridge_template(
        &parsed.instructions,
        parsed.header.sys_version as u32,
    );

    state.ensure_tab(&tab_id);
    let mut tabs = state.tabs.lock().unwrap();
    let tab = tabs.get_mut(&tab_id).unwrap();
    tab.bin_path = Some(bin_path);
    tab.raw_bin_data = Some(data);
    tab.bridge_template = Some(bridge);
    tab.parsed_bin = Some(parsed);

    Ok(true)
}

/// Save the current project state as a JSON file.
#[tauri::command]
pub async fn save_project(
    tab_id: String,
    path: String,
    entries: Vec<DialogueEntry>,
    glossary: Vec<GlossaryEntry>,
    characters: Vec<CharacterEntry>,
    inserted_instructions: Vec<InsertedInstruction>,
    model: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;

    let bin_path_str = tab.bin_path.as_ref().ok_or("No BIN file loaded")?;
    let raw = tab.raw_bin_data.as_ref().ok_or("No BIN data loaded")?;
    let parsed_bin = tab.parsed_bin.as_ref().ok_or("No parsed BIN data")?;

    // Compute hash
    let mut hasher = Sha256::new();
    hasher.update(raw);
    let hash = hex::encode(hasher.finalize());

    // Extract just the filename
    let filename = Path::new(bin_path_str)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let api_key_configured = state.api_key.lock().unwrap().is_some();

    let project = ProjectFile {
        version: 1,
        source_bin_filename: filename,
        source_bin_sha256: hash,
        is_ver5: parsed_bin.header.is_ver5,
        sys_version: parsed_bin.header.sys_version,
        entries,
        glossary,
        characters,
        inserted_instructions,
        settings: crate::project::types::ProjectSettings {
            claude_model: model,
            api_key_configured,
        },
    };

    save_project_file(Path::new(&path), &project)
        .map_err(|e| format!("Failed to save project: {}", e))?;

    Ok(())
}

/// Export a modified BIN file with applied translations.
#[tauri::command]
pub async fn export_bin(
    tab_id: String,
    output_path: String,
    entries: Vec<DialogueEntry>,
    inserted_instructions: Vec<InsertedInstruction>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    let parsed_bin = tab.parsed_bin.as_ref().ok_or("No BIN file loaded")?;

    // Apply changes to the instruction list
    let modified_instructions =
        apply_changes(&parsed_bin.instructions, &entries, &inserted_instructions);

    // Write the modified BIN
    let output_data = write_bin(&parsed_bin.header, &modified_instructions)
        .map_err(|e| format!("Failed to write BIN: {}", e))?;

    // Pre-write backup: if the output path already exists (i.e. we're about
    // to overwrite a real BIN, not creating a new one), copy it to a sibling
    // `<name>.bak` first. Only one rolling backup is kept — enough to recover
    // from an accidental bad export without bloating the user's folder.
    let out_path = std::path::Path::new(&output_path);
    if out_path.exists() {
        let backup_path = {
            let mut p = out_path.to_path_buf();
            let new_name = match p.file_name().and_then(|n| n.to_str()) {
                Some(n) => format!("{}.bak", n),
                None => return Err("Invalid output path".into()),
            };
            p.set_file_name(new_name);
            p
        };
        // Non-fatal: a failed backup shouldn't block a legitimate save (e.g.
        // permissions issue on the backup target). Log to stderr; continue.
        if let Err(e) = std::fs::copy(out_path, &backup_path) {
            eprintln!("export_bin: backup copy failed ({}): {}", backup_path.display(), e);
        }
    }

    std::fs::write(&output_path, &output_data)
        .map_err(|e| format!("Failed to write output file: {}", e))?;

    Ok(())
}

/// Set the API key for Claude translation.
#[tauri::command]
pub async fn set_api_key(key: String, state: State<'_, AppState>) -> Result<(), String> {
    // Store in memory
    *state.api_key.lock().unwrap() = Some(key.clone());

    // Try to store in OS keyring
    if let Ok(keyring_entry) = keyring::Entry::new("eushully-bin-editor", "claude-api-key") {
        let _ = keyring_entry.set_password(&key);
    }

    Ok(())
}

/// Get the stored API key.
#[tauri::command]
pub async fn get_api_key(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let key = state.api_key.lock().unwrap().clone();
    Ok(key)
}

/// Set the API provider configuration.
#[tauri::command]
pub async fn set_provider_config(
    provider: String,
    endpoint: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let parsed_provider = match provider.as_str() {
        "anthropic" => ApiProvider::Anthropic,
        "openai-compatible" => ApiProvider::OpenAiCompatible,
        _ => return Err(format!("Unknown provider: {}", provider)),
    };
    *state.api_provider.lock().unwrap() = parsed_provider;
    *state.custom_endpoint.lock().unwrap() = endpoint;
    Ok(())
}

/// Get the current API provider configuration.
#[tauri::command]
pub async fn get_provider_config(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let provider = state.api_provider.lock().unwrap().clone();
    let endpoint = state.custom_endpoint.lock().unwrap().clone();
    Ok(serde_json::json!({
        "provider": provider,
        "endpoint": endpoint,
    }))
}

/// Translate a single dialogue entry using the Claude API.
#[tauri::command]
pub async fn translate_single_entry(
    entry: DialogueEntry,
    context_entries: Vec<DialogueEntry>,
    glossary: Vec<GlossaryEntry>,
    model: String,
    block_char_cap: u32,
    line_char_cap: u32,
    enforce_char_limits: bool,
    cross_bin_context: Option<String>,
    minimal_notes: bool,
    fast_mode: bool,
    scoped_glossary: bool,
    state: State<'_, AppState>,
) -> Result<crate::claude::session::TranslationResult, String> {
    // Fast mode: skip context window and force-disable cross-bin context
    let (context_entries, cross_bin_context) = if fast_mode {
        (Vec::new(), None)
    } else {
        (context_entries, cross_bin_context)
    };

    let options = TranslationOptions {
        block_char_cap,
        line_char_cap,
        enforce_char_limits,
        cross_bin_context,
        minimal_notes,
        fast_mode,
        block_mode: false,
        context_blocks: 0,
        scoped_glossary,
    };

    let client = create_client(&state, model)?;
    let control_rx = watch::channel(SessionControl::Continue).1;
    let mut session = TranslationSession::new(client, control_rx);

    session
        .translate_single(&entry, &context_entries, &glossary, &options)
        .await
        .map_err(|e| format!("Translation error: {}", e))
}

/// Optimize a single translated entry to fit within character limits.
#[tauri::command]
pub async fn optimize_entry(
    entry: DialogueEntry,
    context_entries: Vec<DialogueEntry>,
    glossary: Vec<GlossaryEntry>,
    model: String,
    line_char_cap: u32,
    block_char_cap: u32,
    current_line_length: u32,
    current_block_length: u32,
    minimal_notes: bool,
    state: State<'_, AppState>,
) -> Result<crate::claude::session::TranslationResult, String> {
    let client = create_client(&state, model)?;
    let control_rx = watch::channel(SessionControl::Continue).1;
    let mut session = TranslationSession::new(client, control_rx);

    session
        .optimize_single(
            &entry, &context_entries, &glossary,
            line_char_cap, block_char_cap,
            current_line_length, current_block_length,
            minimal_notes,
        )
        .await
        .map_err(|e| format!("Optimization error: {}", e))
}

/// Result of a proofread call: one string per input slot, plus optional notes.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct ProofreadResult {
    pub translations: Vec<String>,
    pub notes: String,
}

/// Proofread an entire dialog block in one call. Returns N corrected strings,
/// one per input entry, in the same order.
#[tauri::command]
pub async fn proofread_block(
    entries: Vec<DialogueEntry>,
    glossary: Vec<GlossaryEntry>,
    model: String,
    line_char_cap: u32,
    block_char_cap: u32,
    enforce_char_limits: bool,
    state: State<'_, AppState>,
) -> Result<ProofreadResult, String> {
    if entries.is_empty() {
        return Err("No entries to proofread".to_string());
    }

    let client = create_client(&state, model)?;

    let system_prompt = crate::claude::prompt::build_proofread_prompt(
        &glossary,
        line_char_cap,
        block_char_cap,
        enforce_char_limits,
    );
    let user_message = crate::claude::prompt::build_proofread_message(&entries);

    let messages = vec![crate::claude::client::Message {
        role: "user".to_string(),
        content: user_message,
    }];

    let response = client
        .send_message(&system_prompt, messages, 4096)
        .await
        .map_err(|e| format!("Proofread error: {}", e))?;

    let raw_text = crate::claude::client::ClaudeClient::extract_text(&response);

    // Strip markdown code fences if present
    let json_text = raw_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let mut parsed: ProofreadResult = serde_json::from_str(json_text)
        .map_err(|e| format!("Failed to parse proofread response: {} | Raw: {}", e, raw_text))?;

    if parsed.translations.len() != entries.len() {
        return Err(format!(
            "Proofread returned {} slots, expected {}",
            parsed.translations.len(),
            entries.len()
        ));
    }

    // Normalize em/en-dashes to ASCII hyphens — the AGE renderer doesn't
    // display the wider dash glyphs correctly in-game.
    for s in parsed.translations.iter_mut() {
        *s = s.replace('\u{2014}', "-").replace('\u{2013}', "-");
    }

    Ok(parsed)
}

/// Start the batch translation workflow.
#[tauri::command]
pub async fn start_translation(
    tab_id: String,
    entries: Vec<DialogueEntry>,
    glossary: Vec<GlossaryEntry>,
    model: String,
    block_char_cap: u32,
    line_char_cap: u32,
    enforce_char_limits: bool,
    cross_bin_context: Option<String>,
    minimal_notes: bool,
    fast_mode: bool,
    block_mode: bool,
    context_blocks: u32,
    scoped_glossary: bool,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let client = create_client(&state, model)?;

    let (control_tx, control_rx) = watch::channel(SessionControl::Continue);

    // Store the control channel in the tab
    {
        state.ensure_tab(&tab_id);
        let mut tabs = state.tabs.lock().unwrap();
        let tab = tabs.get_mut(&tab_id).unwrap();
        tab.translation_control_tx = Some(control_tx);
    }

    // Fast mode: force-disable cross-bin context AND block context
    let cross_bin_context = if fast_mode { None } else { cross_bin_context };
    let context_blocks = if fast_mode { 0 } else { context_blocks };

    let options = TranslationOptions {
        block_char_cap,
        line_char_cap,
        enforce_char_limits,
        cross_bin_context,
        minimal_notes,
        fast_mode,
        block_mode,
        context_blocks,
        scoped_glossary,
    };

    let mut session = TranslationSession::new(client, control_rx);

    let mut entries = entries;
    let mut glossary = glossary;
    let tab_id_clone = tab_id.clone();

    // Run in a spawned task with per-entry event emission
    tokio::spawn(async move {
        let total = entries.len();

        // Pre-compute structural dialog blocks: runs of editable rows bounded by
        // wait-for-input (opcode 0x72). Deleted entries are skipped entirely.
        // Each block is a Vec<usize> of indices into `entries`.
        let mut blocks: Vec<Vec<usize>> = Vec::new();
        {
            let mut current: Vec<usize> = Vec::new();
            for (i, e) in entries.iter().enumerate() {
                if e.deleted { continue; }
                if e.opcode == 0x72 {
                    if !current.is_empty() {
                        blocks.push(std::mem::take(&mut current));
                    }
                    continue;
                }
                if e.is_context_row { continue; }
                current.push(i);
            }
            if !current.is_empty() {
                blocks.push(current);
            }
        }

        // Furigana fold pre-pass: when a show-text row is followed by one or
        // more display-furigana rows, fold their JP readings into the show-text's
        // original_text as a `[reading: ...]` annotation. The furigana rows
        // themselves are marked "absorbed" and skipped during translation — they
        // stay with empty translated_text, which the exporter writes through as
        // an empty string (no duplicated English in-game).
        let (folded_jp, absorbed_furigana): (
            std::collections::HashMap<usize, String>,
            std::collections::HashSet<usize>,
        ) = {
            let mut folded: std::collections::HashMap<usize, String> =
                std::collections::HashMap::new();
            let mut absorbed: std::collections::HashSet<usize> =
                std::collections::HashSet::new();
            for (idx, e) in entries.iter().enumerate() {
                if e.label != "show-text" || e.deleted || e.is_context_row {
                    continue;
                }
                let mut readings: Vec<String> = Vec::new();
                let mut j = idx + 1;
                while j < entries.len() {
                    let n = &entries[j];
                    if n.deleted || n.is_context_row {
                        j += 1;
                        continue;
                    }
                    if n.label == "display-furigana" {
                        if !n.original_text.is_empty() {
                            readings.push(n.original_text.clone());
                        }
                        absorbed.insert(j);
                        j += 1;
                        continue;
                    }
                    break;
                }
                if !readings.is_empty() {
                    let joined = readings.join(" / ");
                    folded.insert(
                        idx,
                        format!("{} [reading: {}]", e.original_text, joined),
                    );
                }
            }
            (folded, absorbed)
        };

        let total_to_translate: usize = blocks
            .iter()
            .flat_map(|b| b.iter())
            .filter(|&&i| {
                entries[i].translated_text.is_empty() && !absorbed_furigana.contains(&i)
            })
            .count();
        let total_blocks = blocks.len();
        let mut entries_done: usize = 0;

        // Helper to emit running progress with block + entry counters.
        // Flat shape so the frontend can assign it directly to translationProgress.
        macro_rules! emit_progress {
            ($status:expr, $bidx:expr) => {{
                let _ = app.emit("translation-progress", serde_json::json!({
                    "tab_id": tab_id_clone,
                    "status": $status,
                    "current_index": entries_done,
                    "total_entries": total_to_translate,
                    "total_input_tokens": session.input_tokens(),
                    "total_output_tokens": session.output_tokens(),
                    "last_error": serde_json::Value::Null,
                    "block_index": $bidx,
                    "total_blocks": total_blocks,
                }));
            }};
        }
        let _ = total; // total entries unused now (kept for parity)

        for (block_idx, block) in blocks.iter().enumerate() {
            // Check control signal at the block boundary
            let control = { *session.control_rx().borrow() };
            match control {
                SessionControl::Stop => {
                    let _ = app.emit("translation-complete", serde_json::json!({
                        "tab_id": tab_id_clone,
                        "total_entries": entries_done
                    }));
                    return;
                }
                SessionControl::Pause => {
                    emit_progress!("Paused", block_idx);
                    loop {
                        if session.control_rx_mut().changed().await.is_err() {
                            return;
                        }
                        let ctrl = { *session.control_rx().borrow() };
                        match ctrl {
                            SessionControl::Continue => {
                                emit_progress!("Running", block_idx);
                                break;
                            }
                            SessionControl::Stop => {
                                let _ = app.emit("translation-complete", serde_json::json!({
                                    "tab_id": tab_id_clone,
                                    "total_entries": entries_done
                                }));
                                return;
                            }
                            SessionControl::Pause => continue,
                        }
                    }
                }
                SessionControl::Continue => {}
            }

            // Indices in this block that still need translation. Absorbed
            // furigana rows are excluded — they stay empty and get skipped.
            let to_translate: Vec<usize> = block
                .iter()
                .copied()
                .filter(|&i| {
                    entries[i].translated_text.is_empty() && !absorbed_furigana.contains(&i)
                })
                .collect();
            if to_translate.is_empty() { continue; }

            // Effective block size excludes absorbed furigana so the block-call
            // gate (to_translate == effective size) still triggers correctly.
            let block_effective_size = block
                .iter()
                .filter(|&&i| !absorbed_furigana.contains(&i))
                .count();

            emit_progress!("Running", block_idx);

            // Build prior_blocks (last `context_blocks` blocks), full DialogueEntry
            // clones so the LLM gets both JP and any existing EN.
            let prior_count = options.context_blocks as usize;
            let prior_blocks: Vec<Vec<DialogueEntry>> = if prior_count > 0 && !fast_mode && block_idx > 0 {
                let prior_start = block_idx.saturating_sub(prior_count);
                blocks[prior_start..block_idx]
                    .iter()
                    .map(|b| b.iter().map(|&i| entries[i].clone()).collect())
                    .collect()
            } else {
                Vec::new()
            };

            // Block call qualifies when: enabled, >1 slot, ≤12 slots, and the
            // entire block is untranslated (so we can fill all slots in one shot).
            let try_block_call = block_mode
                && to_translate.len() > 1
                && to_translate.len() <= 12
                && to_translate.len() == block_effective_size;

            let max_transient_retries = 5u32;
            let mut block_succeeded = false;

            if try_block_call {
                // Apply furigana fold to show-text slots so Claude sees the
                // reading inline as `[reading: ...]` and skips re-emitting it.
                let slots: Vec<DialogueEntry> = to_translate
                    .iter()
                    .map(|&i| {
                        let mut e = entries[i].clone();
                        if let Some(folded) = folded_jp.get(&i) {
                            e.original_text = folded.clone();
                        }
                        e
                    })
                    .collect();

                let mut transient_attempt = 0u32;
                let mut rate_limit_attempt = 0u32;
                let block_result = loop {
                    match session.translate_block(&slots, &prior_blocks, &glossary, &options).await {
                        Ok(r) => break Ok(r),
                        Err(e) if e.is_auth_error() => break Err(e),
                        Err(e) if e.is_rate_limit() => {
                            rate_limit_attempt += 1;
                            let delay_secs = e.retry_after().unwrap_or_else(|| match rate_limit_attempt {
                                1 => 5, 2 => 10, 3 => 20, 4 => 30, _ => 60,
                            });
                            if !wait_with_progress(
                                &app, &tab_id_clone, &session, entries_done, total_to_translate,
                                delay_secs,
                                &format!("Rate limited — retrying block in {{}}s (attempt {})", rate_limit_attempt),
                            ).await {
                                let _ = app.emit("translation-complete", serde_json::json!({
                                    "tab_id": tab_id_clone, "total_entries": entries_done
                                }));
                                return;
                            }
                        }
                        Err(e) if e.is_retryable() && transient_attempt < max_transient_retries => {
                            transient_attempt += 1;
                            let delay_secs = 2u64.pow(transient_attempt);
                            if !wait_with_progress(
                                &app, &tab_id_clone, &session, entries_done, total_to_translate,
                                delay_secs,
                                &format!("Transient error — retrying block in {{}}s ({}/{})", transient_attempt, max_transient_retries),
                            ).await {
                                let _ = app.emit("translation-complete", serde_json::json!({
                                    "tab_id": tab_id_clone, "total_entries": entries_done
                                }));
                                return;
                            }
                        }
                        Err(e) => break Err(e),
                    }
                };

                match block_result {
                    Ok(br) => {
                        let tag = session.model_tag();
                        for (slot_idx, &entry_idx) in to_translate.iter().enumerate() {
                            entries[entry_idx].translated_text = br.translations[slot_idx].clone();
                            entries[entry_idx].notes = if br.notes.is_empty() {
                                format!("{} Translated automatically (block)", tag)
                            } else {
                                format!("{} {}", tag, br.notes)
                            };
                            let _ = app.emit("translation-entry-updated", serde_json::json!({
                                "tab_id": tab_id_clone,
                                "index": entry_idx,
                                "entry": entries[entry_idx],
                            }));
                            entries_done += 1;
                        }

                        for update in br.glossary_updates {
                            if let Some(existing) = glossary.iter().find(|g| g.japanese == update.japanese) {
                                if existing.english != update.english || existing.notes != update.notes {
                                    let _ = app.emit("glossary-review-pending", serde_json::json!({
                                        "tab_id": tab_id_clone,
                                        "existing": existing,
                                        "proposed": update,
                                        "source_entry_index": to_translate[0],
                                    }));
                                }
                            } else {
                                glossary.push(update.clone());
                                let _ = app.emit("glossary-new-term", serde_json::json!({
                                    "tab_id": tab_id_clone,
                                    "entry": update,
                                }));
                            }
                        }
                        block_succeeded = true;
                    }
                    Err(e) => {
                        // Block call failed (parse error, slot count mismatch, etc).
                        // Fall back to per-row for this block — emit a soft warning.
                        let _ = app.emit("translation-error", serde_json::json!({
                            "tab_id": tab_id_clone,
                            "error": format!("Block call failed, falling back to per-row: {}", e),
                        }));
                    }
                }
            }

            if block_succeeded { continue; }

            // Per-row path (either block disabled, didn't qualify, or fell back).
            for &entry_idx in &to_translate {
                // Mid-block control check
                let control = { *session.control_rx().borrow() };
                match control {
                    SessionControl::Stop => {
                        let _ = app.emit("translation-complete", serde_json::json!({
                            "tab_id": tab_id_clone, "total_entries": entries_done
                        }));
                        return;
                    }
                    SessionControl::Pause => {
                        emit_progress!("Paused", block_idx);
                        loop {
                            if session.control_rx_mut().changed().await.is_err() { return; }
                            let ctrl = { *session.control_rx().borrow() };
                            match ctrl {
                                SessionControl::Continue => {
                                    emit_progress!("Running", block_idx);
                                    break;
                                }
                                SessionControl::Stop => {
                                    let _ = app.emit("translation-complete", serde_json::json!({
                                        "tab_id": tab_id_clone, "total_entries": entries_done
                                    }));
                                    return;
                                }
                                SessionControl::Pause => continue,
                            }
                        }
                    }
                    SessionControl::Continue => {}
                }

                emit_progress!("Running", block_idx);

                // Per-row context: previous 30 entries window, skipped in fast mode.
                let context: Vec<DialogueEntry> = if fast_mode {
                    Vec::new()
                } else {
                    let context_start = entry_idx.saturating_sub(30);
                    entries[context_start..entry_idx]
                        .iter()
                        .filter(|e| !e.is_context_row && !e.deleted)
                        .cloned()
                        .collect()
                };

                // Apply furigana fold for show-text rows that absorbed a
                // following display-furigana reading.
                let folded_entry = folded_jp.get(&entry_idx).map(|folded| {
                    let mut c = entries[entry_idx].clone();
                    c.original_text = folded.clone();
                    c
                });
                let entry_for_call: &DialogueEntry = folded_entry.as_ref().unwrap_or(&entries[entry_idx]);

                let mut transient_attempt = 0u32;
                let mut rate_limit_attempt = 0u32;
                let result = loop {
                    match session.translate_single(entry_for_call, &context, &glossary, &options).await {
                        Ok(r) => break Ok(r),
                        Err(e) if e.is_auth_error() => break Err(e),
                        Err(e) if e.is_rate_limit() => {
                            rate_limit_attempt += 1;
                            let delay_secs = e.retry_after().unwrap_or_else(|| match rate_limit_attempt {
                                1 => 5, 2 => 10, 3 => 20, 4 => 30, _ => 60,
                            });
                            if !wait_with_progress(
                                &app, &tab_id_clone, &session, entries_done, total_to_translate,
                                delay_secs,
                                &format!("Rate limited — retrying in {{}}s (attempt {})", rate_limit_attempt),
                            ).await {
                                let _ = app.emit("translation-complete", serde_json::json!({
                                    "tab_id": tab_id_clone, "total_entries": entries_done
                                }));
                                return;
                            }
                        }
                        Err(e) if e.is_retryable() && transient_attempt < max_transient_retries => {
                            transient_attempt += 1;
                            let delay_secs = 2u64.pow(transient_attempt);
                            if !wait_with_progress(
                                &app, &tab_id_clone, &session, entries_done, total_to_translate,
                                delay_secs,
                                &format!("Transient error — retrying in {{}}s ({}/{})", transient_attempt, max_transient_retries),
                            ).await {
                                let _ = app.emit("translation-complete", serde_json::json!({
                                    "tab_id": tab_id_clone, "total_entries": entries_done
                                }));
                                return;
                            }
                        }
                        Err(e) => break Err(e),
                    }
                };

                match result {
                    Ok(result) => {
                        entries[entry_idx].translated_text = result.translation;
                        let tag = session.model_tag();
                        entries[entry_idx].notes = if result.notes.is_empty() {
                            format!("{} Translated automatically", tag)
                        } else {
                            format!("{} {}", tag, result.notes)
                        };

                        for update in result.glossary_updates {
                            if let Some(existing) = glossary.iter().find(|g| g.japanese == update.japanese) {
                                if existing.english != update.english || existing.notes != update.notes {
                                    let _ = app.emit("glossary-review-pending", serde_json::json!({
                                        "tab_id": tab_id_clone,
                                        "existing": existing,
                                        "proposed": update,
                                        "source_entry_index": entry_idx,
                                    }));
                                }
                            } else {
                                glossary.push(update.clone());
                                let _ = app.emit("glossary-new-term", serde_json::json!({
                                    "tab_id": tab_id_clone,
                                    "entry": update,
                                }));
                            }
                        }

                        let _ = app.emit("translation-entry-updated", serde_json::json!({
                            "tab_id": tab_id_clone,
                            "index": entry_idx,
                            "entry": entries[entry_idx],
                        }));
                        entries_done += 1;
                    }
                    Err(e) => {
                        let _ = app.emit("translation-error", serde_json::json!({
                            "tab_id": tab_id_clone,
                            "error": e.to_string()
                        }));
                    }
                }
            }
        }

        // Done
        let _ = app.emit("translation-complete", serde_json::json!({
            "tab_id": tab_id_clone,
            "total_entries": total_to_translate
        }));
        let _ = app.emit("glossary-updated", serde_json::json!({
            "tab_id": tab_id_clone,
            "glossary": glossary
        }));
    });

    Ok(())
}

/// Pause the ongoing translation for a tab.
#[tauri::command]
pub async fn pause_translation(tab_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let tabs = state.tabs.lock().unwrap();
    if let Some(tab) = tabs.get(&tab_id) {
        if let Some(tx) = tab.translation_control_tx.as_ref() {
            tx.send(SessionControl::Pause)
                .map_err(|_| "Translation session not active".to_string())?;
        }
    }
    Ok(())
}

/// Resume the paused translation for a tab.
#[tauri::command]
pub async fn resume_translation(tab_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let tabs = state.tabs.lock().unwrap();
    if let Some(tab) = tabs.get(&tab_id) {
        if let Some(tx) = tab.translation_control_tx.as_ref() {
            tx.send(SessionControl::Continue)
                .map_err(|_| "Translation session not active".to_string())?;
        }
    }
    Ok(())
}

/// Stop the ongoing translation for a tab.
#[tauri::command]
pub async fn stop_translation(tab_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let tabs = state.tabs.lock().unwrap();
    if let Some(tab) = tabs.get(&tab_id) {
        if let Some(tx) = tab.translation_control_tx.as_ref() {
            tx.send(SessionControl::Stop)
                .map_err(|_| "Translation session not active".to_string())?;
        }
    }
    Ok(())
}

/// Extract all referenced speaker pointers from the loaded BIN.
/// Returns CharacterEntry for each unique speaker — name is empty if not defined in this BIN.
#[tauri::command]
pub async fn get_referenced_speakers(
    tab_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<CharacterEntry>, String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    let parsed_bin = tab.parsed_bin.as_ref().ok_or("No BIN file loaded")?;

    let sys_version = parsed_bin.header.sys_version;

    // SYS3: inline literal names in show-text
    if sys_version <= 3 {
        return Ok(extract_referenced_speakers_sys3(&parsed_bin.instructions));
    }

    // SYS5: lookup-array character ID pattern (preferred)
    if sys_version >= 5 {
        if let Some((array_addr, _)) = detect_speaker_lookup_array(&parsed_bin.instructions) {
            return Ok(extract_referenced_speakers_sys5(&parsed_bin.instructions, array_addr));
        }
        // SYS5 legacy fallback
        if let Some(id_var) = detect_speaker_id_variable(&parsed_bin.instructions) {
            return Ok(extract_referenced_speakers_sys5_legacy(&parsed_bin.instructions, id_var));
        }
        return Ok(Vec::new());
    }

    // SYS4: set-string + show-text register reference
    let speakers = extract_referenced_speakers(&parsed_bin.instructions);
    if !speakers.is_empty() {
        return Ok(speakers);
    }

    Ok(Vec::new())
}

/// Extract speaker references for each dialogue entry from the loaded BIN.
#[tauri::command]
pub async fn get_row_speakers(
    tab_id: String,
    state: State<'_, AppState>,
) -> Result<std::collections::BTreeMap<usize, crate::editor::characters::SpeakerRef>, String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    let parsed_bin = tab.parsed_bin.as_ref().ok_or("No BIN file loaded")?;

    let sys_version = parsed_bin.header.sys_version;

    // SYS3: inline literal names
    if sys_version <= 3 {
        return Ok(extract_row_speakers_sys3(&parsed_bin.instructions));
    }

    // SYS5: lookup-array character ID pattern (preferred)
    if sys_version >= 5 {
        if let Some((array_addr, _)) = detect_speaker_lookup_array(&parsed_bin.instructions) {
            return Ok(extract_row_speakers_sys5(&parsed_bin.instructions, array_addr));
        }
        // SYS5 legacy fallback
        if let Some(id_var) = detect_speaker_id_variable(&parsed_bin.instructions) {
            return Ok(extract_row_speakers_sys5_legacy(&parsed_bin.instructions, id_var));
        }
        return Ok(std::collections::BTreeMap::new());
    }

    // SYS4: set-string + speaker register
    if let Some(speaker_reg) = detect_speaker_register(&parsed_bin.instructions) {
        return Ok(extract_row_speakers(&parsed_bin.instructions, speaker_reg));
    }

    Ok(std::collections::BTreeMap::new())
}

/// Scan a BIN file for character definitions (set-string pairs).
/// Tries CVINIT-style extraction first, then falls back to simple set-string extraction
/// for files like CNINIT2.BIN that just write names to global string addresses.
#[tauri::command]
pub async fn scan_bin_for_characters(
    path: String,
) -> Result<Vec<CharacterEntry>, String> {
    let data = std::fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;
    let parsed = parse_bin(&data).map_err(|e| format!("Failed to parse BIN: {}", e))?;

    // CIINIT.BIN uses a different layout: full names at a contiguous stride-1
    // run of string addresses. Dispatch by filename before the CVINIT path,
    // since the CVINIT heuristic produces garbage on CIINIT.
    let filename = std::path::Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();
    if filename.starts_with("CIINIT") {
        let ciinit_chars = extract_ciinit_character_names(&parsed.instructions);
        if !ciinit_chars.is_empty() {
            return Ok(ciinit_chars);
        }
    }

    // Try CVINIT-style extraction (SYS4/SYS5: set-string blocks + ID register)
    let mut chars = extract_all_character_definitions(&parsed.instructions);
    if !chars.is_empty() {
        // If pointers look like raw global string addresses (all > 0x1000 and in a
        // contiguous range), this is a CNINIT-style file without an ID register.
        // Normalize pointers to character IDs: ID = address - min_address + 1.
        if chars.len() > 1 {
            let min_ptr = chars.iter().map(|c| c.pointer).min().unwrap_or(0);
            let max_ptr = chars.iter().map(|c| c.pointer).max().unwrap_or(0);
            let range = max_ptr - min_ptr;
            // Heuristic: if minimum pointer is high (global string address space)
            // and the range is reasonable relative to entry count, normalize.
            if min_ptr > 0x1000 && range < (chars.len() as u32 * 10) {
                for c in &mut chars {
                    c.pointer = c.pointer - min_ptr + 1;
                }
            }
        }

        // Reconcile with the file's OWN ciinit-style run. Some games structure
        // CVINIT like a CIINIT — a single stride-1 name run with no per-block ID
        // register — which makes extract_all_character_definitions fall back to
        // raw string addresses for most blocks, leaving many junk high pointers
        // (e.g. range 1..=75554). When that happens the ciinit-style extractor
        // recovers the correct contiguous scene-facing IDs. Only override when
        // (a) the CVINIT-style result is clearly broken (≥3 junk pointers > 1000)
        // AND (b) the ciinit run produces more usable low IDs — so games whose
        // CVINIT has a real ID register are untouched.
        let usable_low = |v: &[CharacterEntry]| {
            v.iter().filter(|c| c.pointer > 0 && c.pointer <= 1000).count()
        };
        let junk_high = chars.iter().filter(|c| c.pointer > 1000).count();
        if junk_high >= 3 {
            let ci_self = extract_ciinit_character_names(&parsed.instructions);
            if usable_low(&ci_self) > usable_low(&chars) {
                chars = ci_self;
            }
        }

        // When scanning CVINIT, augment with CIINIT.BIN from the same folder:
        // CIINIT carries full names for IDs that CVINIT may only reference by
        // short-name or omit entirely. We fill gaps and replace short/empty
        // names, but CVINIT wins when both have a non-empty name.
        if filename.starts_with("CVINIT") {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                let ciinit_path = parent.join("CIINIT.BIN");
                if ciinit_path.exists() {
                    if let Ok(ci_data) = std::fs::read(&ciinit_path) {
                        if let Ok(ci_parsed) = parse_bin(&ci_data) {
                            let ci_chars = extract_ciinit_character_names(&ci_parsed.instructions);
                            let known: std::collections::HashSet<u32> =
                                chars.iter().map(|c| c.pointer).collect();
                            for ci in ci_chars {
                                if !known.contains(&ci.pointer) {
                                    chars.push(ci);
                                }
                            }
                            chars.sort_by_key(|c| c.pointer);
                        }
                    }
                }
            }
        }

        return Ok(chars);
    }

    // Fall back to simple set-string extraction (CNINIT2-style: each set-string is a name)
    let mut result = Vec::new();
    let mut id = 1u32;
    for instr in &parsed.instructions {
        if instr.opcode == 0x192 && instr.arguments.len() >= 2 {
            if let Some(ref name) = instr.arguments[1].decoded_string {
                if !name.is_empty() {
                    result.push(CharacterEntry { pointer: id, name: name.clone() });
                    id += 1;
                }
            }
        }
    }

    Ok(result)
}

/// Get all instructions from the loaded BIN file for raw inspection.
#[tauri::command]
pub async fn get_all_instructions(
    tab_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::bin_format::instruction::Instruction>, String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    let parsed_bin = tab.parsed_bin.as_ref().ok_or("No BIN file loaded")?;
    Ok(parsed_bin.instructions.clone())
}

/// Compute the SHA-256 hash of a file.
#[tauri::command]
pub async fn compute_file_hash(path: String) -> Result<String, String> {
    let data = std::fs::read(&path).map_err(|e| format!("Failed to read file: {}", e))?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(hex::encode(hasher.finalize()))
}

/// Close a tab and free its Rust-side state.
#[tauri::command]
pub async fn close_tab(tab_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.remove_tab(&tab_id);
    Ok(())
}

/// Split capability for a tab's loaded BIN, used to size the CC-translate
/// budget per file: whether the script can grow into extra text boxes (a bridge
/// template is available) and whether it can break lines within a box (it
/// contains at least one end-text-line `0x6F` to clone).
#[derive(serde::Serialize)]
pub struct SplitCapability {
    pub can_add_boxes: bool,
    pub can_break_lines: bool,
}

#[tauri::command]
pub async fn get_split_capability(
    tab_id: String,
    state: State<'_, AppState>,
) -> Result<SplitCapability, String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    let can_add_boxes = tab
        .bridge_template
        .as_ref()
        .map(|b| !matches!(b.quality, crate::editor::bridge::TemplateQuality::Unavailable))
        .unwrap_or(false);
    let can_break_lines = tab
        .parsed_bin
        .as_ref()
        .map(|p| p.instructions.iter().any(|i| i.opcode == 0x6F))
        .unwrap_or(false);
    Ok(SplitCapability {
        can_add_boxes,
        can_break_lines,
    })
}

/// Return the bridge template quality for a tab's loaded BIN.
/// The frontend uses this to drive the auto-split status badge.
#[tauri::command]
pub async fn get_bridge_template_status(
    tab_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    match &tab.bridge_template {
        Some(bt) => Ok(serde_json::to_string(&bt.quality).unwrap().trim_matches('"').to_string()),
        None => Ok("unavailable".to_string()),
    }
}

/// Compute a split plan for an overflowing entry. Returns a SplitPlan that
/// the frontend applies via `applySplitPlan`.
#[tauri::command]
pub async fn auto_split_entry(
    tab_id: String,
    translated_text: String,
    entry_index: usize,
    line_char_cap: u32,
    block_char_cap: u32,
    max_lines_per_box: u32,
    state: State<'_, AppState>,
) -> Result<crate::editor::bridge::SplitPlan, String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;

    let template = tab
        .bridge_template
        .as_ref()
        .ok_or("No bridge template available")?;

    if line_char_cap == 0 {
        return Err("Per-line character cap must be set for auto-split".into());
    }

    // Explicit max-lines-per-box always wins. Fallback to block_cap/line_cap
    // ratio for backward compat with older frontend versions that don't pass it.
    let lines_per_box = if max_lines_per_box > 0 {
        max_lines_per_box
    } else if block_char_cap > 0 && line_char_cap > 0 {
        (block_char_cap / line_char_cap).max(1)
    } else {
        3
    };

    let wrap = crate::editor::wrap::wrap_english(&translated_text, line_char_cap, lines_per_box);

    // Empty wrap (no text at all) — nothing to do.
    if wrap.boxes.is_empty() {
        return Err("No text to split".into());
    }

    // A bridge is only needed to open ADDITIONAL text boxes. A single-box wrap
    // just inserts line breaks (end-text-line) and needs no bridge — so only
    // require a usable bridge when the wrap spilled into more than one box.
    // This lets bridge-less BINs still wrap long lines within their one box.
    if wrap.boxes.len() > 1
        && matches!(template.quality, crate::editor::bridge::TemplateQuality::Unavailable)
    {
        return Err(
            "This translation needs an extra text box, but this BIN has no usable bridge template. Shorten it to fit one box.".into(),
        );
    }

    // If wrap produced exactly one box with one line AND that line matches
    // the original, there's nothing to fix.
    if wrap.boxes.len() == 1 && wrap.boxes[0].len() == 1 && wrap.boxes[0][0] == translated_text {
        return Err("Text already fits within the line cap".into());
    }

    // Speaker ID: 0 (narrator) is fine — the bridge clone will just leave
    // speaker args at 0 if no speaker_arg_positions are populated.
    let speaker_id = 0u32;

    // Extract argument template from the source show-text instruction so the
    // frontend can build structurally correct continuation instructions.
    let parsed_bin = tab.parsed_bin.as_ref().ok_or("No parsed BIN data")?;
    let show_text_template_args = if entry_index < parsed_bin.instructions.len() {
        let source_instr = &parsed_bin.instructions[entry_index];
        source_instr
            .arguments
            .iter()
            .map(|a| crate::editor::bridge::SerializedArg {
                arg_type: a.arg_type,
                raw_data: a.raw_data,
                decoded_string: a.decoded_string.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };

    // Template for the end-text-line (opcode 0x6F) inserted between wrapped
    // lines. Prefer cloning a real one from the BIN; if the BIN has none (rare —
    // e.g. auto-wrap SYS4 scenes with no native line breaks), synthesize it.
    // 0x6F is a 1-arg opcode whose single argument is a literal 0 (verified
    // across scenes), so a synthetic instance is structurally identical to a
    // cloned one — safe to insert.
    const OP_END_TEXT_LINE: u32 = 0x6F;
    let end_text_line_template = Some(
        parsed_bin
            .instructions
            .iter()
            .find(|i| i.opcode == OP_END_TEXT_LINE)
            .map(crate::editor::bridge::SerializedInstruction::from)
            .unwrap_or_else(|| crate::editor::bridge::SerializedInstruction {
                opcode: OP_END_TEXT_LINE,
                label: "end-text-line".to_string(),
                arguments: vec![crate::editor::bridge::SerializedArg {
                    arg_type: 0,
                    raw_data: 0,
                    decoded_string: None,
                }],
            }),
    );

    Ok(crate::editor::bridge::plan_split(
        entry_index,
        &wrap,
        template,
        speaker_id,
        None,
        show_text_template_args,
        end_text_line_template,
    ))
}

/// Compose the BATCH-PROMPT.md content that the parent-level Claude Code
/// session reads to drive iteration across every subfolder.
///
/// Modernized for v0.5: defers all workflow details to per-folder PROMPT.md
/// (single source of truth) and assumes the launcher pre-filtered the work
/// list via pending_folders.py so Claude never even sees fully-translated
/// folders. Previously this prompt taught a stale per-entry workflow that
/// conflicted with PROMPT.md's modern block-mode flow.
fn build_batch_prompt(_subfolder_names: &[String]) -> String {
    String::from(
        r#"# Batch translation — sequential mode

You're running at the **parent directory** of a CC Translate project. Your work list is the subfolders in `pending-folders.txt` — already pre-filtered to exclude any folder that's 100% translated, so you don't need to verify completion before starting.

## Workflow

For each folder listed in `pending-folders.txt`, in order:

1. `cd <folder>`
2. **Once per session** (i.e. before the first folder), read `PROMPT.md` for the per-block translation rules.
3. Follow the block-by-block workflow described in `PROMPT.md` until that folder's `python translate.py status` reports `N/N translated`.
4. `cd ..` and move to the next folder.

When every listed folder reports 100%, print a one-line summary like:

> **Batch complete — N subfolders, M total entries translated.**

Then exit. Don't start any other work.

## Important — do not re-derive the workflow

`PROMPT.md` is the authoritative source for HOW to translate (block dump → tl-out.json → apply). Don't iterate entries one-by-one — that's the old, slow approach. Use the block-mode flow PROMPT.md describes.

## Cross-folder duplicates

Rows flagged `external_duplicate_of` are skipped by `untranslated_blocks()` and get filled at import time from a sibling folder's canonical translation. You won't see them in `block N` output — `translate.py` handles them.

## Workflow constraints (carry over from PROMPT.md, repeated for emphasis)

- One `block N` call per turn — no batching, no `for i in 0 1 2`, no whole-file dumps.
- No `python -c "..."` with Japanese text — Windows cp932 console will crash.
- Only modify `translated` (and optionally `notes`) on entries; never touch `id`, `original`, `block_id`, `duplicate_of`, `external_duplicate_of`, or any other field.
"#,
    )
}

/// Build the `pending_folders.py` helper that prints subfolders still
/// needing translation work. Used by translate-all.bat/.sh to pre-filter
/// the work list BEFORE spawning Claude — same logic as
/// translate-parallel.py's `folder_progress()` so the two modes treat
/// completion identically.
///
/// Exit code 0 = at least one pending folder was printed.
/// Exit code 1 = nothing pending. The launcher uses this to skip the
/// Claude session entirely when there's no work, saving the full session
/// cost (vs. the old behavior of launching Claude, having it check each
/// folder, and exit — ~600-1000 tokens per skipped folder).
fn build_pending_folders_script(subfolder_names: &[String]) -> String {
    let folders_json = serde_json::to_string(subfolder_names).unwrap_or_else(|_| "[]".to_string());
    format!(
        r#"#!/usr/bin/env python3
"""Print subfolders still needing translation work, one per line.

A folder counts as "done" when every entry in its translation.json either
has a non-empty `translated`, or has `duplicate_of` / `external_duplicate_of`
set (those get auto-resolved at import time and don't need Claude).

Exit code 0 = >=1 folder pending (printed to stdout).
Exit code 1 = nothing pending (launcher uses this to skip Claude entirely)."""
from __future__ import annotations
import json, sys
from pathlib import Path

FOLDERS = {folders_json}
HERE = Path(__file__).resolve().parent

def is_pending(folder: str) -> bool:
    try:
        path = HERE / folder / "translation.json"
        if not path.is_file():
            return True  # missing — surface so user notices
        data = json.loads(path.read_text(encoding="utf-8"))
        items = data.get("entries", [])
        if not items:
            return False  # truly empty file, nothing to do
        done = sum(
            1 for e in items
            if str(e.get("translated", "")).strip()
            or e.get("duplicate_of") is not None
            or e.get("external_duplicate_of") is not None
        )
        return done < len(items)
    except Exception:
        return True  # corrupt — surface

pending = [f for f in FOLDERS if is_pending(f)]
for f in pending:
    print(f)
sys.exit(0 if pending else 1)
"#,
    )
}

/// Write a `translate-all.bat` and `translate-all.sh` at `parent_dir` that
/// iterate every subfolder and run its own translate launcher. Used by the
/// multi-BIN export so the user can kick off all translations sequentially
/// with a single double-click.
#[tauri::command]
pub async fn write_multi_translate_launchers(
    parent_dir: String,
    subfolder_names: Vec<String>,
    claude_model: Option<String>,
    parallel: bool,
    parallel_limit: u32,
) -> Result<(), String> {
    let parent = std::path::Path::new(&parent_dir);
    if !parent.is_dir() {
        return Err(format!("Not a directory: {}", parent_dir));
    }
    let model_flag = match claude_model.as_deref().map(str::trim) {
        Some(m) if !m.is_empty() => format!(" --model \"{}\"", m),
        _ => String::new(),
    };

    // Batch launcher: one interactive Claude Code session at the parent
    // directory that iterates every subfolder itself. We deliberately do NOT
    // use `claude -p` — the user wants the interactive UI with progress
    // visibility — so we instead let Claude handle the loop. BATCH-PROMPT.md
    // alongside the launcher gives Claude the full iteration instructions
    // and the launcher just points it there.
    let batch_prompt_md = build_batch_prompt(&subfolder_names);
    std::fs::write(parent.join("BATCH-PROMPT.md"), batch_prompt_md)
        .map_err(|e| format!("Failed to write BATCH-PROMPT.md: {}", e))?;

    // Pre-filter helper used by both sequential launchers: prints subfolders
    // that still need work, exits with code 1 when there's nothing pending.
    // Lets the .bat / .sh short-circuit completely instead of launching
    // Claude just to discover everything's done.
    let pending_script = build_pending_folders_script(&subfolder_names);
    std::fs::write(parent.join("pending_folders.py"), pending_script)
        .map_err(|e| format!("Failed to write pending_folders.py: {}", e))?;

    // Pre-authorize the parent-level session for Bash(python:*) + file edits
    // so it doesn't prompt as it cd's between subfolders.
    let claude_dir = parent.join(".claude");
    std::fs::create_dir_all(&claude_dir)
        .map_err(|e| format!("Failed to create parent .claude/: {}", e))?;
    let settings_json = r#"{
  "permissions": {
    "allow": [
      "Bash(python:*)",
      "Bash(python3:*)",
      "Bash(py:*)",
      "Bash(cd:*)",
      "Bash(ls:*)",
      "Bash(dir:*)",
      "Read",
      "Edit",
      "Write",
      "Glob",
      "Grep"
    ]
  }
}
"#;
    std::fs::write(claude_dir.join("settings.local.json"), settings_json)
        .map_err(|e| format!("Failed to write parent .claude/settings.local.json: {}", e))?;

    let launch_prompt = "Read BATCH-PROMPT.md in this directory and execute it end-to-end.";

    // Sequential launcher with a pre-Claude filter: pending_folders.py runs
    // first, writes pending-folders.txt for Claude to read, and exits with
    // code 1 if nothing's pending → the launcher skips spawning Claude
    // entirely. Matches translate-parallel.py's skip behavior and saves the
    // ~600-1000 tokens-per-skipped-folder that the old "let Claude check
    // each folder itself" approach cost.
    let bat = format!(
        "@echo off\r\n\
where claude >nul 2>nul || ( echo Claude Code CLI not found on PATH. Install from https://claude.com/code && pause && exit /b 1 )\r\n\
where python >nul 2>nul || ( echo Python not found on PATH; required for the pre-check helper && pause && exit /b 1 )\r\n\
python pending_folders.py > pending-folders.txt\r\n\
if errorlevel 1 (\r\n\
  echo All {0} subfolder(s) already 100%% translated. Nothing to do.\r\n\
  del pending-folders.txt 2>nul\r\n\
  pause\r\n\
  exit /b 0\r\n\
)\r\n\
echo Pending folders \\(from pending_folders.py\\):\r\n\
type pending-folders.txt\r\n\
echo.\r\n\
echo Launching Claude Code to process the above...\r\n\
claude --permission-mode acceptEdits{1} \"{2}\"\r\n\
del pending-folders.txt 2>nul\r\n\
pause\r\n",
        subfolder_names.len(), model_flag, launch_prompt
    );
    std::fs::write(parent.join("translate-all.bat"), bat)
        .map_err(|e| format!("Failed to write translate-all.bat: {}", e))?;

    let sh = format!(
        "#!/usr/bin/env bash\nset -e\n\
if ! command -v claude >/dev/null 2>&1; then\n\
  echo \"Claude Code CLI not found on PATH. Install from https://claude.com/code\"\n\
  exit 1\n\
fi\n\
PY=$(command -v python3 || command -v python || true)\n\
if [ -z \"$PY\" ]; then\n\
  echo \"Python not found on PATH; required for the pre-check helper.\"\n\
  exit 1\n\
fi\n\
if ! \"$PY\" pending_folders.py > pending-folders.txt; then\n\
  echo \"All {0} subfolder(s) already 100% translated. Nothing to do.\"\n\
  rm -f pending-folders.txt\n\
  exit 0\n\
fi\n\
echo \"Pending folders (from pending_folders.py):\"\n\
cat pending-folders.txt\n\
echo\n\
echo \"Launching Claude Code to process the above...\"\n\
claude --permission-mode acceptEdits{1} \"{2}\"\n\
rm -f pending-folders.txt\n",
        subfolder_names.len(), model_flag, launch_prompt
    );
    let sh_path = parent.join("translate-all.sh");
    std::fs::write(&sh_path, sh)
        .map_err(|e| format!("Failed to write translate-all.sh: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&sh_path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&sh_path, perms);
        }
    }

    // Optional: parallel launcher. Uses a small Python orchestrator (shipped
    // as translate-parallel.py) so concurrency control + cross-platform
    // process management is one consistent implementation. Each spawned
    // process runs `claude -p` headless because interactive sessions don't
    // multiplex well — N interactive windows would just be a mess.
    if parallel {
        let limit = parallel_limit.max(1);
        let model = claude_model.as_deref().map(str::trim).filter(|s| !s.is_empty());

        let py = build_parallel_orchestrator(&subfolder_names, model, limit);
        std::fs::write(parent.join("translate-parallel.py"), py)
            .map_err(|e| format!("Failed to write translate-parallel.py: {}", e))?;

        let bat = format!(
            "@echo off\r\n\
where claude >nul 2>nul || ( echo Claude Code CLI not found on PATH. Install from https://claude.com/code && pause && exit /b 1 )\r\n\
where python >nul 2>nul || ( echo Python not found on PATH. Install Python 3.8+. && pause && exit /b 1 )\r\n\
echo Running parallel translation across {} subfolder(s), {} at a time...\r\n\
python translate-parallel.py\r\npause\r\n",
            subfolder_names.len(), limit
        );
        std::fs::write(parent.join("translate-parallel.bat"), bat)
            .map_err(|e| format!("Failed to write translate-parallel.bat: {}", e))?;

        let sh = format!(
            "#!/usr/bin/env bash\nset -e\n\
command -v claude >/dev/null 2>&1 || {{ echo \"Claude Code CLI not on PATH\"; exit 1; }}\n\
command -v python3 >/dev/null 2>&1 || command -v python >/dev/null 2>&1 || {{ echo \"Python not on PATH\"; exit 1; }}\n\
echo \"Running parallel translation across {} subfolder(s), {} at a time...\"\n\
(command -v python3 >/dev/null 2>&1 && python3 translate-parallel.py) || python translate-parallel.py\n",
            subfolder_names.len(), limit
        );
        let par_sh = parent.join("translate-parallel.sh");
        std::fs::write(&par_sh, sh)
            .map_err(|e| format!("Failed to write translate-parallel.sh: {}", e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&par_sh) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(&par_sh, perms);
            }
        }
    }

    Ok(())
}

/// Build the translate-parallel.py orchestrator script. Spawns subprocesses
/// with a semaphore-limited concurrency, prints per-folder progress, exits
/// cleanly when all done.
fn build_parallel_orchestrator(
    subfolder_names: &[String],
    claude_model: Option<&str>,
    parallel_limit: u32,
) -> String {
    let folders_json = serde_json::to_string(subfolder_names).unwrap_or_else(|_| "[]".to_string());
    let model_arg = match claude_model {
        Some(m) => format!("\"--model\", {}, ", serde_json::to_string(m).unwrap_or_default()),
        None => String::new(),
    };
    let task = "Read PROMPT.md and follow its instructions. Use translate.py to load/save/iterate translation.json. Translate every untranslated entry. When done, save and exit.";
    let task_json = serde_json::to_string(task).unwrap_or_default();

    format!(
        r#"#!/usr/bin/env python3
"""
Parallel orchestrator for the CC Translate batch workflow.

Spawns up to MAX_PARALLEL `claude -p` sessions concurrently across the
listed subfolders. Each session translates one folder headlessly and exits.
Prints per-folder start/finish lines so progress is visible.

Log files: each subfolder gets a `claude-output.log` with the full stdout
+ stderr from its session, so you can debug what went wrong on failure
without re-running.
"""
from __future__ import annotations
import concurrent.futures as cf
import subprocess
import sys
import time
from pathlib import Path

FOLDERS = {folders_json}
MAX_PARALLEL = {limit}
HERE = Path(__file__).resolve().parent

def folder_progress(cwd: Path) -> tuple[int, int]:
    """Return (translated, total) by reading the folder's translation.json
    directly. Used to skip already-completed folders on resume.

    An entry counts as "done" if any of these are true:
      - `translated` is non-empty
      - `duplicate_of` is set (in-folder canonical handles it at import)
      - `external_duplicate_of` is set (sibling folder handles it at import)
    The latter two need no Claude work — counting them avoids re-spawning
    Claude on folders whose only "remaining" rows are auto-resolved."""
    try:
        import json
        data = json.loads((cwd / "translation.json").read_text(encoding="utf-8"))
        items = data.get("entries", [])
        total = len(items)
        done = sum(
            1 for e in items
            if str(e.get("translated", "")).strip()
            or e.get("duplicate_of") is not None
            or e.get("external_duplicate_of") is not None
        )
        return done, total
    except Exception:
        return 0, 0

def run_one(folder: str) -> tuple[str, int, str]:
    cwd = HERE / folder
    if not cwd.is_dir():
        return folder, 1, "folder missing"

    # Resume support — skip folders that are already 100% translated.
    # untranslated entries are anything still empty + not a duplicate_of.
    done, total = folder_progress(cwd)
    if total > 0 and done >= total:
        print(f"[skip  ] {{folder}}  (already 100% — {{done}}/{{total}})", flush=True)
        return folder, 0, "skipped (complete)"

    started = time.time()
    pending = total - done if total > 0 else "?"
    print(f"[start ] {{folder}}  ({{pending}} pending)", flush=True)
    log_path = cwd / "claude-output.log"
    try:
        # text=True alone uses the system locale, which on Japanese Windows is
        # cp932 — and Claude outputs UTF-8 that can include Japanese characters
        # from translation work. Force UTF-8 explicitly with replace-on-error
        # so a bad byte never crashes the orchestrator.
        result = subprocess.run(
            ["claude", "-p", "--permission-mode", "acceptEdits", {model_arg}{task_json}],
            cwd=str(cwd),
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        # Always write a log so debugging is possible after the fact.
        try:
            log_path.write_text(
                f"[exit {{result.returncode}}]\n\n"
                f"=== STDOUT ===\n{{result.stdout}}\n\n"
                f"=== STDERR ===\n{{result.stderr}}\n",
                encoding="utf-8",
            )
        except Exception:
            pass
        elapsed = time.time() - started
        if result.returncode == 0:
            status = "ok"
        else:
            # Surface the last line of stderr (or stdout fallback) so the
            # console gives a hint about WHY this folder failed without
            # making the user open the log file.
            tail_src = (result.stderr or result.stdout or "").strip()
            tail = tail_src.splitlines()[-1] if tail_src else "no output"
            if len(tail) > 80:
                tail = tail[:77] + "..."
            status = f"exit {{result.returncode}}: {{tail}}"
        print(f"[done  ] {{folder}}  ({{elapsed:.1f}}s, {{status}})", flush=True)
        return folder, result.returncode, status
    except Exception as e:
        elapsed = time.time() - started
        print(f"[error ] {{folder}}  ({{elapsed:.1f}}s, {{e}})", flush=True)
        return folder, 99, str(e)

def main() -> int:
    print(f"Translating {{len(FOLDERS)}} subfolder(s), up to {{MAX_PARALLEL}} in parallel...", flush=True)
    print("(folders already 100% translated will be skipped — re-running is safe)", flush=True)
    failures: list[str] = []
    skipped = 0
    succeeded = 0
    with cf.ThreadPoolExecutor(max_workers=MAX_PARALLEL) as ex:
        for folder, code, status in ex.map(run_one, FOLDERS):
            if code == 0 and "skipped" in status:
                skipped += 1
            elif code == 0:
                succeeded += 1
            else:
                failures.append(f"{{folder}} ({{status}})")
    print(flush=True)
    if failures:
        print(f"Batch done — {{succeeded}} ok, {{skipped}} skipped, {{len(failures)}} failed.", flush=True)
        for f in failures[:20]:
            print(f"  - {{f}}", flush=True)
        if len(failures) > 20:
            print(f"  (and {{len(failures) - 20}} more)", flush=True)
        print(flush=True)
        print("Per-folder claude-output.log files have the full stdout+stderr.", flush=True)
        print("Re-run this script to retry only the failed folders (completed ones auto-skip).", flush=True)
        return 1
    print(f"Batch complete — {{succeeded}} translated, {{skipped}} already done. All {{len(FOLDERS)}} folder(s) at 100%.", flush=True)
    return 0

if __name__ == "__main__":
    sys.exit(main())
"#,
        folders_json = folders_json,
        limit = parallel_limit,
        model_arg = model_arg,
        task_json = task_json,
    )
}

/// One subfolder's translation progress, surfaced by `scan_translation_project_status`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranslationFolderStatus {
    /// Subfolder name (just the basename, e.g. "SC0000")
    pub folder: String,
    /// Source BIN filename recorded inside the folder's translation.json
    pub source_bin: String,
    /// Source BIN SHA-256 from the translation.json
    pub source_bin_sha256: String,
    /// Total entries that REQUIRE manual translation — excludes `duplicate_of`
    /// rows since those are auto-filled at import time from the canonical's
    /// translation (mirrors translate.py's `untranslated()` filter, so the
    /// GUI's "complete" matches the CLI's "all done").
    pub total: u32,
    /// Entries (within the required set) whose `translated` field is non-empty.
    pub translated: u32,
    /// Optional error string if the folder is malformed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Scan a parent folder for subfolders containing a translation.json file.
/// Each subfolder is treated as a CC Translate export. Returns one
/// `TranslationFolderStatus` per subfolder, sorted alphabetically.
#[tauri::command]
pub async fn scan_translation_project_status(
    parent_dir: String,
) -> Result<Vec<TranslationFolderStatus>, String> {
    let parent = std::path::Path::new(&parent_dir);
    if !parent.is_dir() {
        return Err(format!("Not a directory: {}", parent_dir));
    }

    let mut out: Vec<TranslationFolderStatus> = Vec::new();
    let entries = std::fs::read_dir(parent)
        .map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let translation_json = path.join("translation.json");
        if !translation_json.is_file() {
            continue;
        }
        let folder_name = path.file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("?")
            .to_string();

        let raw = match std::fs::read_to_string(&translation_json) {
            Ok(s) => s,
            Err(e) => {
                out.push(TranslationFolderStatus {
                    folder: folder_name,
                    source_bin: String::new(),
                    source_bin_sha256: String::new(),
                    total: 0,
                    translated: 0,
                    error: Some(format!("Failed to read: {}", e)),
                });
                continue;
            }
        };
        let import: crate::project::translation_export::TranslationExportFile =
            match serde_json::from_str(&raw) {
                Ok(p) => p,
                Err(e) => {
                    out.push(TranslationFolderStatus {
                        folder: folder_name,
                        source_bin: String::new(),
                        source_bin_sha256: String::new(),
                        total: 0,
                        translated: 0,
                        error: Some(format!("Invalid JSON: {}", e)),
                    });
                    continue;
                }
            };
        // Exclude rows that don't need Claude work from BOTH numerator and
        // denominator so the % matches translate.py's view:
        //   - `duplicate_of`         → in-folder dup, canonical handles it
        //   - `external_duplicate_of` → cross-folder dup, sibling handles it
        // Both get their translation copied at import time, so they'd never
        // count toward "translated" yet always inflate the denominator —
        // making folders look unfinished when Claude is genuinely done.
        let required: Vec<_> = import.entries.iter()
            .filter(|e| e.duplicate_of.is_none() && e.external_duplicate_of.is_none())
            .collect();
        let total = required.len() as u32;
        let translated = required.iter()
            .filter(|e| !e.translated.trim().is_empty())
            .count() as u32;
        out.push(TranslationFolderStatus {
            folder: folder_name,
            source_bin: import.source_bin,
            source_bin_sha256: import.source_bin_sha256,
            total,
            translated,
            error: None,
        });
    }

    out.sort_by(|a, b| a.folder.cmp(&b.folder));
    Ok(out)
}

/// Scan a list of BIN files for `set-string [pointer] = "name"` writes that
/// match any of the requested pointers. Returns a flat pointer→name map.
///
/// Used to resolve unknown character names by directly looking up known
/// speaker-slot pointers (identified from the scene's bracket triplet pattern
/// `show-text "【"` / `show-text [ref]` / `show-text "】"`) across CVINIT,
/// CIINIT, CNINIT, CHINIT, and the scene BIN itself.
///
/// First non-empty write per pointer wins (the canonical name slot in CVINIT
/// is always the first set-string in each character block).
#[tauri::command]
pub async fn lookup_character_names_for_pointers(
    bin_paths: Vec<String>,
    pointers: Vec<u32>,
) -> Result<std::collections::HashMap<u32, String>, String> {
    let wanted: std::collections::HashSet<u32> = pointers.into_iter().collect();
    if wanted.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let mut combined: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    for path in bin_paths {
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => continue, // non-fatal — skip missing files
        };
        let parsed = match parse_bin(&data) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let found = crate::editor::characters::scan_setstring_writes_for_pointers(
            &parsed.instructions,
            &wanted,
        );
        // First wins across files too — don't override
        for (k, v) in found {
            combined.entry(k).or_insert(v);
        }
    }
    Ok(combined)
}

/// A single candidate name extracted from an init BIN, surfaced in the
/// "Pick from init BINs" dialog so the user can manually resolve an
/// unresolved scene character ID. Scene-local lookup-array indirection
/// makes static ID→name resolution unreliable for SYS4, so we let the user
/// pick from the full pool of names found in CVINIT/CIINIT/CNINIT/CHINIT.
#[derive(serde::Serialize)]
pub struct InitNameCandidate {
    pub source_file: String,
    pub address: u32,
    pub name: String,
}

/// Read a prior `translation.json` from a folder and return its
/// `original → translated` map. Used by re-export delta mode in the
/// frontend: when re-exporting a folder that's already been translated,
/// we preserve the prior translations so Claude only sees the rows whose
/// originals are still empty (no token waste re-translating finished work).
/// Returns an empty map if the file doesn't exist or can't be parsed —
/// the export proceeds normally in that case.
#[tauri::command]
pub async fn read_prior_translation_map(
    folder_path: String,
) -> Result<std::collections::HashMap<String, String>, String> {
    use crate::project::translation_export::TranslationExportFile;
    let path = std::path::Path::new(&folder_path).join("translation.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(std::collections::HashMap::new());
    };
    let Ok(parsed) = serde_json::from_str::<TranslationExportFile>(&raw) else {
        return Ok(std::collections::HashMap::new());
    };
    let mut out = std::collections::HashMap::new();
    for row in parsed.entries {
        let t = row.translated.trim();
        if t.is_empty() { continue; }
        // First non-empty wins (in-folder duplicates share the canonical's
        // translation; we only need one entry per unique original).
        out.entry(row.original).or_insert_with(|| t.to_string());
    }
    Ok(out)
}

/// Project autosave keeps three rolling backups (`.bak1` newest → `.bak3`
/// oldest). This is the metadata for one of them.
#[derive(serde::Serialize)]
pub struct ProjectBackupInfo {
    pub path: String,
    pub depth: u32, // 1 = newest, 3 = oldest
    /// Unix milliseconds when the backup file was last modified.
    pub modified_ms: u64,
    pub size_bytes: u64,
    /// Best-effort entry count from inside the JSON — `None` if the file
    /// couldn't be parsed (corrupt / wrong format).
    pub entry_count: Option<u32>,
}

/// List all `.bak1`..`.bak3` backups for a given project JSON path.
/// Returns newest-first. Missing backups are skipped (no error).
#[tauri::command]
pub async fn list_project_backups(project_path: String) -> Result<Vec<ProjectBackupInfo>, String> {
    let path = std::path::Path::new(&project_path);
    let base = path.file_name().and_then(|n| n.to_str())
        .ok_or("Invalid project path")?;
    let parent = path.parent().ok_or("Project path has no parent directory")?;

    let mut out: Vec<ProjectBackupInfo> = Vec::new();
    for depth in 1..=3u32 {
        let bak = parent.join(format!("{}.bak{}", base, depth));
        let Ok(meta) = std::fs::metadata(&bak) else { continue };
        if !meta.is_file() { continue; }
        let modified_ms = meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        // Try to count entries by parsing; fall back to None on any error.
        let entry_count: Option<u32> = std::fs::read_to_string(&bak).ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| v.get("entries").and_then(|e| e.as_array()).map(|a| a.len() as u32));
        out.push(ProjectBackupInfo {
            path: bak.to_string_lossy().to_string(),
            depth,
            modified_ms,
            size_bytes: meta.len(),
            entry_count,
        });
    }
    Ok(out)
}

/// One differing entry between two project snapshots — fed to the backup
/// browser's diff list so the user can see what would change BEFORE reverting.
#[derive(serde::Serialize)]
pub struct BackupDiffRow {
    pub instruction_index: i64,
    pub label: String,
    pub original_text: String,
    /// Translated text in the CURRENT project (what reverting overwrites).
    pub current_translated: String,
    /// Translated text in the BACKUP (what reverting restores).
    pub backup_translated: String,
    /// True when omit_export differs.
    pub omit_changed: bool,
}

#[derive(serde::Serialize)]
pub struct BackupDiff {
    pub total_current: u32,
    pub total_backup: u32,
    /// Entries with a different `translated_text`.
    pub translated_diff_count: u32,
    /// Entries with a different `omit_export` flag.
    pub omit_diff_count: u32,
    /// Detailed differences (capped at 200 to keep the dialog responsive).
    pub rows: Vec<BackupDiffRow>,
}

/// Compute a side-by-side diff between the current project JSON and a backup.
#[tauri::command]
pub async fn diff_project_backup(
    current_path: String,
    backup_path: String,
) -> Result<BackupDiff, String> {
    use crate::project::types::ProjectFile;
    let cur_raw = std::fs::read_to_string(&current_path)
        .map_err(|e| format!("Failed to read current project: {}", e))?;
    let bak_raw = std::fs::read_to_string(&backup_path)
        .map_err(|e| format!("Failed to read backup: {}", e))?;
    let cur: ProjectFile = serde_json::from_str(&cur_raw)
        .map_err(|e| format!("Invalid current project JSON: {}", e))?;
    let bak: ProjectFile = serde_json::from_str(&bak_raw)
        .map_err(|e| format!("Invalid backup JSON: {}", e))?;

    // Index backup by instruction_index for O(1) lookup.
    let bak_by_idx: std::collections::HashMap<i64, &crate::editor::extract::DialogueEntry> =
        bak.entries.iter().map(|e| (e.instruction_index, e)).collect();

    let mut rows: Vec<BackupDiffRow> = Vec::new();
    let mut translated_diff = 0u32;
    let mut omit_diff = 0u32;
    const ROW_CAP: usize = 200;

    for cur_e in &cur.entries {
        let Some(bak_e) = bak_by_idx.get(&cur_e.instruction_index) else { continue };
        let tr_diff = cur_e.translated_text != bak_e.translated_text;
        let om_diff = cur_e.omit_export != bak_e.omit_export;
        if !tr_diff && !om_diff { continue; }
        if tr_diff { translated_diff += 1; }
        if om_diff { omit_diff += 1; }
        if rows.len() < ROW_CAP {
            rows.push(BackupDiffRow {
                instruction_index: cur_e.instruction_index,
                label: cur_e.label.clone(),
                original_text: cur_e.original_text.clone(),
                current_translated: cur_e.translated_text.clone(),
                backup_translated: bak_e.translated_text.clone(),
                omit_changed: om_diff,
            });
        }
    }

    Ok(BackupDiff {
        total_current: cur.entries.len() as u32,
        total_backup: bak.entries.len() as u32,
        translated_diff_count: translated_diff,
        omit_diff_count: omit_diff,
        rows,
    })
}

/// Replace the current project JSON with a backup. Implementation: copy
/// backup over the current file. The next autosave will rotate the (now
/// reverted) state into a fresh `.bak1`, pushing prior backups down.
/// This preserves backup chain integrity at the cost of one rotation cycle.
#[tauri::command]
pub async fn restore_project_backup(
    current_path: String,
    backup_path: String,
) -> Result<(), String> {
    let bak = std::path::Path::new(&backup_path);
    let cur = std::path::Path::new(&current_path);
    if !bak.exists() {
        return Err(format!("Backup not found: {}", backup_path));
    }
    std::fs::copy(bak, cur)
        .map_err(|e| format!("Failed to restore backup: {}", e))?;
    Ok(())
}

/// Read an optional project-folder-level glossary at `<bin_dir>/glossary.json`.
/// Returns the entries if the file exists and parses; empty list otherwise.
/// Lets a translator drop a `glossary.json` next to the BINs and have every
/// loaded BIN pick it up automatically — a third glossary tier between
/// per-tab project and app-global.
#[tauri::command]
pub async fn load_folder_glossary(
    bin_path: String,
) -> Result<Vec<crate::project::types::GlossaryEntry>, String> {
    let path = std::path::Path::new(&bin_path);
    let dir = path.parent().ok_or("Invalid bin path")?;
    let glossary_path = dir.join("glossary.json");
    if !glossary_path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&glossary_path)
        .map_err(|e| format!("Failed to read glossary.json: {}", e))?;
    // Accept both the editor's export format ({version, entries: [...]}) and
    // a bare entry array so the user can hand-write a simple JSON file.
    #[derive(serde::Deserialize)]
    struct ExportShape { entries: Vec<crate::project::types::GlossaryEntry> }
    if let Ok(parsed) = serde_json::from_str::<ExportShape>(&raw) {
        return Ok(parsed.entries);
    }
    let entries: Vec<crate::project::types::GlossaryEntry> = serde_json::from_str(&raw)
        .map_err(|e| format!("Invalid glossary.json (must be export-shape or entry array): {}", e))?;
    Ok(entries)
}

/// Write the project-folder-level glossary at `<bin_dir>/glossary.json`,
/// in the same export-shape format as the regular glossary export.
#[tauri::command]
pub async fn save_folder_glossary(
    bin_path: String,
    entries: Vec<crate::project::types::GlossaryEntry>,
) -> Result<(), String> {
    let path = std::path::Path::new(&bin_path);
    let dir = path.parent().ok_or("Invalid bin path")?;
    let glossary_path = dir.join("glossary.json");
    let body = serde_json::json!({
        "version": 1,
        "type": "eushully-glossary",
        "entries": entries,
    });
    let json = serde_json::to_string_pretty(&body)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    std::fs::write(&glossary_path, json)
        .map_err(|e| format!("Failed to write glossary.json: {}", e))?;
    Ok(())
}

/// Open a folder in the system's native file explorer (Explorer on Windows,
/// Finder on macOS, xdg-open on Linux). Used by the translation status dialog
/// so users can inspect a specific folder without leaving the app.
#[tauri::command]
pub async fn open_folder_in_explorer(folder_path: String) -> Result<(), String> {
    let path = std::path::Path::new(&folder_path);
    if !path.exists() {
        return Err(format!("Folder does not exist: {}", folder_path));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open explorer: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open Finder: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to open file manager: {}", e))?;
    }
    Ok(())
}

/// Re-launch a single translation folder's `translate.bat`/`.sh` in a new
/// terminal window. Used by the per-folder retry button in the translation
/// status dialog so the user doesn't have to bail to the file explorer to
/// re-run a stalled or partially-translated subfolder.
///
/// On Windows, `cmd /c start "" "<folder>\translate.bat"` opens a new
/// console; the user can watch progress live and close it when done.
#[tauri::command]
pub async fn relaunch_translation_folder(folder_path: String) -> Result<(), String> {
    let path = std::path::Path::new(&folder_path);
    if !path.is_dir() {
        return Err(format!("Not a directory: {}", folder_path));
    }
    #[cfg(target_os = "windows")]
    {
        let bat = path.join("translate.bat");
        if !bat.exists() {
            return Err(format!("translate.bat not found in {}", folder_path));
        }
        // `start "" /d <dir> <path>` opens the .bat in a new console with a
        // pinned working directory. Belt-and-suspenders with the .bat's own
        // `cd /d "%~dp0"` — either alone fixes the "Claude searches the
        // wrong folder for PROMPT.md" symptom, but both together survive
        // anything weird (UNC paths, network shares, console-host quirks).
        // The empty "" is start's title arg, required when first arg is quoted.
        std::process::Command::new("cmd")
            .args(["/c", "start", "", "/d", folder_path.as_str(), bat.to_str().unwrap_or("")])
            .spawn()
            .map_err(|e| format!("Failed to launch translate.bat: {}", e))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let sh = path.join("translate.sh");
        if !sh.exists() {
            return Err(format!("translate.sh not found in {}", folder_path));
        }
        // Use the system terminal; user can configure $TERMINAL if needed.
        // Set cwd explicitly so Claude lands in the right folder regardless
        // of how the terminal handles its own cwd inheritance.
        let term = std::env::var("TERMINAL").unwrap_or_else(|_| "x-terminal-emulator".into());
        std::process::Command::new(term)
            .args(["-e", "bash", sh.to_str().unwrap_or("")])
            .current_dir(&folder_path)
            .spawn()
            .map_err(|e| format!("Failed to launch translate.sh: {}", e))?;
    }
    Ok(())
}

/// Build a `character_id → name` map for a CHINIT-style init BIN by detecting
/// its consecutive-register character table. See `extract_chinit_id_name_map`
/// in `editor/characters.rs` for the pattern explanation. Returns string keys
/// because serde_json serializes integer-keyed maps as strings anyway.
///
/// Cached by file SHA-256: every scene loaded from the same game folder shares
/// a single parse of CHINIT.BIN (which can be 80k+ instructions). Cache lives
/// in AppState so it survives across tab opens but resets on app restart.
#[tauri::command]
pub async fn extract_chinit_id_name_map(
    bin_path: String,
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let data = std::fs::read(&bin_path).map_err(|e| format!("Failed to read file: {}", e))?;

    // Hash first so we can short-circuit before paying the parse cost.
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let hash = hex::encode(hasher.finalize());

    if let Some(cached) = state.chinit_cache.lock().unwrap().get(&hash) {
        return Ok(cached.iter().map(|(k, v)| (k.to_string(), v.clone())).collect());
    }

    let parsed = parse_bin(&data).map_err(|e| format!("Failed to parse BIN: {}", e))?;
    let map = crate::editor::characters::extract_chinit_id_name_map(&parsed.instructions);

    state.chinit_cache.lock().unwrap().insert(hash, map.clone());

    Ok(map.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// List every set-string-defined name in the provided init BIN paths.
/// Returns one entry per unique (source_file, address, name) triple, in
/// stable file order — frontend filters/sorts as needed for the picker UI.
#[tauri::command]
pub async fn list_init_bin_names(
    bin_paths: Vec<String>,
) -> Result<Vec<InitNameCandidate>, String> {
    let mut out = Vec::new();
    for path in bin_paths {
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(_) => continue, // skip missing
        };
        let parsed = match parse_bin(&data) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let filename = std::path::Path::new(&path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let mut seen: std::collections::HashSet<(u32, String)> = std::collections::HashSet::new();
        for instr in &parsed.instructions {
            if instr.opcode != 0x192 || instr.arguments.len() < 2 {
                continue;
            }
            let addr = instr.arguments[0].raw_data;
            let name = match &instr.arguments[1].decoded_string {
                Some(n) => n.trim().to_string(),
                None => continue,
            };
            if name.is_empty() {
                continue;
            }
            if seen.insert((addr, name.clone())) {
                out.push(InitNameCandidate {
                    source_file: filename.clone(),
                    address: addr,
                    name,
                });
            }
        }
    }
    Ok(out)
}

/// Return a cloned bridge template's instructions, ready for the frontend to
/// insert when manually splitting a line group across textbox boundaries
/// (e.g. when splitOverflowingGroup hits the max-lines-per-box limit).
#[tauri::command]
pub async fn get_bridge_clone(
    tab_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::editor::bridge::SerializedInstruction>, String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    let template = tab
        .bridge_template
        .as_ref()
        .ok_or("No bridge template available")?;
    if matches!(template.quality, crate::editor::bridge::TemplateQuality::Unavailable) {
        return Err("Bridge template unavailable for this BIN".into());
    }
    // Speaker ID 0 (narrator) — same default used by auto_split_entry. For
    // group splits we generally want to preserve the current speaker context
    // anyway, and the bridge template instructions don't need speaker rewriting
    // when speaker_arg_positions is empty (which it currently is in v1).
    let cloned = crate::editor::bridge::clone_bridge(template, 0, None);
    Ok(cloned.iter().map(crate::editor::bridge::SerializedInstruction::from).collect())
}

/// Export a glossary to a JSON file.
#[tauri::command]
pub async fn export_glossary(
    entries: Vec<GlossaryEntry>,
    path: String,
) -> Result<(), String> {
    let export_file = serde_json::json!({
        "version": 1,
        "type": "eushully-glossary",
        "entries": entries
    });

    let json = serde_json::to_string_pretty(&export_file)
        .map_err(|e| format!("Failed to serialize glossary: {}", e))?;

    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write glossary file: {}", e))?;

    Ok(())
}

/// Load editor settings from a local config file.
#[tauri::command]
pub async fn load_settings(app_handle: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let config_dir = app_handle.path().app_config_dir()
        .map_err(|e| format!("Failed to get config dir: {}", e))?;
    let config_path = config_dir.join("settings.json");

    if !config_path.exists() {
        return Ok(serde_json::json!({}));
    }

    let json = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read settings: {}", e))?;
    serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse settings: {}", e))
}

/// Save editor settings to a local config file.
#[tauri::command]
pub async fn save_settings(app_handle: tauri::AppHandle, settings: serde_json::Value) -> Result<(), String> {
    let config_dir = app_handle.path().app_config_dir()
        .map_err(|e| format!("Failed to get config dir: {}", e))?;

    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config dir: {}", e))?;

    let config_path = config_dir.join("settings.json");
    let json = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;

    std::fs::write(&config_path, json)
        .map_err(|e| format!("Failed to write settings: {}", e))?;

    Ok(())
}

/// Check if a file exists at the given path.
#[tauri::command]
pub async fn file_exists(path: String) -> Result<bool, String> {
    Ok(Path::new(&path).exists())
}

/// Ensure a directory exists, creating it (and parents) if needed.
#[tauri::command]
pub async fn ensure_directory(path: String) -> Result<(), String> {
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("Failed to create directory: {}", e))?;
    Ok(())
}

/// Fetch available Claude models from the Anthropic API.
/// Returns a list of model objects with `id` and `label` fields,
/// filtered to chat-capable Claude models and sorted newest first.
#[tauri::command]
pub async fn fetch_available_models(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let api_key = state.api_key.lock().unwrap().clone()
        .ok_or_else(|| "No API key configured".to_string())?;

    let client = reqwest::Client::new();
    let response = client
        .get("https://api.anthropic.com/v1/models")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch models: {}", e))?;

    let status = response.status().as_u16();
    if status != 200 {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("API error {}: {}", status, body));
    }

    let body: serde_json::Value = response.json().await
        .map_err(|e| format!("Failed to parse models response: {}", e))?;

    // The API returns { "data": [ { "id": "claude-...", "created_at": "...", ... } ] }
    let models = body.get("data")
        .and_then(|d| d.as_array())
        .ok_or("Unexpected API response format")?;

    // Filter to claude chat models (exclude embedding, legacy, etc.)
    let mut filtered: Vec<serde_json::Value> = models.iter()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?;
            // Only include claude-{opus,sonnet,haiku} models
            if !id.starts_with("claude-") { return None; }
            let has_tier = id.contains("opus") || id.contains("sonnet") || id.contains("haiku");
            if !has_tier { return None; }
            // Skip very old models (claude-2, claude-instant, etc.)
            if id.starts_with("claude-2") || id.starts_with("claude-instant") { return None; }
            let created = m.get("created_at").and_then(|c| c.as_str()).unwrap_or("").to_string();
            // Build a friendly label: "Claude Sonnet 4.6" from "claude-sonnet-4-6-20250627"
            let label = build_model_label(id);
            Some(serde_json::json!({
                "id": id,
                "label": label,
                "created_at": created,
            }))
        })
        .collect();

    // Sort by created_at descending (newest first)
    filtered.sort_by(|a, b| {
        let ca = a.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        let cb = b.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        cb.cmp(ca)
    });

    Ok(serde_json::json!(filtered))
}

/// Fetch the model list from an OpenAI-compatible provider via `/v1/models`.
///
/// Works with Mistral, OpenAI, OpenRouter, Groq, Ollama, LM Studio — anything
/// that implements the OpenAI Chat Completions wire protocol. Returns the
/// model `id` strings sorted alphabetically. Empty `api_key` is accepted for
/// local endpoints (Ollama / LM Studio) — the Authorization header is omitted.
///
/// On any error (404, 401, network, malformed JSON) returns Err with the
/// reason — the frontend treats that as "manual entry only" and falls back
/// silently to the placeholder hint instead of blocking the user.
#[tauri::command]
pub async fn fetch_openai_compat_models(
    endpoint: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    let base = endpoint.trim_end_matches('/');
    // Match the same URL normalization the chat client uses.
    let url = if base.ends_with("/v1/models") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{}/models", base)
    } else {
        format!("{}/v1/models", base)
    };

    let mut req = reqwest::Client::new().get(&url);
    if !api_key.trim().is_empty() {
        req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {}", api_key.trim()));
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    let status = response.status().as_u16();
    if status != 200 {
        let body = response.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        return Err(format!("HTTP {}: {}", status, snippet));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let data = body
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "Response missing `data` array".to_string())?;

    let mut ids: Vec<String> = data
        .iter()
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(String::from))
        .collect();
    ids.sort();
    Ok(ids)
}

/// Build a human-readable label from a model ID like "claude-sonnet-4-6-20250627".
fn build_model_label(id: &str) -> String {
    // Strip "claude-" prefix and the date suffix (last segment if it's all digits)
    let rest = id.strip_prefix("claude-").unwrap_or(id);
    let parts: Vec<&str> = rest.split('-').collect();

    // Find where the date suffix starts (a segment that's 8+ digits)
    let name_parts: Vec<&str> = parts.iter()
        .take_while(|p| !(p.len() >= 8 && p.chars().all(|c| c.is_ascii_digit())))
        .copied()
        .collect();

    if name_parts.is_empty() {
        return id.to_string();
    }

    // Try to detect version numbers: consecutive numeric segments like "4" "6" → "4.6"
    let mut label = String::from("Claude ");
    let mut i = 0;
    while i < name_parts.len() {
        if i > 0 { label.push(' '); }
        let part = name_parts[i];
        if part.chars().all(|c| c.is_ascii_digit()) {
            // Start of version: collect consecutive numeric segments joined by dots
            let mut version = part.to_string();
            while i + 1 < name_parts.len() && name_parts[i + 1].chars().all(|c| c.is_ascii_digit()) {
                version.push('.');
                version.push_str(name_parts[i + 1]);
                i += 1;
            }
            label.push_str(&version);
        } else {
            // Capitalize first letter
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                label.push(first.to_ascii_uppercase());
                label.extend(chars);
            }
        }
        i += 1;
    }

    label
}

/// Import a glossary from a JSON file.
#[tauri::command]
pub async fn import_glossary(path: String) -> Result<Vec<GlossaryEntry>, String> {
    let json = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read glossary file: {}", e))?;

    let parsed: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse glossary JSON: {}", e))?;

    let entries_value = parsed.get("entries")
        .ok_or("Glossary file missing 'entries' field")?;

    let entries: Vec<GlossaryEntry> = serde_json::from_value(entries_value.clone())
        .map_err(|e| format!("Failed to parse glossary entries: {}", e))?;

    Ok(entries)
}

/// Scan sibling BIN files in the same folder and extract dialogue summaries
/// for cross-file narrative context. Returns a condensed string suitable for
/// inclusion in the translation system prompt.
#[tauri::command]
pub async fn scan_cross_bin_context(bin_path: String, max_chars: usize) -> Result<String, String> {
    let path = Path::new(&bin_path);
    let parent = path.parent().ok_or("Cannot determine parent directory")?;
    let current_filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");

    let mut context_parts: Vec<String> = Vec::new();
    let mut total_chars: usize = 0;

    // Collect and sort sibling BIN files alphabetically (scene order)
    let mut bin_files: Vec<std::path::PathBuf> = Vec::new();
    let entries = std::fs::read_dir(parent)
        .map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if let Some(ext) = entry_path.extension() {
            let ext_lower = ext.to_string_lossy().to_lowercase();
            if ext_lower == "bin" {
                let fname = entry_path.file_name().and_then(|f| f.to_str()).unwrap_or("");
                // Skip the current file, init files, and system files
                if fname.eq_ignore_ascii_case(current_filename) { continue; }
                let fname_upper = fname.to_uppercase();
                if fname_upper.starts_with("CVINIT") || fname_upper.starts_with("CNINIT") { continue; }
                if fname_upper == "SYS.BIN" || fname_upper == "SYS2.BIN" { continue; }
                bin_files.push(entry_path);
            }
        }
    }

    bin_files.sort();

    for bin_file in &bin_files {
        if total_chars >= max_chars { break; }

        // Try to parse and extract dialogue
        let data = match std::fs::read(bin_file) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let parsed = match parse_bin(&data) {
            Ok(p) => p,
            Err(_) => continue,
        };

        let dialogue = extract_dialogue(&parsed.instructions);
        if dialogue.is_empty() { continue; }

        let filename = bin_file.file_name().and_then(|f| f.to_str()).unwrap_or("?");
        let mut file_section = format!("[{}]\n", filename);

        // Take a sample of dialogue lines (first 20 non-empty lines)
        let mut line_count = 0;
        for entry in &dialogue {
            if entry.is_context_row || entry.original_text.is_empty() { continue; }
            file_section.push_str(&format!("  {}: {}\n", entry.label, entry.original_text));
            line_count += 1;
            if line_count >= 20 { break; }
        }

        if line_count > 0 {
            total_chars += file_section.len();
            context_parts.push(file_section);
        }
    }

    if context_parts.is_empty() {
        return Ok(String::new());
    }

    Ok(context_parts.join("\n"))
}

/// Write `translation.json` and `PROMPT.md` to `output_dir` so the user can
/// translate the file with Claude Code (or any tool) instead of using the API.
/// The frontend has already resolved speakers and built the row list.
#[tauri::command]
pub async fn export_for_translation(
    output_dir: String,
    source_bin_filename: String,
    source_bin_sha256: String,
    wrap_width: u32,
    max_lines_per_box: u32,
    can_add_boxes: bool,
    can_break_lines: bool,
    glossary: Vec<GlossaryEntry>,
    entries: Vec<crate::project::translation_export::TranslationRow>,
    claude_model: Option<String>,
) -> Result<String, String> {
    use crate::project::translation_export::{
        render_prompt, ExportCharLimits, PromptBudget, TranslationExportFile,
    };

    let dir = std::path::Path::new(&output_dir);
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Failed to create directory: {}", e))?;

    // Derive the per-entry budget from the BIN's split capability:
    // - can add boxes        → no hard cap (tool inserts continuation boxes)
    // - no boxes, can wrap    → one box: wrap_width * max_lines
    // - no boxes, no wrap     → one physical line: wrap_width
    let box_total: Option<u32> = if wrap_width == 0 {
        None
    } else if can_add_boxes {
        None
    } else if can_break_lines && max_lines_per_box > 0 {
        Some(wrap_width.saturating_mul(max_lines_per_box))
    } else {
        Some(wrap_width)
    };
    let budget = PromptBudget {
        wrap_width,
        max_lines: max_lines_per_box,
        can_add_boxes,
        box_total,
    };
    // Kept in translation.json for backward-compat readers; `translate.py`
    // ignores it (limits live in PROMPT.md).
    let char_limits = ExportCharLimits {
        line: wrap_width,
        block: box_total.unwrap_or(0),
    };

    let export = TranslationExportFile {
        version: 1,
        source_bin: source_bin_filename,
        source_bin_sha256,
        char_limits: char_limits.clone(),
        // Glossary lives in PROMPT.md only — duplicating it here just inflated
        // tokens. The Python helper never read this field. Field stays in the
        // type for backward compat reading of old JSONs.
        glossary: Vec::new(),
        entries,
    };

    let json = serde_json::to_string_pretty(&export)
        .map_err(|e| format!("Failed to serialize JSON: {}", e))?;
    std::fs::write(dir.join("translation.json"), json)
        .map_err(|e| format!("Failed to write translation.json: {}", e))?;

    let prompt = render_prompt(&budget, &glossary);
    std::fs::write(dir.join("PROMPT.md"), prompt)
        .map_err(|e| format!("Failed to write PROMPT.md: {}", e))?;

    // Glossary as its own file — translate.py auto-injects matching entries
    // into each `block N` output so only relevant terms enter Claude's context.
    // Schema: {"version": 1, "entries": [{japanese, english, category, notes}]}
    let glossary_payload = serde_json::json!({
        "version": 1,
        "entries": &glossary,
    });
    let glossary_json = serde_json::to_string_pretty(&glossary_payload)
        .map_err(|e| format!("Failed to serialize glossary.json: {}", e))?;
    std::fs::write(dir.join("glossary.json"), glossary_json)
        .map_err(|e| format!("Failed to write glossary.json: {}", e))?;

    // Canonical Python helper — ships in every export so Claude Code has a
    // consistent API for loading/saving/iterating translation.json instead
    // of inventing its own helper script each run.
    let translate_py = include_str!("../assets/translate.py");
    std::fs::write(dir.join("translate.py"), translate_py)
        .map_err(|e| format!("Failed to write translate.py: {}", e))?;

    // Pre-authorize Claude Code for the operations our workflow needs.
    // Without this, every `python translate.py status` invocation prompts
    // the user — fatal for multi-BIN batch runs where it'd be hundreds of
    // prompts. With it, the launch is fully fire-and-forget while still
    // limiting Claude Code to a narrow allowlist (no network, no rm, etc.).
    let claude_dir = dir.join(".claude");
    std::fs::create_dir_all(&claude_dir)
        .map_err(|e| format!("Failed to create .claude directory: {}", e))?;
    let settings_json = r#"{
  "permissions": {
    "allow": [
      "Bash(python:*)",
      "Bash(python3:*)",
      "Bash(py:*)",
      "Read",
      "Edit",
      "Write",
      "Glob",
      "Grep"
    ]
  }
}
"#;
    std::fs::write(claude_dir.join("settings.local.json"), settings_json)
        .map_err(|e| format!("Failed to write .claude/settings.local.json: {}", e))?;

    // Launcher scripts — double-click to start Claude Code in this folder.
    // Claude Code reads PROMPT.md which tells it to use translate.py for
    // file IO so every run uses the same canonical helper.
    let model_flag = match claude_model.as_deref().map(str::trim) {
        Some(m) if !m.is_empty() => format!(" --model \"{}\"", m),
        _ => String::new(),
    };
    let task_prompt = "Read PROMPT.md and follow its instructions. Use the translate.py helper to load/save/iterate translation.json. Translate every untranslated entry. When done, save and exit.";

    // CRITICAL: `cd /d "%~dp0"` (Windows) / `cd "$(dirname "$0")"` (Unix)
    // anchors the script's working directory to its OWN folder regardless of
    // how it was launched. Without this, double-clicking or `cmd /c start`
    // from another cwd leaves Claude in the wrong dir — Claude then can't
    // find PROMPT.md or translate.py and starts filesystem-walking
    // (sometimes with PowerShell cmdlets that fail in Git Bash).
    let bat = format!(
        "@echo off\r\n\
cd /d \"%~dp0\"\r\n\
where claude >nul 2>nul || ( echo Claude Code CLI not found on PATH. Install from https://claude.com/code && pause && exit /b 1 )\r\n\
echo Launching Claude Code to translate translation.json...\r\n\
echo Working directory: %CD%\r\n\
echo Tip: run 'python translate.py status' anytime to check progress.\r\n\
claude --permission-mode acceptEdits{} \"{}\"\r\npause\r\n",
        model_flag, task_prompt
    );
    std::fs::write(dir.join("translate.bat"), bat)
        .map_err(|e| format!("Failed to write translate.bat: {}", e))?;

    let sh = format!(
        "#!/usr/bin/env bash\nset -e\n\
cd \"$(dirname \"$0\")\"\n\
if ! command -v claude >/dev/null 2>&1; then\n\
  echo \"Claude Code CLI not found on PATH. Install from https://claude.com/code\"\n\
  exit 1\n\
fi\n\
echo \"Launching Claude Code to translate translation.json...\"\n\
echo \"Working directory: $(pwd)\"\n\
echo \"Tip: run 'python translate.py status' anytime to check progress.\"\n\
claude --permission-mode acceptEdits{} \"{}\"\n",
        model_flag, task_prompt
    );
    std::fs::write(dir.join("translate.sh"), sh)
        .map_err(|e| format!("Failed to write translate.sh: {}", e))?;
    // Mark sh as executable on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("translate.sh");
        if let Ok(meta) = std::fs::metadata(&path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&path, perms);
        }
    }

    Ok(dir.to_string_lossy().to_string())
}

/// Returns true when a string contains only structural markup characters
/// (brackets and whitespace) or is empty. Mirrors `isStructuralOnly` on the
/// TS side — used to protect speaker-name bracket opcodes from being wiped
/// by the auto-omit pass.
fn is_structural_only(s: &str) -> bool {
    if s.is_empty() { return true; }
    s.chars().all(|c| matches!(c,
        '【' | '】' | '「' | '」' | '『' | '』' | '〔' | '〕'
        | '（' | '）' | '(' | ')' | '｛' | '｝' | '[' | ']' | '{' | '}'
    ) || c.is_whitespace())
}

/// Read an edited `translation.json` and merge its translations into the
/// provided dialogue entries by `id`. Returns the updated entries + stats.
///
/// `mode`:
/// - `"fill_missing"` — only set `translated_text` on entries whose current
///   value is empty. Preserves existing edits.
/// - `"overwrite"` — replace `translated_text` and `notes` on every matched
///   entry, even ones that already had translations.
#[tauri::command]
pub async fn import_translations(
    json_path: String,
    current_bin_sha256: String,
    mode: String,
    entries: Vec<DialogueEntry>,
) -> Result<crate::project::translation_export::ImportMergeResult, String> {
    use crate::project::translation_export::{ImportMergeResult, TranslationExportFile};

    let overwrite = mode == "overwrite";

    let raw = std::fs::read_to_string(&json_path)
        .map_err(|e| format!("Failed to read file: {}", e))?;
    let import: TranslationExportFile = serde_json::from_str(&raw)
        .map_err(|e| format!("Invalid translation.json: {}", e))?;

    let hash_mismatch = !current_bin_sha256.is_empty()
        && import.source_bin_sha256 != current_bin_sha256;

    // Pre-load sibling translation.json files referenced by any row's
    // `external_duplicate_of`. Multi-tab export marks cross-folder duplicates
    // so Claude only translates each unique Japanese once — at import time we
    // need the canonical translation from the sibling folder it was assigned
    // to. The parent-of-the-input's-parent is the export's root directory
    // (one subfolder per BIN), so siblings live at <root>/<folder>/translation.json.
    let mut sibling_files: std::collections::HashMap<String, TranslationExportFile> =
        std::collections::HashMap::new();
    {
        let needed: std::collections::HashSet<String> = import.entries.iter()
            .filter_map(|r| r.external_duplicate_of.as_ref().map(|e| e.folder.clone()))
            .collect();
        if !needed.is_empty() {
            let json_path_buf = std::path::Path::new(&json_path).to_path_buf();
            if let Some(parent) = json_path_buf.parent().and_then(|p| p.parent()) {
                for folder in needed {
                    let sib_path = parent.join(&folder).join("translation.json");
                    let Ok(raw) = std::fs::read_to_string(&sib_path) else { continue };
                    let Ok(parsed) = serde_json::from_str::<TranslationExportFile>(&raw) else { continue };
                    sibling_files.insert(folder, parsed);
                }
            }
        }
    }

    // Build per-id lookup AND per-non-anchor-id group membership lookup.
    // For grouped rows, the anchor entry (first id) gets the full translation
    // and the other members are emptied with a "[Group-translated, see #N]"
    // note so apply.rs knows to write empty strings rather than skipping.
    let mut by_id: std::collections::HashMap<i64, &crate::project::translation_export::TranslationRow> =
        std::collections::HashMap::new();
    // For each non-anchor member id, the anchor id of its group.
    let mut group_member_to_anchor: std::collections::HashMap<i64, i64> =
        std::collections::HashMap::new();
    for row in &import.entries {
        by_id.insert(row.id, row);
        if !row.group_member_ids.is_empty() {
            for &member_id in row.group_member_ids.iter().skip(1) {
                group_member_to_anchor.insert(member_id, row.id);
            }
        }
    }

    let mut merged = entries.clone();
    let mut applied = 0u32;
    let mut skipped_existing = 0u32;
    let mut unchanged = 0u32;
    let mut group_emptied = 0u32;
    let mut matched_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();

    for entry in &mut merged {
        // First: is this entry a non-anchor member of a translated group?
        if let Some(&anchor_id) = group_member_to_anchor.get(&entry.instruction_index) {
            if let Some(anchor_row) = by_id.get(&anchor_id) {
                if !anchor_row.translated.trim().is_empty() {
                    let entry_has_text = !entry.translated_text.trim().is_empty();
                    if !overwrite && entry_has_text {
                        skipped_existing += 1;
                    } else {
                        entry.translated_text = String::new();
                        entry.omit_export = true;
                        entry.notes = format!("[Group-translated, see #{}]", anchor_id);
                        group_emptied += 1;
                    }
                    matched_ids.insert(entry.instruction_index);
                    continue;
                }
            }
        }

        // Otherwise: regular per-id match (anchor entries OR ungrouped rows).
        // Dedup handling: if the row points at a canonical via duplicate_of and
        // the canonical has a translation, propagate it here.
        let row_opt = by_id.get(&entry.instruction_index).copied();
        let effective_translation: Option<String> = match row_opt {
            Some(row) if !row.translated.trim().is_empty() => Some(row.translated.clone()),
            Some(row) => {
                // Empty here — try resolution paths in order:
                //   1. In-folder duplicate (`duplicate_of` → canonical in this file)
                //   2. Cross-folder duplicate (`external_duplicate_of` → sibling)
                let in_folder = row.duplicate_of
                    .and_then(|c| by_id.get(&c))
                    .filter(|c| !c.translated.trim().is_empty())
                    .map(|c| c.translated.clone());
                if in_folder.is_some() {
                    in_folder
                } else if let Some(ext) = &row.external_duplicate_of {
                    sibling_files.get(&ext.folder)
                        .and_then(|sf| sf.entries.iter().find(|r| r.id == ext.id))
                        .filter(|c| !c.translated.trim().is_empty())
                        .map(|c| c.translated.clone())
                } else {
                    None
                }
            }
            None => None,
        };

        match (row_opt, effective_translation) {
            (Some(_), Some(text)) => {
                matched_ids.insert(entry.instruction_index);
                let entry_has_text = !entry.translated_text.trim().is_empty();
                if !overwrite && entry_has_text {
                    skipped_existing += 1;
                } else {
                    entry.translated_text = text;
                    if let Some(row) = row_opt {
                        if !row.notes.trim().is_empty() {
                            entry.notes = row.notes.clone();
                        }
                    }
                    applied += 1;
                }
            }
            (Some(_), None) => {
                unchanged += 1;
                matched_ids.insert(entry.instruction_index);
            }
            (None, _) => {
                unchanged += 1;
            }
        }
    }

    // Roll group-empties into the applied count for user-facing reporting —
    // they're a side effect of translating the group, not "unchanged".
    applied += group_emptied;

    // Auto-omit pass: after import, any editable DIALOGUE entry whose
    // translation is still empty but whose original has text gets
    // omit_export=true. This gives the user a clean preview/export by
    // default — untranslated rows won't leak their original JP into the
    // in-game render. User can flip omit_export back off per-row or via
    // the bulk button if they want JP fallback for specific rows.
    //
    // SAFETY GATE — TWO LAYERS:
    //   1. File-level: skip the whole pass if `applied == 0` (empty/broken
    //      import, e.g. blank translation.json dropped in by accident).
    //   2. Block-level: only auto-omit rows inside dialog blocks where at
    //      least ONE editable row already has a translation. A block where
    //      NO rows are translated hasn't been started yet — falsely
    //      omitting those would misrepresent it as "complete" and force
    //      the user to manually un-toggle each row to recover.
    //
    // Block boundary = `wait-for-input` opcode (same definition the rest of
    // the app uses for dialog blocks).
    //
    // CRITICAL: `set-string` is EXCLUDED from auto-omit. These instructions
    // populate game-state slots (especially speaker name slots that the
    // engine wraps with 【】 brackets). Emptying them produces visible
    // breakage like "【】 Liselle" — the engine renders the bracket
    // template around an empty string, with the actual name appearing
    // elsewhere. For set-string, empty translation means "leave the JP
    // value as-is".
    if applied > 0 {
        const AUTO_OMIT_LABELS: [&str; 3] = [
            "show-text", "display-furigana", "comment",
        ];

        // First pass: assign a block_id to each entry index and record which
        // blocks contain at least one translated editable row. A block
        // starts after a wait-for-input opcode (or at the file start) and
        // extends until the NEXT wait-for-input.
        let mut block_id_of: Vec<usize> = Vec::with_capacity(merged.len());
        let mut current_block: usize = 0;
        let mut block_has_translation: std::collections::HashSet<usize> =
            std::collections::HashSet::new();
        for entry in &merged {
            block_id_of.push(current_block);
            if AUTO_OMIT_LABELS.contains(&entry.label.as_str())
                && !entry.translated_text.trim().is_empty()
            {
                block_has_translation.insert(current_block);
            }
            if entry.label == "wait-for-input" {
                current_block += 1;
            }
        }

        // Second pass: omit empties only in blocks that have at least one
        // translation. Blocks the user hasn't touched stay untouched so
        // their progress numbers reflect reality.
        for (i, entry) in merged.iter_mut().enumerate() {
            if entry.deleted || entry.is_context_row { continue; }
            if !AUTO_OMIT_LABELS.contains(&entry.label.as_str()) { continue; }
            if !entry.translated_text.trim().is_empty() { continue; }
            if entry.original_text.trim().is_empty() { continue; }
            if entry.omit_export { continue; } // already set (e.g. by group flow)
            if is_structural_only(&entry.original_text) { continue; }
            // Block-level gate: untouched block → leave alone.
            if !block_has_translation.contains(&block_id_of[i]) { continue; }
            entry.omit_export = true;
        }
    }

    let unknown_ids = import.entries.iter()
        .filter(|r| !matched_ids.contains(&r.id))
        .count() as u32;

    Ok(ImportMergeResult {
        entries: merged,
        applied,
        unknown_ids,
        unchanged,
        skipped_existing,
        hash_mismatch,
        source_bin: import.source_bin,
    })
}
