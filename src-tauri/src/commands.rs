use crate::bin_format::parser::{parse_bin, ParsedBin};
use crate::bin_format::writer::write_bin;
use crate::claude::client::{ApiProvider, ClaudeClient};
use crate::claude::prompt::TranslationOptions;
use crate::claude::session::{SessionControl, TranslationSession};
use crate::editor::apply::apply_changes;
use crate::editor::characters::{extract_all_character_definitions, extract_ciinit_character_names, extract_referenced_speakers, detect_speaker_lookup_array, detect_speaker_id_variable, extract_referenced_speakers_sys5, extract_referenced_speakers_sys5_legacy, extract_referenced_speakers_sys3, detect_speaker_display_slot, extract_referenced_speakers_1d2};
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
    /// AGE engine build version from the signature (e.g. 443). 0 if unparsed.
    pub engine_version: u16,
    /// UI label: "Madou Koukaku · SYS4 (v443)" (game name when the build is known).
    pub engine_label: String,
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
    let engine_version = parsed.header.engine_version;
    let engine_label = crate::editor::game_profile::engine_label(sys_version, engine_version);

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

    Ok(LoadBinResult { entries, sys_version, engine_version, engine_label })
}

#[derive(serde::Serialize)]
pub struct EngineInfo {
    pub sys_version: u8,
    pub engine_version: u16,
    pub engine_label: String,
}

/// Engine/build info for an already-loaded tab — used by the project-restore path
/// (which loads the BIN via `load_bin_for_project` and so doesn't get a
/// `LoadBinResult`) to populate the UI engine indicator.
#[tauri::command]
pub async fn get_engine_info(tab_id: String, state: State<'_, AppState>) -> Result<EngineInfo, String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    let h = &tab.parsed_bin.as_ref().ok_or("No BIN file loaded")?.header;
    Ok(EngineInfo {
        sys_version: h.sys_version,
        engine_version: h.engine_version,
        engine_label: crate::editor::game_profile::engine_label(h.sys_version, h.engine_version),
    })
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
) -> Result<Vec<String>, String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    let parsed_bin = tab.parsed_bin.as_ref().ok_or("No BIN file loaded")?;

    // Build the translated bytes with the corruption guardrail (see helper).
    // Returns the warn-only export-lint findings to the frontend.
    let TranslatedBin { bytes: output_data, lint_warnings } = build_translated_bin(parsed_bin, &entries, &inserted_instructions)
        .map_err(|e| format!("Export aborted — {e}. No file written (the original BIN is untouched)."))?;

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

    Ok(lint_warnings)
}

/// Build a translated BIN's bytes from a parsed BIN + edits, applying the SAME
/// re-parse corruption guardrail `export_bin` uses: the bytes must re-parse to an
/// identical instruction count or this returns an error instead of corrupt data.
/// Shared by single-file export and `.fanpatch` export so the guardrail can never
/// diverge between them.
pub fn build_translated_bin(
    parsed_bin: &ParsedBin,
    entries: &[DialogueEntry],
    inserted_instructions: &[InsertedInstruction],
) -> Result<TranslatedBin, String> {
    let applied = apply_changes(&parsed_bin.instructions, entries, inserted_instructions);
    // SYS3: inserted click-waits close their line first (as every native wait does),
    // inserted clears re-blit the window like the native clear (else old text stays), and
    // continuation boxes repeat the box's inline name line (Meishoku / Minefuka).
    let applied = if parsed_bin.header.sys_version <= 3 {
        let closed = crate::editor::bridge::close_lines_before_inserted_waits(applied);
        let redrawn = crate::editor::bridge::redraw_box_after_inserted_clears(closed);
        crate::editor::bridge::carry_inline_speaker_names(&parsed_bin.instructions, redrawn)
    } else {
        applied
    };
    let modified = crate::editor::bridge::resolve_speaker_redraws(applied);
    // Warn-only semantic checks (delta vs the original) — see editor::lint.
    let lint_warnings = crate::editor::lint::lint_export(&parsed_bin.instructions, &modified);
    let bytes = write_bin(&parsed_bin.header, &modified)
        .map_err(|e| format!("failed to write BIN: {e}"))?;
    match parse_bin(&bytes) {
        Ok(reparsed) => {
            // Every jump/call/callback target must land on an instruction start.
            let unresolved = crate::bin_format::writer::unresolved_code_targets(&reparsed.instructions);
            if !unresolved.is_empty() {
                return Err(format!(
                    "internal corruption check failed: {} code target(s) point mid-instruction (first: {})",
                    unresolved.len(),
                    unresolved[0]
                ));
            }
            if reparsed.instructions.len() != modified.len() {
                return Err(format!(
                    "internal corruption check failed: wrote {} instructions but read back {} \
                     (the BIN would not render correctly in-game)",
                    modified.len(),
                    reparsed.instructions.len()
                ));
            }
        }
        Err(e) => return Err(format!("the result does not re-parse ({e}) — corruption bug")),
    }
    Ok(TranslatedBin { bytes, lint_warnings })
}

/// Output of `build_translated_bin`: the verified bytes plus warn-only lint
/// findings (`editor::lint`) for the frontend to surface.
pub struct TranslatedBin {
    pub bytes: Vec<u8>,
    pub lint_warnings: Vec<String>,
}

/// One BIN to pack into a `.fanpatch`: the tab to build from + the request-name
/// the game opens it by (zipped uppercased to match the loader's normalisation).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanpatchItem {
    pub tab_id: String,
    pub filename: String,
    pub entries: Vec<DialogueEntry>,
    pub inserted_instructions: Vec<InsertedInstruction>,
}

#[derive(serde::Serialize)]
pub struct FanpatchResult {
    pub written: usize,
    /// "<NAME>: <reason>" for each BIN that was skipped (corruption guard, dup, etc.).
    pub skipped: Vec<String>,
    /// True if version.dll was written next to the patch (the drop-in loader).
    pub loader_written: bool,
    /// "<NAME>: <warning>" export-lint findings (warn-only; the BIN was still written).
    pub lint_warnings: Vec<String>,
    /// Files the text box profile added (system script + untranslated dialogue scenes);
    /// exported scenes carry the profile too.
    pub box_profile_files: usize,
}

/// The fanpatch loader, bundled so an exported `.fanpatch` is a complete drop-in
/// (the user never has to hunt for version.dll). Silent unless an `agepatch.trace`
/// marker is present; Ctrl+Shift+L toggles EN/JP in-game.
///
/// `version.dll` is a prebuilt binary kept OUT of the public source repo; build.rs
/// sets the `has_bundled_loader` cfg only when it's present. A public-repo build
/// has no DLL, so this is an empty slice and `write_loader_package` skips writing
/// it (the rest of the package — README/config — is still produced).
#[cfg(has_bundled_loader)]
const BUNDLED_LOADER_DLL: &[u8] = include_bytes!("../assets/version.dll");
#[cfg(not(has_bundled_loader))]
const BUNDLED_LOADER_DLL: &[u8] = &[];

const FANPATCH_README: &str = "\
Eushully Fanpatch — install
===========================

1. Copy version.dll AND the .fanpatch file(s) into your game folder
   (the folder containing the game's .exe / AGE.EXE).
2. Launch the game. The translation loads automatically — no loose files.

Language toggle: press the ` (backtick) key in-game to switch English <-> Japanese
(applies to the next line/scene loaded). The window title bar shows the current
language, e.g. \"... [Fanpatch - English]\".

Multiple patches load alphabetically; later wins on conflicts. Prefix to order:
  01_english.fanpatch, 02_uncensor.fanpatch, ...

Uninstall: delete version.dll and the .fanpatch file(s). The base game is
completely untouched.
";

/// Write the bundled loader (version.dll) + a README next to the exported patch
/// so it's a complete drop-in package. Non-fatal: failure (e.g. file locked) is
/// reported but never aborts the export — the .fanpatch is the primary artifact.
fn write_loader_package(output_path: &str) -> bool {
    let dir = std::path::Path::new(output_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    // Empty in a public/source build without version.dll → skip the DLL (report
    // it as not written) but still drop the README/config below.
    let dll_ok = !BUNDLED_LOADER_DLL.is_empty()
        && std::fs::write(dir.join("version.dll"), BUNDLED_LOADER_DLL).is_ok();
    let _ = std::fs::write(dir.join("README - fanpatch install.txt"), FANPATCH_README);
    // The title/label config (fanpatch_title.txt) is bundled INSIDE the .fanpatch
    // zip (see bundle_loader_configs) from the in-editor Title/Label fields, and
    // the loader reads the bundled copy first. So we no longer drop a loose
    // template here — keeps the shipped package to version.dll + README + .fanpatch.
    dll_ok
}

/// Opt-in loader features bundled into a `.fanpatch` as `fanpatch_features.txt`
/// (read by the loader's `Feature_Enabled`). See docs/engine/age-sys.md.
#[derive(serde::Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct FanpatchFeatures {
    /// Resizable, aspect-locked window + HQ upscaler (verified on SYS4 438/443).
    #[serde(default)]
    pub resizable_window: bool,
    /// Developer menu + F2/F5/F6/F8/F11 hotkeys, and the game's own developer menu
    /// (scene jump) unlocked behind the title's Quit button (testing builds only).
    #[serde(default)]
    pub dev_menu: bool,
    /// Crisp HD rendering: the engine's own hi-res virtual-fullscreen mode run in the
    /// resizable window (`native_hires_window`), plus the internal-resolution fallback
    /// for the classic windowed mode / builds without it (`internal_resolution`).
    /// Needs `resizable_window` (the loader's scaler). Verified on SYS4 438 (Atelial).
    #[serde(default)]
    pub crisp_hd: bool,
    /// English word wrap: the loader hooks the SYS4 typewriter's per-character wrap
    /// check and wraps before an ASCII word that would not fit (docs/engine/age-sys.md
    /// §7b). Signature found in Atelial 438, Madou, Sankai builds; inert elsewhere.
    #[serde(default)]
    pub english_word_wrap: bool,
}

/// `fanpatch_features.txt` contents (`key = 0|1` lines, parsed by the loader's
/// `Feature_Enabled`), or `None` when every feature is off.
fn features_file(features: FanpatchFeatures) -> Option<String> {
    if !(features.resizable_window || features.dev_menu || features.english_word_wrap) {
        return None;
    }
    let crisp_hd = features.crisp_hd && features.resizable_window;
    Some(format!(
        "# Fanpatch loader features (written by Eushully BIN Editor)\r\nresizable_window = {}\r\ndev_menu = {}\r\nnative_hires_window = {}\r\ninternal_resolution = {}\r\nenglish_word_wrap = {}\r\n",
        u8::from(features.resizable_window),
        u8::from(features.dev_menu),
        u8::from(crisp_hd),
        u8::from(crisp_hd),
        u8::from(features.english_word_wrap)
    ))
}

/// Bundle the loader's runtime config files INSIDE the `.fanpatch` zip so a
/// shipped patch is just `version.dll` + one `.fanpatch` (no loose config files
/// to hunt for). The loader reads `fanpatch_title.txt` (title/label for the
/// window title bar) and `fanpatch_ui.txt` (UI translation table) from inside
/// the zip. Both are optional:
/// - `fanpatch_title.txt` is written only when a title or label is supplied.
/// - `fanpatch_ui.txt` is written only when `ui_config_path` is set AND readable
///   (a missing/unreadable file is silently skipped — non-fatal, the rest of the
///   patch still ships).
fn bundle_loader_configs<W: std::io::Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    opts: zip::write::SimpleFileOptions,
    title: &str,
    label: &str,
    ui_config_path: &str,
    features: FanpatchFeatures,
) -> Result<(), String> {
    use std::io::Write;

    if let Some(contents) = features_file(features) {
        zip.start_file("fanpatch_features.txt", opts)
            .map_err(|e| format!("zip entry fanpatch_features.txt: {e}"))?;
        zip.write_all(contents.as_bytes())
            .map_err(|e| format!("zip write fanpatch_features.txt: {e}"))?;
    }

    let title = title.trim();
    let label = label.trim();
    if !title.is_empty() || !label.is_empty() {
        let mut contents = String::new();
        if !title.is_empty() {
            contents.push_str(&format!("title = {title}\r\n"));
        }
        if !label.is_empty() {
            contents.push_str(&format!("label = {label}\r\n"));
        }
        zip.start_file("fanpatch_title.txt", opts)
            .map_err(|e| format!("zip entry fanpatch_title.txt: {e}"))?;
        zip.write_all(contents.as_bytes())
            .map_err(|e| format!("zip write fanpatch_title.txt: {e}"))?;
    }

    let ui_config_path = ui_config_path.trim();
    if !ui_config_path.is_empty() {
        if let Ok(bytes) = std::fs::read(ui_config_path) {
            zip.start_file("fanpatch_ui.txt", opts)
                .map_err(|e| format!("zip entry fanpatch_ui.txt: {e}"))?;
            zip.write_all(&bytes)
                .map_err(|e| format!("zip write fanpatch_ui.txt: {e}"))?;
        }
    }

    Ok(())
}

/// Export a `.fanpatch` (a ZIP of translated BINs, flat, request-named) for the
/// version.dll fanpatch loader. Each BIN is built with the corruption guardrail;
/// any that fail are skipped (reported) rather than poisoning the whole patch.
/// A text box profile being applied to a `.fanpatch` export (docs/engine/age-sys.md §7b):
/// each BIN written gets the profile, then the system script and every other dialogue
/// scene of the base game are appended. Shared by the tab and folder exports.
struct BoxProfileExport {
    profile: crate::engine::box_profile::BoxProfile,
    root: std::path::PathBuf,
    geometry: crate::engine::box_profile::BoxGeometry,
}

impl BoxProfileExport {
    fn prepare(profile: Option<crate::engine::box_profile::BoxProfile>, root: Option<std::path::PathBuf>) -> Result<Option<Self>, String> {
        let Some(profile) = profile else { return Ok(None) };
        let root = root.ok_or("text box profile: game folder not found (open the game folder once)")?;
        let geometry = crate::engine::base_box_geometry(&root)?;
        Ok(Some(Self { profile, root, geometry }))
    }

    /// The profile applied to one exported BIN (`name` = request name).
    fn apply(&self, name: &str, bytes: Vec<u8>) -> Result<Vec<u8>, String> {
        crate::engine::apply_box_profile_to_scene(&bytes, &self.geometry, &self.profile, crate::engine::is_system_script(name))
            .map_err(|e| format!("{name}: text box profile: {e}"))
    }

    /// Append the patched system script + other dialogue scenes not already written.
    fn append_rest<W: std::io::Write + std::io::Seek>(
        &self,
        zip: &mut zip::ZipWriter<W>,
        opts: zip::write::SimpleFileOptions,
        written: &std::collections::HashSet<String>,
    ) -> Result<usize, String> {
        use std::io::Write;
        let patch = crate::engine::box_profile_patch(&self.root, &self.profile, written)?;
        let count = patch.files.len();
        for (name, bytes) in patch.files {
            zip.start_file(&name, opts).map_err(|e| format!("zip entry {name}: {e}"))?;
            zip.write_all(&bytes).map_err(|e| format!("zip write {name}: {e}"))?;
        }
        Ok(count)
    }
}


/// Testing builds ("Developer menu"): add the game's TITLE.BIN with the developer menu
/// unlocked behind the Quit button (engine::dev_title — scene jump via the SC????.BIN
/// picker). Skipped when the export already carries a TITLE.BIN or the game is unknown.
fn append_dev_title<W: std::io::Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    opts: zip::write::SimpleFileOptions,
    features: Option<FanpatchFeatures>,
    root: Option<&std::path::Path>,
    profile: Option<&BoxProfileExport>,
    seen: &mut std::collections::HashSet<String>,
    skipped: &mut Vec<String>,
) -> Result<(), String> {
    use std::io::Write;
    const TITLE: &str = "TITLE.BIN";
    if !features.is_some_and(|f| f.dev_menu) {
        return Ok(());
    }
    if seen.contains(TITLE) {
        skipped.push(format!("{TITLE}: exported translation kept — developer menu not unlocked"));
        return Ok(());
    }
    let Some(root) = root else {
        skipped.push("developer menu: game folder unknown — TITLE.BIN not unlocked".to_string());
        return Ok(());
    };
    let Some(bytes) = crate::engine::dev_title_patch(root, None)? else {
        skipped.push("developer menu: no developer menu found in TITLE.BIN".to_string());
        return Ok(());
    };
    let bytes = match profile {
        Some(profile) => profile.apply(TITLE, bytes)?,
        None => bytes,
    };
    zip.start_file(TITLE, opts).map_err(|e| format!("zip entry {TITLE}: {e}"))?;
    zip.write_all(&bytes).map_err(|e| format!("zip write {TITLE}: {e}"))?;
    seen.insert(TITLE.to_string());
    Ok(())
}

#[tauri::command]
pub async fn export_fanpatch(
    items: Vec<FanpatchItem>,
    output_path: String,
    title: String,
    label: String,
    ui_config_path: String,
    features: Option<FanpatchFeatures>,
    box_profile: Option<crate::engine::box_profile::BoxProfile>,
    game_dir_hint: Option<String>,
    state: State<'_, AppState>,
) -> Result<FanpatchResult, String> {
    use std::io::Write;

    // Text box profile (docs/engine/age-sys.md §7b): computed from the base game files.
    let first_bin = {
        let tabs = state.tabs.lock().unwrap();
        items.iter().find_map(|item| tabs.get(&item.tab_id).and_then(|t| t.bin_path.clone()))
    };
    let profile = BoxProfileExport::prepare(box_profile, tab_game_root(first_bin.as_deref(), game_dir_hint.as_deref()))?;

    let file = std::fs::File::create(&output_path)
        .map_err(|e| format!("Failed to create {output_path}: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let tabs = state.tabs.lock().unwrap();
    let mut written = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    let mut lint_warnings: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for item in &items {
        // Request-name = basename, uppercased (mirrors the loader's NormalizeA).
        let name = item
            .filename
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&item.filename)
            .to_uppercase();

        let Some(tab) = tabs.get(&item.tab_id) else {
            skipped.push(format!("{name}: tab not found"));
            continue;
        };
        let Some(parsed) = tab.parsed_bin.as_ref() else {
            skipped.push(format!("{name}: no BIN loaded"));
            continue;
        };
        match build_translated_bin(parsed, &item.entries, &item.inserted_instructions) {
            Ok(TranslatedBin { bytes, lint_warnings: bin_warnings }) => {
                if !seen.insert(name.clone()) {
                    skipped.push(format!("{name}: duplicate name (kept first)"));
                    continue;
                }
                let bytes = match &profile {
                    Some(profile) => profile.apply(&name, bytes)?,
                    None => bytes,
                };
                zip.start_file(&name, opts)
                    .map_err(|e| format!("zip entry {name}: {e}"))?;
                zip.write_all(&bytes)
                    .map_err(|e| format!("zip write {name}: {e}"))?;
                written += 1;
                lint_warnings.extend(bin_warnings.into_iter().map(|w| format!("{name}: {w}")));
            }
            Err(e) => skipped.push(format!("{name}: {e}")),
        }
    }
    drop(tabs);
    let root = tab_game_root(first_bin.as_deref(), game_dir_hint.as_deref());
    append_dev_title(&mut zip, opts, features, root.as_deref(), profile.as_ref(), &mut seen, &mut skipped)?;
    let box_profile_files = match &profile {
        Some(profile) => profile.append_rest(&mut zip, opts, &seen)?,
        None => 0,
    };
    bundle_loader_configs(&mut zip, opts, &title, &label, &ui_config_path, features.unwrap_or_default())?;
    zip.finish()
        .map_err(|e| format!("Failed to finalize fanpatch: {e}"))?;

    let loader_written = write_loader_package(&output_path);
    Ok(FanpatchResult { written, skipped, loader_written, lint_warnings, box_profile_files })
}

/// Package every file in `source_dir` (flat, non-recursive) into a `.fanpatch`,
/// names uppercased to match the loader. `.BIN` files are validated (must parse)
/// so a corrupt BIN can't silently ship; non-BIN assets (images, etc.) pass
/// through untouched. Use this to zip an already-translated folder of BINs.
#[tauri::command]
pub async fn pack_fanpatch_from_dir(
    source_dir: String,
    output_path: String,
    title: String,
    label: String,
    ui_config_path: String,
    features: Option<FanpatchFeatures>,
    box_profile: Option<crate::engine::box_profile::BoxProfile>,
    game_dir_hint: Option<String>,
) -> Result<FanpatchResult, String> {
    use std::io::Write;

    let profile = BoxProfileExport::prepare(box_profile, tab_game_root(Some(&source_dir), game_dir_hint.as_deref()))?;
    let file = std::fs::File::create(&output_path)
        .map_err(|e| format!("Failed to create {output_path}: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut written = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    let lint_warnings: Vec<String> = Vec::new(); // no original BIN to diff against → nothing to lint
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let dir = std::fs::read_dir(&source_dir)
        .map_err(|e| format!("Failed to read {source_dir}: {e}"))?;
    for entry in dir.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(fname) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let name = fname.to_uppercase();
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                skipped.push(format!("{name}: read error ({e})"));
                continue;
            }
        };
        // Guard .BIN assets: a non-parsing BIN would crash the game in-engine.
        if name.ends_with(".BIN") && parse_bin(&bytes).is_err() {
            skipped.push(format!("{name}: not a valid BIN (skipped)"));
            continue;
        }
        if !seen.insert(name.clone()) {
            skipped.push(format!("{name}: duplicate name (kept first)"));
            continue;
        }
        let bytes = match (&profile, name.ends_with(".BIN")) {
            (Some(profile), true) => profile.apply(&name, bytes)?,
            _ => bytes,
        };
        zip.start_file(&name, opts)
            .map_err(|e| format!("zip entry {name}: {e}"))?;
        zip.write_all(&bytes)
            .map_err(|e| format!("zip write {name}: {e}"))?;
        written += 1;
    }
    let root = tab_game_root(Some(&source_dir), game_dir_hint.as_deref());
    append_dev_title(&mut zip, opts, features, root.as_deref(), profile.as_ref(), &mut seen, &mut skipped)?;
    let box_profile_files = match &profile {
        Some(profile) => profile.append_rest(&mut zip, opts, &seen)?,
        None => 0,
    };
    bundle_loader_configs(&mut zip, opts, &title, &label, &ui_config_path, features.unwrap_or_default())?;
    zip.finish()
        .map_err(|e| format!("Failed to finalize fanpatch: {e}"))?;

    let loader_written = write_loader_package(&output_path);
    Ok(FanpatchResult { written, skipped, loader_written, lint_warnings, box_profile_files })
}

/// Build a `.fanpatch` from a folder of translated (English) BINs, applying the
/// dual-language pipeline against the matching Japanese originals in `jp_dir`:
/// - `bilingual=false`: narrator-box repair only (continuation boxes regain the
///   speaker's portrait/name/color).
/// - `bilingual=true`: repair + bilingual (each scene switches EN↔JP mid-scene via
///   the loader's backtick).
/// For each EN BIN, the JP original is `jp_dir/<same name>`. BINs that can't be
/// paired or don't align cleanly are shipped unchanged (reported in `skipped`).
#[tauri::command]
pub async fn pack_dual_language_fanpatch(
    en_dir: String,
    jp_dir: String,
    agf_dir: String,
    bilingual: bool,
    output_path: String,
    title: String,
    label: String,
    ui_config_path: String,
    features: Option<FanpatchFeatures>,
) -> Result<FanpatchResult, String> {
    use crate::editor::dual_language;
    use std::io::Write;

    let file = std::fs::File::create(&output_path)
        .map_err(|e| format!("Failed to create {output_path}: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut written = 0usize;
    let mut processed = 0usize; // BINs that got repaired/bilingual
    let mut skipped: Vec<String> = Vec::new();
    let lint_warnings: Vec<String> = Vec::new(); // no original BIN to diff against → nothing to lint
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let dir = std::fs::read_dir(&en_dir).map_err(|e| format!("Failed to read {en_dir}: {e}"))?;
    for entry in dir.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(fname) = p.file_name().and_then(|n| n.to_str()) else { continue };
        let name = fname.to_uppercase();
        let Ok(en_bytes) = std::fs::read(&p) else {
            skipped.push(format!("{name}: read error"));
            continue;
        };
        if !seen.insert(name.clone()) {
            skipped.push(format!("{name}: duplicate name (kept first)"));
            continue;
        }

        // Non-BIN assets ship as-is; BINs go through the pipeline.
        let out_bytes = if name.ends_with(".BIN") {
            match parse_bin(&en_bytes) {
                Ok(en) => {
                    let jp_path = std::path::Path::new(&jp_dir).join(fname);
                    match std::fs::read(&jp_path).ok().and_then(|d| parse_bin(&d).ok()) {
                        Some(jp) => {
                            let result = if bilingual {
                                dual_language::make_bilingual(&en, &jp)
                            } else {
                                dual_language::make_repaired(&en, &jp)
                            };
                            match result {
                                Ok(b) => {
                                    processed += 1;
                                    b
                                }
                                Err(reason) => {
                                    skipped.push(format!("{name}: shipped as-is ({reason})"));
                                    en_bytes
                                }
                            }
                        }
                        None => {
                            skipped.push(format!("{name}: no JP original in jp_dir — shipped as-is"));
                            en_bytes
                        }
                    }
                }
                Err(_) => {
                    skipped.push(format!("{name}: not a valid BIN (skipped)"));
                    continue;
                }
            }
        } else {
            en_bytes
        };

        zip.start_file(&name, opts).map_err(|e| format!("zip entry {name}: {e}"))?;
        zip.write_all(&out_bytes).map_err(|e| format!("zip write {name}: {e}"))?;
        written += 1;
    }

    // Optionally bundle translated assets (AGF UI/CG images, etc.) as gated files:
    // the loader serves them in English and falls through to the original (.alf) in
    // Japanese, so the whole UI flips with the language.
    let mut assets = 0usize;
    if !agf_dir.trim().is_empty() {
        if let Ok(adir) = std::fs::read_dir(&agf_dir) {
            for entry in adir.flatten() {
                let p = entry.path();
                if !p.is_file() {
                    continue;
                }
                let Some(fname) = p.file_name().and_then(|n| n.to_str()) else { continue };
                let name = fname.to_uppercase();
                if !seen.insert(name.clone()) {
                    skipped.push(format!("{name}: duplicate name (kept first)"));
                    continue;
                }
                let Ok(bytes) = std::fs::read(&p) else {
                    skipped.push(format!("{name}: read error"));
                    continue;
                };
                zip.start_file(&name, opts).map_err(|e| format!("zip entry {name}: {e}"))?;
                zip.write_all(&bytes).map_err(|e| format!("zip write {name}: {e}"))?;
                written += 1;
                assets += 1;
            }
        } else {
            skipped.push(format!("asset folder not readable: {agf_dir}"));
        }
    }
    bundle_loader_configs(&mut zip, opts, &title, &label, &ui_config_path, features.unwrap_or_default())?;
    zip.finish().map_err(|e| format!("Failed to finalize fanpatch: {e}"))?;

    skipped.insert(0, format!("{} BIN(s) {} + {} asset(s)", processed, if bilingual { "made bilingual" } else { "repaired" }, assets));
    let loader_written = write_loader_package(&output_path);
    Ok(FanpatchResult { written, skipped, loader_written, lint_warnings, box_profile_files: 0 })
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

/// Installed game folder for a tab: the game holding its BIN, else `game_dir_hint`
/// (workspace tabs are copies outside the game).
fn tab_game_root(bin_path: Option<&str>, game_dir_hint: Option<&str>) -> Option<std::path::PathBuf> {
    bin_path
        .and_then(|p| crate::engine::game_root_of(std::path::Path::new(p)))
        .or_else(|| game_dir_hint.filter(|h| !h.is_empty()).and_then(|h| crate::engine::game_root_of(std::path::Path::new(h))))
}

/// The game's speaker tables, parsed once per game folder + change stamp.
fn game_speaker_tables(state: &AppState, root: &std::path::Path) -> std::sync::Arc<crate::engine::GameSpeakerTables> {
    let key = format!("{}|{}", root.display(), crate::engine::game_files_stamp(root));
    if let Some(tables) = state.speaker_tables.lock().unwrap().get(&key) {
        return tables.clone();
    }
    let tables = std::sync::Arc::new(crate::engine::GameSpeakerTables::load(root));
    let mut cache = state.speaker_tables.lock().unwrap();
    let prefix = format!("{}|", root.display());
    cache.retain(|k, _| !k.starts_with(&prefix)); // drop stale stamps of this game
    cache.insert(key, tables.clone());
    tables
}

/// Talk-key speakers (SYS4 438, Atelial): per-row character indices + their names,
/// resolved through the installed game's INIT2/CFINIT/CHINIT. `None` when the scene
/// doesn't use the scheme or the game folder can't be found.
fn talk_key_speakers_for(
    state: &AppState,
    instructions: &[crate::bin_format::instruction::Instruction],
    bin_path: Option<&str>,
    game_dir_hint: Option<&str>,
) -> Option<(std::collections::BTreeMap<usize, crate::editor::characters::SpeakerRef>, Vec<CharacterEntry>)> {
    use crate::editor::characters::{detect_talk_key_scheme, talk_key_row_speakers, SpeakerRef};
    let scheme = detect_talk_key_scheme(instructions)?;
    let root = tab_game_root(bin_path, game_dir_hint)?;
    let speakers = game_speaker_tables(state, &root).talk_key_speakers(&root, scheme.table)?;
    let mut rows = std::collections::BTreeMap::new();
    for (index, character) in talk_key_row_speakers(instructions, &scheme, &speakers.owners) {
        rows.insert(index, SpeakerRef { source_type: 3, source_raw_data: character, literal_name: None });
    }
    let mut ids: Vec<u32> = rows.values().map(|r| r.source_raw_data).collect();
    ids.sort_unstable();
    ids.dedup();
    let characters = ids
        .into_iter()
        .map(|id| CharacterEntry { pointer: id, name: speakers.names.get(&id).cloned().unwrap_or_default() })
        .collect();
    Some((rows, characters))
}

/// Names for numeric speaker ids, from the game's name tables (CNINIT string table /
/// CHINIT ID register, discovered per game — see `discover_speaker_names`). Looks in the
/// installed game holding the tab's BIN, then `game_dir_hint`, then beside the BIN
/// (`$N$` append copies matching the BIN's own prefix first, then base copies).
#[tauri::command]
pub async fn resolve_speaker_names(
    tab_id: String,
    ids: Vec<u32>,
    game_dir_hint: Option<String>,
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<u32, String>, String> {
    let bin_path = {
        let tabs = state.tabs.lock().unwrap();
        tabs.get(&tab_id).ok_or("Tab not found")?.bin_path.clone()
    };
    if let Some(root) = tab_game_root(bin_path.as_deref(), game_dir_hint.as_deref()) {
        let tables = game_speaker_tables(&state, &root);
        return Ok(crate::editor::characters::discover_speaker_names(&tables.name_scripts, &ids));
    }
    let bin = bin_path.as_deref().map(std::path::Path::new).ok_or("no folder for this tab")?;
    let dir = bin.parent().ok_or("no folder for this tab")?;
    let append_prefix: String = bin
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .and_then(|n| n.strip_prefix('$').and_then(|rest| rest.find('$').map(|end| format!("${}$", &rest[..end]))))
        .unwrap_or_default();
    let mut scripts = Vec::new();
    for file in crate::editor::characters::NAME_TABLE_FILES {
        let mut candidates = vec![file.to_string()];
        if !append_prefix.is_empty() {
            candidates.insert(0, format!("{append_prefix}{file}"));
        }
        for candidate in candidates {
            if let Some(bin) = std::fs::read(dir.join(&candidate)).ok().and_then(|b| parse_bin(&b).ok()) {
                scripts.push((candidate, bin.instructions));
            }
        }
    }
    Ok(crate::editor::characters::discover_speaker_names(&scripts, &ids))
}

/// Extract all referenced speaker pointers from the loaded BIN.
/// Returns CharacterEntry for each unique speaker — name is empty if not defined in this BIN.
#[tauri::command]
pub async fn get_referenced_speakers(
    tab_id: String,
    game_dir_hint: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<CharacterEntry>, String> {
    let (talk_key_input, bin_path) = {
        let tabs = state.tabs.lock().unwrap();
        let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
        let parsed_bin = tab.parsed_bin.as_ref().ok_or("No BIN file loaded")?;
        let input = (parsed_bin.header.sys_version == 4 && crate::editor::characters::detect_talk_key_scheme(&parsed_bin.instructions).is_some())
            .then(|| parsed_bin.instructions.clone());
        (input, tab.bin_path.clone())
    };
    // SYS4 talk-key scenes (Atelial): speakers live only in the game's INIT tables. Without
    // the game folder report none — the generic detectors below pick unrelated variables
    // in these scenes (audit 2026-11-11).
    if let Some(instructions) = talk_key_input {
        return Ok(talk_key_speakers_for(&state, &instructions, bin_path.as_deref(), game_dir_hint.as_deref())
            .map(|(_, characters)| characters)
            .unwrap_or_default());
    }
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

    // SYS4 fallback: speaker set via set-display-slot (0x1D2), e.g. Megami Zero.
    if let Some(slot) = detect_speaker_display_slot(&parsed_bin.instructions) {
        return Ok(extract_referenced_speakers_1d2(&parsed_bin.instructions, slot));
    }

    // SYS4 fallback: SYS5-style g-int char-ID register (e.g. 戦女神VERITA —
    // `mov g-int:13449, <id>` before each line). It's mov-before-lookup-array,
    // so the set-string register + 0x1D2 paths above both miss it. Strictly
    // additive: only runs when those returned empty.
    if let Some(id_var) = detect_speaker_id_variable(&parsed_bin.instructions) {
        let s = extract_referenced_speakers_sys5_legacy(&parsed_bin.instructions, id_var);
        if !s.is_empty() {
            return Ok(s);
        }
    }

    Ok(Vec::new())
}

/// Extract speaker references for each dialogue entry from the loaded BIN.
#[tauri::command]
pub async fn get_row_speakers(
    tab_id: String,
    game_dir_hint: Option<String>,
    state: State<'_, AppState>,
) -> Result<std::collections::BTreeMap<usize, crate::editor::characters::SpeakerRef>, String> {
    let (talk_key_input, bin_path) = {
        let tabs = state.tabs.lock().unwrap();
        let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
        let parsed_bin = tab.parsed_bin.as_ref().ok_or("No BIN file loaded")?;
        let input = (parsed_bin.header.sys_version == 4 && crate::editor::characters::detect_talk_key_scheme(&parsed_bin.instructions).is_some())
            .then(|| parsed_bin.instructions.clone());
        (input, tab.bin_path.clone())
    };
    // SYS4 talk-key scenes (Atelial): see get_referenced_speakers — none without the game.
    if let Some(instructions) = talk_key_input {
        return Ok(talk_key_speakers_for(&state, &instructions, bin_path.as_deref(), game_dir_hint.as_deref())
            .map(|(rows, _)| rows)
            .unwrap_or_default());
    }
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    let parsed_bin = tab.parsed_bin.as_ref().ok_or("No BIN file loaded")?;

    Ok(crate::editor::characters::detect_row_speakers(&parsed_bin.instructions, parsed_bin.header.sys_version)
        .map(|(_, rows)| rows)
        .unwrap_or_default())
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
/// Core of auto-split: compute a `SplitPlan` for one entry from an
/// already-resolved bridge template + parsed BIN. Shared by `auto_split_entry`
/// and `auto_split_entries_batch` so the batch path computes N plans under a
/// single tab lock (no per-row IPC round-trip). Auto-split is LINE-ONLY: it wraps
/// the overflow into one box of end-text-line-separated lines (the engine
/// paginates); multi-box splitting is the user's explicit `insertTextBoxBridge`.
/// Re-flow against the engine's own row breaking (loader word-wrap hook + reported
/// metrics, docs/engine/age-sys.md §7b): the plan's lines are whole BOX texts.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineSplit {
    pub metrics: crate::engine::text_metrics::EngineTextMetrics,
    pub word_wrap: bool,
    pub rows_per_box: u32,
    pub bottom_cap: u32,
}

fn build_split_plan(
    template: &crate::editor::bridge::BridgeTemplate,
    parsed_bin: &crate::bin_format::parser::ParsedBin,
    translated_text: &str,
    entry_index: usize,
    caps: crate::editor::wrap::RowCaps,
    engine: Option<&EngineSplit>,
) -> Result<crate::editor::bridge::SplitPlan, String> {
    // Line-only plan: all lines in one "box" (pagination is a separate step).
    // - Engine mode: one line per BOX text; the engine wraps each box's rows itself, the
    //   later pagination (1 line per box) opens a box per line.
    // - Otherwise: rows laid out as balanced boxes (even share per box, bottom rows within
    //   the UI-art cap) so the later pagination recovers even boxes.
    let wrap = match engine {
        Some(engine) => {
            let wrap = crate::editor::wrap::EngineWrap { metrics: &engine.metrics, word_wrap: engine.word_wrap };
            let boxes = crate::editor::wrap::engine_boxes(translated_text, wrap, engine.rows_per_box as usize, engine.bottom_cap);
            crate::editor::wrap::WrapOutput { boxes: vec![boxes], overflow_words: Vec::new() }
        }
        None => {
            let rows = crate::editor::wrap::wrap_rows_balanced(translated_text, caps);
            crate::editor::wrap::WrapOutput { boxes: vec![rows.rows], overflow_words: rows.overflow_words }
        }
    };
    if wrap.boxes.is_empty() {
        return Err("No text to split".into());
    }
    if wrap.boxes.len() == 1 && wrap.boxes[0].len() == 1 && wrap.boxes[0][0] == translated_text {
        return Err("Text already fits within the line cap".into());
    }

    // Speaker ID 0 (narrator) is fine — the bridge clone leaves speaker args at 0
    // unless speaker_arg_positions are populated.
    let speaker_id = 0u32;

    // Argument template for continuation show-text instructions. Normally the
    // source entry IS a show-text (0x6E), so we clone its args verbatim. But an
    // overflowing entry can be a display-furigana (0x196) whose group anchor got
    // promoted so it carries a whole line's English. A 0x196 has a 3-arg,
    // two-string-slot shape; using it as a show-text template would emit a ruby
    // instruction with a stale kanji slot. So when the source isn't a show-text,
    // clone the FIRST real 0x6E's arguments instead — a proper single-string
    // show-text shape (mirrors the end_text_line_template fallback below).
    // argument_count stays == arguments.len() via build_instruction_from_insertion.
    const OP_SHOW_TEXT: u32 = 0x6E;
    let show_text_template_args = parsed_bin
        .instructions
        .get(entry_index)
        .filter(|i| i.opcode == OP_SHOW_TEXT)
        .or_else(|| parsed_bin.instructions.iter().find(|i| i.opcode == OP_SHOW_TEXT))
        .map(|i| {
            i.arguments
                .iter()
                .map(|a| crate::editor::bridge::SerializedArg {
                    arg_type: a.arg_type,
                    raw_data: a.raw_data,
                    decoded_string: a.decoded_string.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    // end-text-line (0x6F) template — clone a real one, else synthesize (0x6F is a
    // 1-arg literal-0 opcode, so a synthetic is structurally identical).
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

#[tauri::command]
pub async fn auto_split_entry(
    tab_id: String,
    translated_text: String,
    entry_index: usize,
    line_char_cap: u32,
    block_char_cap: u32,
    max_lines_per_box: u32,
    bottom_line_cap: Option<u32>,
    state: State<'_, AppState>,
) -> Result<crate::editor::bridge::SplitPlan, String> {
    let _ = block_char_cap; // line-only; see build_split_plan
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    if line_char_cap == 0 {
        return Err("Per-line character cap must be set for auto-split".into());
    }
    let template = tab.bridge_template.as_ref().ok_or("No bridge template available")?;
    let parsed_bin = tab.parsed_bin.as_ref().ok_or("No parsed BIN data")?;
    let caps = crate::editor::wrap::RowCaps { line: line_char_cap, bottom: bottom_line_cap.unwrap_or(0), rows_per_box: max_lines_per_box };
    build_split_plan(template, parsed_bin, &translated_text, entry_index, caps, None)
}

/// One overflow entry to plan a split for.
#[derive(serde::Deserialize)]
pub struct SplitRequest {
    pub translated_text: String,
    pub entry_index: usize,
}

/// Result for one batch item: either a plan or a per-item error (so one bad
/// entry doesn't fail the whole batch).
#[derive(serde::Serialize)]
pub struct SplitOutcome {
    pub entry_index: usize,
    pub plan: Option<crate::editor::bridge::SplitPlan>,
    pub error: Option<String>,
}

/// Compute split plans for many overflowing entries in a single call. Replaces N
/// `auto_split_entry` round-trips during a bulk auto-split: one tab lock, one IPC
/// hop, plans computed in Rust. The frontend then applies them synchronously so
/// the editor table repaints once.
#[tauri::command]
pub async fn auto_split_entries_batch(
    tab_id: String,
    items: Vec<SplitRequest>,
    line_char_cap: u32,
    max_lines_per_box: Option<u32>,
    bottom_line_cap: Option<u32>,
    engine: Option<EngineSplit>,
    state: State<'_, AppState>,
) -> Result<Vec<SplitOutcome>, String> {
    let caps = crate::editor::wrap::RowCaps {
        line: line_char_cap,
        bottom: bottom_line_cap.unwrap_or(0),
        rows_per_box: max_lines_per_box.unwrap_or(0),
    };
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    if line_char_cap == 0 {
        return Err("Per-line character cap must be set for auto-split".into());
    }
    let template = tab.bridge_template.as_ref().ok_or("No bridge template available")?;
    let parsed_bin = tab.parsed_bin.as_ref().ok_or("No parsed BIN data")?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match build_split_plan(template, parsed_bin, &item.translated_text, item.entry_index, caps, engine.as_ref()) {
            Ok(plan) => out.push(SplitOutcome { entry_index: item.entry_index, plan: Some(plan), error: None }),
            Err(e) => out.push(SplitOutcome { entry_index: item.entry_index, plan: None, error: Some(e) }),
        }
    }
    Ok(out)
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
        // explorer.exe silently ignores forward-slash paths (it opens the
        // default Documents/home folder instead), and the frontend joins paths
        // with "/" (e.g. `${parentDir}/${folder}`). Normalize to backslashes so
        // the target folder actually opens.
        let native = folder_path.replace('/', "\\");
        std::process::Command::new("explorer")
            .arg(&native)
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

/// One AGE script BIN discovered in a game folder, classified for the SYS
/// folder picker.
#[derive(serde::Serialize)]
pub struct SysBinFile {
    pub path: String,
    pub filename: String,
    /// Filename with the `$N$` append-prefix stripped (the logical name).
    pub base_name: String,
    /// Append layer: `$1$`/`$2$`… override/patch files (N≥1) vs the base game BIN
    /// (0 = no `$N$` prefix). The highest append is what the game loads, so it
    /// wins the dedupe.
    pub layer: u32,
    /// 3, 4, or 5.
    pub sys_version: u8,
    /// Picker bucket: "names" (character/item name tables) | "sc" (story
    /// scenes) | "sp" (special scenes) | "ui" (menu/system text) | "other"
    /// (everything else). See `classify_bin`.
    pub category: String,
    /// Count of TRANSLATABLE strings (non-empty, letter-bearing) on the text
    /// opcodes show-text / display-furigana / comment, plus set-string in
    /// name/item files. 0 ⇒ a data/structural BIN with nothing to translate
    /// (the picker hides these). Drives the picker's per-file line count.
    pub string_count: usize,
    /// Heuristic: the dialogue is mostly Latin (not Japanese) → this loose file
    /// is probably already translated / not the original game data.
    pub likely_translated: bool,
    pub size: u64,
    /// Set only for entries packed in a `.alf` archive (not loose on disk):
    /// which data volume + where, so the frontend can extract them on open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alf_archive_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alf_offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alf_length: Option<u64>,
}

/// Split an Eushully update-layer filename: `"$2$NAME.BIN"` -> `(2, "NAME.BIN")`.
/// Unprefixed names -> `(0, name)`.
fn parse_update_layer(filename: &str) -> (u32, String) {
    if let Some(rest) = filename.strip_prefix('$') {
        if let Some(idx) = rest.find('$') {
            if let Ok(n) = rest[..idx].parse::<u32>() {
                return (n, rest[idx + 1..].to_string());
            }
        }
    }
    (0, filename.to_string())
}

/// Filename prefixes (uppercased, append-prefix stripped) marking a character/
/// item NAME table where `set-string` (0x192) writes ARE translatable content.
/// Mirrors the frontend `NAME_ITEM_PREFIXES` in `src/lib/fileCategory.ts` and
/// drives both the `names` bucket in `classify_bin` and the set-string string
/// count in `scan_bin_folder`.
const NAME_ITEM_PREFIXES: [&str; 7] =
    ["CVINIT", "CIINIT", "CNINIT", "CHINIT", "AFINIT", "ITINIT", "ITMES"];

/// True when the uppercased logical base name is a character/item NAME table.
fn is_name_item(base_upper: &str) -> bool {
    NAME_ITEM_PREFIXES.iter().any(|p| base_upper.starts_with(p))
}

/// Classify a script BIN into one of five picker buckets so the user can
/// translate in staged categories (names first → scenes inherit final names):
///   - `"names"` — character/item NAME tables (CVINIT/CIINIT/CNINIT/CHINIT/
///     AFINIT/ITINIT/ITMES). Translate first; `set-string` is editable here.
///   - `"sp"`    — special scenes (prefix(es) configurable via `sp_prefixes`).
///   - `"ui"`    — menu/system text (MENU/INFO/BTLMENU/CGMODE/CHMODE/SAVE/FIELD/
///     GAMESTART/CHANGECH/HOSOKU, or any `SYS*`).
///   - `"sc"`    — story scenes: an `SC`-prefixed BIN that actually has dialogue.
///   - `"other"` — everything else (data/structural BINs, misc text).
///
/// `base_upper` is the uppercased logical filename (append-prefix stripped; may
/// still carry the `.BIN` extension — prefix checks are unaffected). `sp_prefixes`
/// must already be uppercased. `has_dialogue` gates the SC bucket so empty `SC`
/// stubs fall through to other rather than masquerading as story scenes.
fn classify_bin(base_upper: &str, has_dialogue: bool, sp_prefixes: &[String]) -> &'static str {
    const UI_PREFIXES: [&str; 10] = [
        "MENU", "INFO", "BTLMENU", "CGMODE", "CHMODE", "SAVE", "FIELD", "GAMESTART",
        "CHANGECH", "HOSOKU",
    ];

    if is_name_item(base_upper) {
        return "names";
    }
    if sp_prefixes
        .iter()
        .any(|p| !p.is_empty() && base_upper.starts_with(p.as_str()))
    {
        return "sp";
    }
    if base_upper.starts_with("SYS") || UI_PREFIXES.iter().any(|p| base_upper.starts_with(p)) {
        return "ui";
    }
    if base_upper.starts_with("SC") && has_dialogue {
        return "sc";
    }
    "other"
}

/// Scan a folder (non-recursive) for AGE script `.BIN` files for the SYS picker.
///
/// Eushully ships updates as loose `$1$`/`$2$`…-prefixed BINs that override the
/// `.alf`-packed base; the highest layer is what the game actually loads, so we
/// **dedupe by logical name and keep the highest layer** (the user's "most
/// recent file"). Non-script BINs (graphics/data — no SYS signature) are skipped.
///
/// Files are bucketed into five picker categories (names/sc/sp/ui/other) by
/// `classify_bin`: name/item tables are `names`, story scenes are `SC`-prefixed
/// BINs that actually carry dialogue, special-scene prefixes come from
/// `sp_prefixes` (default `["SP"]`), menu/system files are `ui`, and everything
/// else is `other`. The translatable-string count + `likely_translated`
/// heuristics still run per file.
#[tauri::command]
pub async fn scan_bin_folder(
    folder_path: String,
    sp_prefixes: Vec<String>,
) -> Result<Vec<SysBinFile>, String> {
    use std::collections::HashMap;
    let dir = std::path::Path::new(&folder_path);
    if !dir.is_dir() {
        return Err(format!("Not a folder: {}", folder_path));
    }

    // Normalize the special-scene prefixes: trim + uppercase (classify compares
    // against uppercased names), drop blanks. Default to ["SP"] when unset.
    let sp_prefixes: Vec<String> = {
        let cleaned: Vec<String> = sp_prefixes
            .iter()
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
            .collect();
        if cleaned.is_empty() {
            vec!["SP".to_string()]
        } else {
            cleaned
        }
    };
    let entries = std::fs::read_dir(dir).map_err(|e| format!("Failed to read folder: {}", e))?;

    // Pass 1: pick the highest update layer per logical (base) name.
    let mut winners: HashMap<String, (std::path::PathBuf, String, u32)> = HashMap::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let is_bin = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("bin"))
            .unwrap_or(false);
        if !is_bin {
            continue;
        }
        let filename = match p.file_name().and_then(|f| f.to_str()) {
            Some(f) => f.to_string(),
            None => continue,
        };
        let (layer, base) = parse_update_layer(&filename);
        let key = base.to_uppercase();
        match winners.get(&key) {
            Some((_, _, l)) if *l >= layer => {}
            _ => {
                winners.insert(key, (p, filename, layer));
            }
        }
    }

    // Pass 2: validate + classify only the winners (parse for dialogue count).
    let mut out: Vec<SysBinFile> = Vec::new();
    for (_key, (p, filename, layer)) in winners {
        let data = match std::fs::read(&p) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let header = match crate::bin_format::header::Header::from_bytes(&data) {
            Ok(h) => h,
            Err(_) => continue, // not an AGE script BIN — skip
        };
        let (_, base) = parse_update_layer(&filename);
        let base_upper = base.to_uppercase();
        let name_item = is_name_item(&base_upper);
        // Parse once: count TRANSLATABLE strings AND tally Japanese vs Latin
        // letters so we can flag loose files that look already-translated.
        //
        // A "translatable" string is a non-empty, trimmed string carrying at
        // least one LETTER (a JP char in the ranges below OR an ASCII letter) on
        // one of these opcodes: show-text (0x6E), display-furigana (0x196), and
        // comment (0x1A7) for every file, PLUS set-string (0x192) ONLY in
        // name/item files (where it's the editable name table — elsewhere it's
        // the structural speaker slot). Pure symbol/numeric/structural strings
        // are skipped, so data/structural BINs (ALLMAP, ANIME, SAVE, MP*, …)
        // come out 0 and the picker can hide them.
        let (string_count, likely_translated) = match crate::bin_format::parser::parse_bin(&data) {
            Ok(parsed) => {
                let mut count = 0usize;
                let mut jp = 0usize;
                let mut latin = 0usize;
                for instr in &parsed.instructions {
                    let is_text = matches!(instr.opcode, 0x6E | 0x196 | 0x1A7)
                        || (name_item && instr.opcode == 0x192);
                    if !is_text {
                        continue;
                    }
                    for a in &instr.arguments {
                        let Some(s) = a.decoded_string.as_deref() else { continue };
                        let t = s.trim();
                        if t.is_empty() {
                            continue;
                        }
                        let mut has_letter = false;
                        for ch in t.chars() {
                            let c = ch as u32;
                            if (0x3040..=0x30FF).contains(&c)   // hiragana + katakana
                                || (0xFF66..=0xFF9D).contains(&c) // half-width katakana
                                || (0x4E00..=0x9FFF).contains(&c) // CJK kanji
                                || (0x3400..=0x4DBF).contains(&c)
                            {
                                jp += 1;
                                has_letter = true;
                            } else if ch.is_ascii_alphabetic() {
                                latin += 1;
                                has_letter = true;
                            }
                        }
                        // Only real content counts — skip pure symbol/number strings.
                        if has_letter {
                            count += 1;
                        }
                    }
                }
                // Enough letters to judge, and Latin clearly dominates → translated.
                let translated = (jp + latin) >= 20 && (latin as f64) / ((jp + latin) as f64) > 0.7;
                (count, translated)
            }
            Err(_) => (0, false),
        };
        let category = classify_bin(&base_upper, string_count > 0, &sp_prefixes);
        out.push(SysBinFile {
            path: p.to_string_lossy().to_string(),
            filename,
            base_name: base,
            layer,
            sys_version: header.sys_version,
            category: category.to_string(),
            string_count,
            likely_translated,
            size: std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
            alf_archive_index: None,
            alf_offset: None,
            alf_length: None,
        });
    }

    // Merge in base BINs packed in the `.alf` archives (the bulk of the game —
    // loose files are just updates). A loose file overrides the archived base of
    // the same name, so skip archived names already present loose. Classified by
    // filename (extracting+parsing all archived entries would be too slow).
    let loose_names: std::collections::HashSet<String> =
        out.iter().map(|f| f.base_name.to_uppercase()).collect();
    if let Ok(mdata) = std::fs::read(dir.join("SYS4INI.BIN")) {
        if let Some(idx) = crate::alf::read_sys4_master(&mdata) {
            for e in &idx.entries {
                let up = e.name.to_uppercase();
                if !up.ends_with(".BIN") || loose_names.contains(&up) {
                    continue;
                }
                // Archived entries aren't parsed (too slow to extract all), so
                // classify by filename only — treat any `SC*` as a dialogue scene.
                let category = classify_bin(&up, true, &sp_prefixes);
                out.push(SysBinFile {
                    path: String::new(),
                    filename: e.name.clone(),
                    base_name: e.name.clone(),
                    layer: 0,
                    sys_version: 4,
                    category: category.to_string(),
                    string_count: 0,
                    likely_translated: false,
                    size: e.length,
                    alf_archive_index: Some(e.archive_index),
                    alf_offset: Some(e.offset),
                    alf_length: Some(e.length),
                });
            }
        }
    }

    out.sort_by(|a, b| a.base_name.cmp(&b.base_name));
    Ok(out)
}

/// A reference to one packed entry, for extraction (sent from the frontend).
#[derive(serde::Deserialize)]
pub struct AlfRef {
    pub archive_index: u32,
    pub offset: u64,
    pub length: u64,
    pub name: String,
}

/// Extract packed `.alf` entries from a game folder into `dest_folder` as real
/// `.BIN` files (so they can be edited + autosaved + exported like any BIN).
/// Returns the written paths.
///
/// Best-effort per entry: a single bad entry (corrupt offset, decompress error)
/// is logged and skipped rather than aborting the whole workspace — so the
/// scenes that DO extract still open. Only catastrophic failures (can't read the
/// master index / can't create the dest) return `Err`. The caller compares the
/// returned count against the requested count to report partial extraction.
#[tauri::command]
pub async fn extract_alf_to_folder(
    game_dir: String,
    dest_folder: String,
    entries: Vec<AlfRef>,
) -> Result<Vec<String>, String> {
    let dir = std::path::Path::new(&game_dir);
    let mdata = std::fs::read(dir.join("SYS4INI.BIN")).map_err(|e| format!("read SYS4INI.BIN: {e}"))?;
    let idx = crate::alf::read_sys4_master(&mdata).ok_or("failed to parse SYS4INI.BIN")?;
    let dest = std::path::Path::new(&dest_folder);
    std::fs::create_dir_all(dest).map_err(|e| format!("create dest: {e}"))?;
    let mut out = Vec::with_capacity(entries.len());
    for r in &entries {
        let entry = crate::alf::AlfEntry {
            name: r.name.clone(),
            archive_index: r.archive_index,
            offset: r.offset,
            length: r.length,
        };
        let bytes = match crate::alf::extract_entry(dir, &idx, &entry) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("workspace extract: skip {} (extract failed: {e})", r.name);
                continue;
            }
        };
        let target = dest.join(&r.name);
        if let Err(e) = std::fs::write(&target, &bytes) {
            eprintln!("workspace extract: skip {} (write failed: {e})", r.name);
            continue;
        }
        out.push(target.to_string_lossy().to_string());
    }
    Ok(out)
}

/// Whether a path is a directory. Backend check (full FS access) — the frontend
/// fs plugin's `stat` is capability-scoped and throws on arbitrary dropped paths.
#[tauri::command]
pub async fn path_is_dir(path: String) -> Result<bool, String> {
    Ok(std::path::Path::new(&path).is_dir())
}

/// Probe whether a folder is writable by creating + removing a temp file.
/// More reliable than inspecting ACLs (Windows perms are layered) — an actual
/// write attempt is the ground truth. Used to detect admin-locked game folders
/// so the UI can offer a writable workspace before autosave silently fails.
#[tauri::command]
pub async fn is_folder_writable(folder_path: String) -> Result<bool, String> {
    let dir = std::path::Path::new(&folder_path);
    if !dir.is_dir() {
        return Ok(false);
    }
    let probe = dir.join(".eushully_write_probe.tmp");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

/// Copy a set of files into `dest_folder` (created if needed), preserving file
/// names. Returns the new full paths (same order). Used to set up a writable
/// workspace when the source game folder is read-only — copies the selected
/// scenes plus the speaker-name init BINs so names still resolve.
#[tauri::command]
pub async fn copy_files_to_folder(
    sources: Vec<String>,
    dest_folder: String,
) -> Result<Vec<String>, String> {
    let dest = std::path::Path::new(&dest_folder);
    std::fs::create_dir_all(dest)
        .map_err(|e| format!("Failed to create destination folder: {}", e))?;
    let mut out = Vec::with_capacity(sources.len());
    for src in &sources {
        let sp = std::path::Path::new(src);
        let name = sp
            .file_name()
            .ok_or_else(|| format!("Invalid source path: {}", src))?;
        let target = dest.join(name);
        std::fs::copy(sp, &target).map_err(|e| format!("Failed to copy {}: {}", src, e))?;
        out.push(target.to_string_lossy().to_string());
    }
    Ok(out)
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

/// Semantic box-break bridge (see `bridge::build_semantic_bridge`): built from
/// opcode semantics instead of cloning the scene's transition, so it contains no
/// scene-specific calls and is available even when the native template is
/// Degraded/Unavailable. `keep_speaker_plate` makes continuation boxes keep the
/// speaker (calls the scene's speaker box-begin routine; narration continuations
/// are fixed per box at export by `bridge::resolve_speaker_redraws`).
#[tauri::command]
pub async fn get_semantic_bridge(
    tab_id: String,
    keep_speaker_plate: bool,
    state: State<'_, AppState>,
) -> Result<Vec<crate::editor::bridge::SerializedInstruction>, String> {
    let tabs = state.tabs.lock().unwrap();
    let tab = tabs.get(&tab_id).ok_or("Tab not found")?;
    let parsed_bin = tab.parsed_bin.as_ref().ok_or("No BIN file loaded")?;
    let bridge = crate::editor::bridge::build_semantic_bridge(
        &parsed_bin.instructions,
        tab.bridge_template.as_ref(),
        keep_speaker_plate,
    );
    Ok(bridge.iter().map(crate::editor::bridge::SerializedInstruction::from).collect())
}

/// One term to send to the Claude CLI for glossary translation.
#[derive(serde::Deserialize)]
pub struct GlossaryCliEntry {
    pub term: String,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub suggestion: String,
}

/// One translated term read back from the CLI's `glossary-out.json`.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct GlossaryCliResult {
    pub term: String,
    #[serde(default)]
    pub translation: String,
    #[serde(default)]
    pub notes: String,
}

/// The Eushully-world brief + task rules the CLI Claude reads (`PROMPT.md`).
const GLOSSARY_CLI_PROMPT: &str = r#"# Eushully glossary translation

You are finalizing **glossary terms** — proper nouns, character / place names,
items, skills, and lore words — for a game by **Eushully**, a Japanese
high-fantasy SRPG / visual-novel studio (their titles include Madou Koukaku,
Sankai Ou no Yubiwa, Fuukan no Grasesta, Yamiyo no Parente, Kamidori Alchemy
Meister, Soukoku no Arterial, and others). Eushully games share a loosely
connected fantasy setting and recurring naming conventions: Western-style
fantasy names written in katakana, plus coined kanji compounds for skills and
lore. Your translations should fit that world, stay consistent with established
fan translations where they exist, and stay consistent within this game.

## Input
`glossary.json` — a list of terms. Each has:
- `term` — the original Japanese.
- `examples` — in-game lines where it appears (use them for context, gender, role).
- `suggestion` — an optional starting guess (may be empty or rough).

## Task
For each term, decide the best English rendering:
- Render **katakana** names to their intended **Western form** (the actual name
  behind the kana), not a literal kana transliteration.
- **Localize coined kanji** terms meaningfully (skills, titles, lore) — not
  word-for-word.
- Preserve **title / honorific** nuance where relevant.
- Keep **related terms consistent** (same root → same rendering).
- Prefer established **Eushully fan-translation** spellings when a term is known.

If a term is genuinely ambiguous (multiple valid readings / localizations),
present 2–3 options with a one-line rationale and ask before finalizing.
Otherwise go with your best call. You may discuss any term with the user.

## Output
When finalized, write **`glossary-out.json`** in this folder as a JSON array:

```json
[ { "term": "<original>", "translation": "<english>", "notes": "<short rationale or alternatives>" } ]
```

Write it as your LAST step — the editor watches for this file and auto-imports
it once it appears. Include every term you were given.
"#;

/// Export the selected glossary terms to a self-contained `glossary-cli/` folder
/// (glossary.json + PROMPT.md + interactive `claude` launcher + allowlist) so the
/// user can translate them on their Claude subscription (no API billing) and the
/// editor can auto-import the results. Optionally launches the launcher.
/// Returns the absolute path of the created folder.
#[tauri::command]
pub async fn export_glossary_cli(
    out_dir: String,
    entries: Vec<GlossaryCliEntry>,
    launch: bool,
) -> Result<String, String> {
    let folder = std::path::Path::new(&out_dir).join("glossary-cli");
    std::fs::create_dir_all(&folder).map_err(|e| format!("Failed to create folder: {}", e))?;
    // Clear any stale results from a previous run so the editor's watcher only
    // fires on THIS session's output.
    let _ = std::fs::remove_file(folder.join("glossary-out.json"));

    // glossary.json (input)
    let terms: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| serde_json::json!({ "term": e.term, "examples": e.examples, "suggestion": e.suggestion }))
        .collect();
    let glossary = serde_json::json!({ "terms": terms });
    std::fs::write(
        folder.join("glossary.json"),
        serde_json::to_string_pretty(&glossary).unwrap(),
    )
    .map_err(|e| format!("Failed to write glossary.json: {}", e))?;

    // PROMPT.md (Eushully brief)
    std::fs::write(folder.join("PROMPT.md"), GLOSSARY_CLI_PROMPT)
        .map_err(|e| format!("Failed to write PROMPT.md: {}", e))?;

    // .claude/settings.local.json — pre-authorize file read/write so the session
    // doesn't prompt for each glossary.json read / glossary-out.json write.
    let claude_dir = folder.join(".claude");
    std::fs::create_dir_all(&claude_dir).map_err(|e| format!("Failed to create .claude: {}", e))?;
    let settings = r#"{
  "permissions": {
    "allow": ["Read", "Write", "Edit", "Glob", "Grep"],
    "deny": []
  }
}
"#;
    std::fs::write(claude_dir.join("settings.local.json"), settings)
        .map_err(|e| format!("Failed to write settings.local.json: {}", e))?;

    // Interactive launchers. The initial message points Claude at the task; the
    // user can then discuss before it writes glossary-out.json.
    let msg = "Read PROMPT.md and glossary.json in this folder, then translate the terms per the instructions. Discuss any ambiguous term with me first; otherwise use your best judgment. Write the final results to glossary-out.json as your last step.";
    let bat = format!(
        "@echo off\r\n\
where claude >nul 2>nul || ( echo Claude Code CLI not found on PATH. Install from https://claude.com/code && pause && exit /b 1 )\r\n\
echo Translating glossary terms with Claude (PROMPT.md + glossary.json)...\r\n\
claude --permission-mode acceptEdits \"{0}\"\r\n",
        msg
    );
    std::fs::write(folder.join("translate.bat"), bat)
        .map_err(|e| format!("Failed to write translate.bat: {}", e))?;
    let sh = format!(
        "#!/usr/bin/env bash\nset -e\n\
if ! command -v claude >/dev/null 2>&1; then echo \"Claude Code CLI not found on PATH.\"; exit 1; fi\n\
echo \"Translating glossary terms with Claude (PROMPT.md + glossary.json)...\"\n\
claude --permission-mode acceptEdits \"{0}\"\n",
        msg
    );
    std::fs::write(folder.join("translate.sh"), sh)
        .map_err(|e| format!("Failed to write translate.sh: {}", e))?;

    if launch {
        // Open the launcher in a new console window (best-effort; the folder is
        // created regardless so the user can run it manually if this fails).
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", "translate.bat"])
            .current_dir(&folder)
            .spawn();
    }

    Ok(folder.to_string_lossy().to_string())
}

/// Read the CLI's `glossary-out.json` from a `glossary-cli/` folder, if present
/// and valid. Returns None when the file is missing or not yet valid JSON (a
/// half-written file), so the frontend's poll can simply retry.
#[tauri::command]
pub async fn read_glossary_cli_results(
    folder: String,
) -> Result<Option<Vec<GlossaryCliResult>>, String> {
    let path = std::path::Path::new(&folder).join("glossary-out.json");
    if !path.exists() {
        return Ok(None);
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    // Accept either a bare array or { "results": [...] }.
    if let Ok(v) = serde_json::from_str::<Vec<GlossaryCliResult>>(&raw) {
        return Ok(Some(v));
    }
    #[derive(serde::Deserialize)]
    struct Wrap {
        results: Vec<GlossaryCliResult>,
    }
    if let Ok(w) = serde_json::from_str::<Wrap>(&raw) {
        return Ok(Some(w.results));
    }
    Ok(None) // present but not valid yet — let the poll retry
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

#[cfg(test)]
mod fanpatch_features_tests {
    use super::{features_file, FanpatchFeatures};

    #[test]
    fn crisp_hd_writes_both_rendering_keys() {
        let text = features_file(FanpatchFeatures { resizable_window: true, dev_menu: false, crisp_hd: true, english_word_wrap: false }).unwrap();
        assert!(text.contains("resizable_window = 1\r\n"));
        assert!(text.contains("native_hires_window = 1\r\n"));
        assert!(text.contains("internal_resolution = 1\r\n"));
    }

    #[test]
    fn crisp_hd_needs_the_resizable_window() {
        let text = features_file(FanpatchFeatures { resizable_window: false, dev_menu: true, crisp_hd: true, english_word_wrap: false }).unwrap();
        assert!(text.contains("native_hires_window = 0\r\n"));
        assert!(text.contains("internal_resolution = 0\r\n"));
    }

    #[test]
    fn nothing_enabled_writes_no_file() {
        assert!(features_file(FanpatchFeatures { resizable_window: false, dev_menu: false, crisp_hd: true, english_word_wrap: false }).is_none());
    }

    #[test]
    fn word_wrap_alone_writes_the_file() {
        let text = features_file(FanpatchFeatures { resizable_window: false, dev_menu: false, crisp_hd: false, english_word_wrap: true }).unwrap();
        assert!(text.contains("english_word_wrap = 1\r\n"));
        assert!(text.contains("resizable_window = 0\r\n"));
    }
}
