//! Yamiyo no Parente (Unity-era Eushully) translation support.
//!
//! Pipeline: `.arc` archive → UnityFS bundle (`.txt`) → TextAsset →
//! Lua-style scene script. We crack all four layers natively so the user can
//! drag in an `.arc`, pick scene files to translate, and export a rebuilt
//! `.arc` — no external tools (UABEA / python) required.

// Format layers live in the shared `parente-arc` crate (also used by the
// Sys Decompiler's age-extractor); re-exported so `parente::archive` etc. keep working.
pub use parente_arc::{archive, bundle, serialized, typetree};
pub mod config;
pub mod font_patch;
pub mod message_layout;
pub mod patch_stamp;
pub mod script;

use crate::editor::extract::DialogueEntry;

/// Result of loading one scene: editor rows + per-row speaker labels.
#[derive(serde::Serialize)]
pub struct ParenteEntryLoad {
    pub name: String,
    pub path: String,
    pub entries: Vec<DialogueEntry>,
    /// (ordinal, speaker-name) for rows that have a detected speaker.
    pub speakers: Vec<(usize, String)>,
}

/// Translations for one scene file, addressed by archive entry index.
#[derive(serde::Deserialize)]
pub struct ParenteEntryEdit {
    pub index: usize,
    /// (ordinal, translated-text) pairs; empty translations are ignored.
    pub translations: Vec<(usize, String)>,
}

/// True for archive entries that are translatable scene scripts (Unity text
/// bundles under `src/`). Excludes images, prefabs, etc.
fn is_scene_entry(e: &archive::ArcEntry) -> bool {
    let name_l = e.name.to_ascii_lowercase();
    let path_l = e.path.to_ascii_lowercase();
    name_l.ends_with(".txt") && path_l.contains("/src")
}

/// High-signal explicit vocabulary used to flag adult scenes. These terms
/// essentially only appear in H-content, so a small count cleanly separates
/// adult scenes from the rest (Parente's H-dialogue is vocabulary-rich; in a
/// scan the lowest adult scene scored ~18 and the highest non-adult ~3, so the
/// threshold below has wide margin). Matched as substrings of dialogue text.
const EXPLICIT_TERMS: &[&str] = &[
    "ちんぽ","ちんちん","肉棒","おちんぽ","ペニス","亀頭","陰茎","男根",
    "まんこ","おまんこ","膣","陰核","クリトリス","陰唇","秘部","蜜壺","媚肉",
    "おっぱい","乳首","乳房","巨乳","乳輪",
    "射精","精液","精子","中出し","ザーメン","ぶっかけ",
    "愛液","絶頂","アクメ","潮吹",
    "挿入","ピストン","性交","セックス","陵辱","輪姦","姦",
    "勃起","オナニー","自慰","フェラ","クンニ","騎乗位","正常位","後背位",
    "舐め","しゃぶ","喘","淫","卑猥","発情","媚薬","絶倫","膣内",
];
/// Total explicit-term hits at/above which a scene is flagged adult.
const ADULT_THRESHOLD: usize = 5;

/// Total explicit-term occurrences across a scene's parsed dialogue.
fn adult_score(strings: &[script::ParenteString]) -> usize {
    let mut n = 0;
    for s in strings {
        for t in EXPLICIT_TERMS {
            if s.text.contains(t) {
                n += 1;
            }
        }
    }
    n
}

/// (translatable string count, adult-flag) for one decoded scene's text.
fn scene_stats_from_text(text: &str) -> (usize, bool) {
    let strings = script::parse_script(text);
    (strings.len(), adult_score(&strings) >= ADULT_THRESHOLD)
}

/// Count translatable strings + adult flag for one scene entry, or `None` if it
/// can't be decoded as a Lua scene (not a TextAsset, etc.).
fn scene_stats(arc_data: &[u8], e: &archive::ArcEntry) -> Option<(usize, bool)> {
    let raw = archive::entry_data(arc_data, e).ok()?;
    let b = bundle::unpack_bundle(raw).ok()?;
    let text = serialized::read_textasset_text(&b.blob).ok()?;
    Some(scene_stats_from_text(&text))
}

/// List the scene files inside an `.arc` that actually contain dialogue.
/// Decodes each candidate bundle to count translatable strings and drops any
/// with none (so the user isn't faced with empty files to open and close).
pub fn list_arc_scene_files(arc_path: &str) -> Result<Vec<ParenteGameFile>, String> {
    let data = std::fs::read(arc_path).map_err(|e| format!("Failed to read archive: {}", e))?;
    let arc = archive::unpack_arc(&data)?;
    let source = std::path::Path::new(arc_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let mut out = Vec::new();
    for e in arc.entries.iter() {
        if !is_scene_entry(e) {
            continue;
        }
        match scene_stats(&data, e) {
            Some((count, adult)) if count > 0 => out.push(ParenteGameFile {
                arc_path: arc_path.to_string(),
                source: source.clone(),
                name: e.name.clone(),
                path: e.path.clone(),
                offset: e.offset,
                size: e.size,
                string_count: count,
                adult,
            }),
            _ => {}
        }
    }
    Ok(out)
}

/// Decode one scene file's script text from the archive.
fn read_scene_text(arc_data: &[u8], arc: &archive::ArcArchive, index: usize) -> Result<(String, archive::ArcEntry, bundle::Bundle), String> {
    let entry = arc
        .entries
        .get(index)
        .ok_or("archive entry index out of range")?
        .clone();
    let raw = archive::entry_data(arc_data, &entry)?;
    let b = bundle::unpack_bundle(raw)?;
    let text = serialized::read_textasset_text(&b.blob)?;
    Ok((text, entry, b))
}

/// Read one scene file's full decoded script text (for preview / testing).
pub fn read_scene_raw_text(arc_path: &str, index: usize) -> Result<String, String> {
    let data = std::fs::read(arc_path).map_err(|e| format!("Failed to read archive: {}", e))?;
    let arc = archive::unpack_arc(&data)?;
    let (text, _entry, _b) = read_scene_text(&data, &arc, index)?;
    Ok(text)
}

/// Load one scene into editor rows.
pub fn load_entry(arc_path: &str, index: usize) -> Result<ParenteEntryLoad, String> {
    let data = std::fs::read(arc_path).map_err(|e| format!("Failed to read archive: {}", e))?;
    let arc = archive::unpack_arc(&data)?;
    let (text, entry, _b) = read_scene_text(&data, &arc, index)?;

    Ok(strings_to_load(&entry.name, &entry.path, &text))
}

/// Build a ParenteEntryLoad from a scene's decoded text (shared by single-arc
/// and game-folder loaders).
fn strings_to_load(name: &str, path: &str, text: &str) -> ParenteEntryLoad {
    let strings = script::parse_script(text);
    let mut entries = Vec::with_capacity(strings.len());
    let mut speakers = Vec::new();
    for s in &strings {
        // Prefer the block marker as the row label so the editor list lines up
        // with in-game text order; fall back to the enclosing function name.
        let label = s
            .block
            .clone()
            .unwrap_or_else(|| if s.label.is_empty() { "text".to_string() } else { s.label.clone() });
        entries.push(DialogueEntry {
            instruction_index: s.ordinal as i64,
            opcode: 0,
            label,
            original_text: s.text.clone(),
            // Existing slot-2 English (if this scene was already translated).
            translated_text: s.translated.clone(),
            notes: String::new(),
            is_context_row: false,
            string_arg_indices: Vec::new(),
            deleted: false,
            omit_export: false,
        });
        if let Some(sp) = &s.speaker {
            speakers.push((s.ordinal, sp.clone()));
        }
    }

    ParenteEntryLoad {
        name: name.to_string(),
        path: path.to_string(),
        entries,
        speakers,
    }
}

/// Rebuild a full `.arc`, applying translations to the edited scene files and
/// copying every other entry unchanged. Writes to `out_path`.
pub fn export_arc(arc_path: &str, edits: &[ParenteEntryEdit], out_path: &str) -> Result<usize, String> {
    let data = std::fs::read(arc_path).map_err(|e| format!("Failed to read archive: {}", e))?;
    let arc = archive::unpack_arc(&data)?;

    let mut edit_map = std::collections::HashMap::new();
    for e in edits {
        edit_map.insert(e.index, e);
    }

    let mut files = Vec::with_capacity(arc.entries.len());
    let mut changed = 0usize;
    for (i, entry) in arc.entries.iter().enumerate() {
        let raw = archive::entry_data(&data, entry)?;
        let file_data = if let Some(edit) = edit_map.get(&i) {
            let b = bundle::unpack_bundle(raw)?;
            let (text, enc) = serialized::read_textasset(&b.blob)?;
            let new_text = script::apply_translations(&text, &edit.translations);
            let new_blob = serialized::patch_textasset(&b.blob, &new_text, enc)?;
            changed += 1;
            bundle::pack_bundle(&b, &new_blob)
        } else {
            raw.to_vec()
        };
        files.push(archive::PackFile {
            name: entry.name.clone(),
            path: entry.path.clone(),
            data: file_data,
        });
    }

    let packed = archive::pack_arc(&arc.timestamp, &files);
    std::fs::write(out_path, &packed).map_err(|e| format!("Failed to write archive: {}", e))?;
    Ok(changed)
}

// ---------------------------------------------------------------------------
// Game-folder mode: resolve the effective (highest-priority) version of every
// dialogue scene across all archives, so the user points at the game once and
// gets one merged list, then exports a single patch.
// ---------------------------------------------------------------------------

/// One effective scene file, resolved across all of the game's archives.
#[derive(serde::Serialize, Clone)]
pub struct ParenteGameFile {
    /// Archive that holds the winning version of this file.
    pub arc_path: String,
    /// Short label of that archive (e.g. "Patch/ver103_x1.arc") for display.
    pub source: String,
    pub name: String,
    pub path: String,
    /// Entry data offset/size within `arc_path` (for direct reads).
    pub offset: u64,
    pub size: u64,
    pub string_count: usize,
    /// Heuristic flag: scene contains adult content (explicit-vocab scan).
    pub adult: bool,
}

/// Locate the `Archives` directory from whatever folder the user dropped (the
/// game root, the `Start` folder, or `Archives` itself).
fn find_archives_dir(dropped: &str) -> Option<std::path::PathBuf> {
    let base = std::path::Path::new(dropped);
    for cand in [
        base.to_path_buf(),
        base.join("Archives"),
        base.join("Start").join("Archives"),
    ] {
        if cand.join("Append").is_dir()
            || cand.join("Patch").is_dir()
            || cand.read_dir().map(|mut d| d.any(|e| {
                e.ok()
                    .map(|e| e.path().extension().map(|x| x == "arc").unwrap_or(false))
                    .unwrap_or(false)
            })).unwrap_or(false)
        {
            return Some(cand);
        }
    }
    None
}

/// Collect the game's OFFICIAL `.arc` files under the Archives dir, ordered
/// lowest → highest priority the way the engine ranks them
/// (`Filemapper.AddArchive`: newer header timestamp wins per file; ties keep the
/// first-loaded arc, approximated by base root < Append < Patch, then filename).
/// Translation patches built by this editor are excluded — they must never be
/// a source for a new export (see `patch_stamp::is_translation_patch`).
fn collect_archives(archives_dir: &std::path::Path) -> Vec<(u32, String, String)> {
    struct Found { timestamp: u64, tier: u32, path: String, label: String }
    let mut arcs: Vec<Found> = Vec::new();
    let tiers = [
        (0u32, archives_dir.to_path_buf(), ""),
        (1, archives_dir.join("Append"), "Append/"),
        (2, archives_dir.join("Patch"), "Patch/"),
    ];
    for (tier, dir, prefix) in tiers {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for path in rd.filter_map(|e| e.ok()).map(|e| e.path()) {
            if !path.extension().map(|x| x.eq_ignore_ascii_case("arc")).unwrap_or(false) {
                continue;
            }
            let (Some(path_str), Some(file_name)) = (path.to_str(), path.file_name().and_then(|s| s.to_str())) else {
                continue;
            };
            let Ok(index) = archive::read_index(path_str) else { continue };
            if patch_stamp::is_translation_patch(file_name, &index) {
                continue;
            }
            arcs.push(Found {
                timestamp: patch_stamp::archive_timestamp_value(&index.timestamp).unwrap_or(0),
                tier,
                path: path_str.to_string(),
                label: format!("{prefix}{file_name}"),
            });
        }
    }
    // Rank: (timestamp, tier, label) — later in the list wins.
    arcs.sort_by(|a, b| (a.timestamp, a.tier, &a.label).cmp(&(b.timestamp, b.tier, &b.label)));
    arcs.into_iter().map(|f| (f.tier, f.path, f.label)).collect()
}

/// Resolve the effective scene files across the whole game and count dialogue.
pub fn list_game_scene_files(dropped_dir: &str) -> Result<Vec<ParenteGameFile>, String> {
    let archives_dir = find_archives_dir(dropped_dir)
        .ok_or("Could not find an Archives folder (drop the game folder, its Start folder, or Archives).")?;
    let arcs = collect_archives(&archives_dir);
    if arcs.is_empty() {
        return Err("No .arc files found in the Archives folder.".to_string());
    }

    // Walk archives low→high priority; later writes win on (path, name).
    use std::collections::HashMap;
    struct Win { arc_path: String, source: String, path: String, offset: u64, size: u64 }
    let mut winners: HashMap<(String, String), Win> = HashMap::new();
    for (_tier, arc_path, label) in &arcs {
        let idx = match archive::read_index(arc_path) {
            Ok(a) => a,
            Err(_) => continue,
        };
        for e in &idx.entries {
            if !is_scene_entry(e) {
                continue;
            }
            winners.insert(
                (e.path.clone(), e.name.clone()),
                Win {
                    arc_path: arc_path.clone(),
                    source: label.clone(),
                    path: e.path.clone(),
                    offset: e.offset,
                    size: e.size,
                },
            );
        }
    }

    // Decode each winner to count dialogue; drop the ones with none.
    let mut out = Vec::new();
    for ((_path, name), w) in winners {
        let raw = match archive::read_entry_at(&w.arc_path, w.offset, w.size) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let (count, adult) = match bundle::unpack_bundle(&raw)
            .and_then(|b| serialized::read_textasset_text(&b.blob))
        {
            Ok(text) => scene_stats_from_text(&text),
            Err(_) => continue,
        };
        if count > 0 {
            out.push(ParenteGameFile {
                arc_path: w.arc_path,
                source: w.source,
                name,
                path: w.path,
                offset: w.offset,
                size: w.size,
                string_count: count,
                adult,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Load one effective scene (read by direct offset/size from its source arc).
pub fn load_game_entry(arc_path: &str, name: &str, path: &str, offset: u64, size: u64) -> Result<ParenteEntryLoad, String> {
    let raw = archive::read_entry_at(arc_path, offset, size)?;
    let b = bundle::unpack_bundle(&raw)?;
    let text = serialized::read_textasset_text(&b.blob)?;
    Ok(strings_to_load(name, path, &text))
}

/// Decoded script text for one scene addressed by offset/size (preview/tests).
pub fn read_scene_raw_text_at(arc_path: &str, offset: u64, size: u64) -> Result<String, String> {
    let raw = archive::read_entry_at(arc_path, offset, size)?;
    let b = bundle::unpack_bundle(&raw)?;
    serialized::read_textasset_text(&b.blob)
}

/// One file to include in an exported patch.
#[derive(serde::Deserialize)]
pub struct ParentePatchFile {
    pub arc_path: String,
    pub name: String,
    pub path: String,
    pub offset: u64,
    pub size: u64,
    pub translations: Vec<(usize, String)>,
}

/// Build a single patch archive (any `*.arc` name) containing only the translated
/// scenes, each pulled from its source archive and re-encoded with edits. Drop
/// this in `Start\Archives\Patch\` — it outranks the official patches per-file
/// (newest header timestamp wins, see `patch_stamp`) without removing anything.
pub fn export_patch(files: &[ParentePatchFile], out_path: &str) -> Result<usize, String> {
    if files.is_empty() {
        return Err("Nothing translated to export.".to_string());
    }
    // Outrank the source archives (see `patch_stamp`).
    let source_timestamp = archive::read_index(&files[0].arc_path)
        .map(|a| a.timestamp)
        .unwrap_or_default();

    let mut pack = Vec::with_capacity(files.len());
    let mut changed = 0usize;
    for f in files {
        let raw = archive::read_entry_at(&f.arc_path, f.offset, f.size)?;
        let b = bundle::unpack_bundle(&raw)?;
        let (text, enc) = serialized::read_textasset(&b.blob)?;
        let new_text = script::apply_translations(&text, &f.translations);
        let new_blob = serialized::patch_textasset(&b.blob, &new_text, enc)?;
        let data = bundle::pack_bundle(&b, &new_blob);
        pack.push(archive::PackFile { name: f.name.clone(), path: f.path.clone(), data });
        changed += 1;
    }
    patch_stamp::write_patch(out_path, &[&source_timestamp], pack)?;
    Ok(changed)
}

/// Build a SLIM destructive patch (English replaces Japanese in slot 1): every
/// edited file, taken from `primary_arc` (the current official patch, e.g.
/// ver103) when it has it, else from its own source archive. Untouched files
/// are NOT copied — the export-time header timestamp (see `patch_stamp`) makes
/// this arc win per file over the official archives, which stay installed and
/// keep serving everything else. Drop it in `Patch` under any `*.arc` name.
pub fn export_full(primary_arc: &str, files: &[ParentePatchFile], out_path: &str) -> Result<usize, String> {
    let data = std::fs::read(primary_arc).map_err(|e| format!("Failed to read {}: {}", primary_arc, e))?;
    let arc = archive::unpack_arc(&data)?;

    use std::collections::{HashMap, HashSet};
    let mut by_key: HashMap<(String, String), &ParentePatchFile> = HashMap::new();
    for f in files {
        by_key.insert((f.path.clone(), f.name.clone()), f);
    }

    let mut out_files = Vec::with_capacity(files.len());
    let mut used: HashSet<(String, String)> = HashSet::new();
    let mut changed = 0usize;
    for entry in &arc.entries {
        let raw = archive::entry_data(&data, entry)?;
        let key = (entry.path.clone(), entry.name.clone());
        let Some(f) = by_key.get(&key) else {
            continue; // untouched: the official archive keeps serving it
        };
        let b = bundle::unpack_bundle(raw)?;
        let (text, enc) = serialized::read_textasset(&b.blob)?;
        let new_text = script::apply_translations(&text, &f.translations);
        let new_blob = serialized::patch_textasset(&b.blob, &new_text, enc)?;
        used.insert(key);
        changed += 1;
        let fdata = bundle::pack_bundle(&b, &new_blob);
        out_files.push(archive::PackFile {
            name: entry.name.clone(),
            path: entry.path.clone(),
            data: fdata,
        });
    }

    // Edited files not present in the primary arc (e.g. base-only scenes) —
    // pull from their own source and add them so they override too.
    for f in files {
        let key = (f.path.clone(), f.name.clone());
        if used.contains(&key) {
            continue;
        }
        let raw = archive::read_entry_at(&f.arc_path, f.offset, f.size)?;
        let b = bundle::unpack_bundle(&raw)?;
        let (text, enc) = serialized::read_textasset(&b.blob)?;
        let new_text = script::apply_translations(&text, &f.translations);
        let new_blob = serialized::patch_textasset(&b.blob, &new_text, enc)?;
        out_files.push(archive::PackFile {
            name: f.name.clone(),
            path: f.path.clone(),
            data: bundle::pack_bundle(&b, &new_blob),
        });
        changed += 1;
    }

    patch_stamp::write_patch(out_path, &[&arc.timestamp], out_files)?;
    Ok(changed)
}

/// Suffix of the one-time pristine backups taken before the runtime font fix.
const FONT_FIX_BACKUP_SUFFIX: &str = ".pre-fontfix.bak";

/// Locate `START_Data` from a dropped game folder, its `Start` folder, or itself.
fn find_start_data(dir: &str) -> Option<std::path::PathBuf> {
    let d = std::path::Path::new(dir);
    [d.join("START_Data"), d.join("Start").join("START_Data"), d.to_path_buf()]
        .into_iter()
        .find(|p| p.join(font_patch::RUNTIME_FONT_ASSETS).is_file())
}

fn backup_path(p: &std::path::Path) -> std::path::PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(FONT_FIX_BACKUP_SUFFIX);
    std::path::PathBuf::from(s)
}

/// Apply the monospace dialogue font to the game's RUNTIME fonts
/// (`START_Data/sharedassets0.assets` + `.resS`, loaded by FontManager — the
/// arc prefab copies are unused). Always patches from the pristine one-time
/// backup, so re-applying never stacks. Returns glyphs replaced.
pub fn apply_runtime_font_fix(game_dir: &str) -> Result<usize, String> {
    let start_data = find_start_data(game_dir).ok_or("Could not find START_Data (drop the game folder or its Start folder).")?;
    let archives_dir = find_archives_dir(game_dir).ok_or("Could not find the game's Archives folder.")?;
    let (assets_path, stream_path) = font_patch::runtime_font_paths(&start_data);
    for p in [&assets_path, &stream_path] {
        let b = backup_path(p);
        if !b.exists() {
            std::fs::copy(p, &b).map_err(|e| format!("Backup failed ({}): {e}", b.display()))?;
        }
    }
    let (da, doff, dsize) = find_file_across(&archives_dir, font_patch::DONOR_BUNDLE).ok_or("Donor font bundle (ui_battlemap) not found in the game.")?;
    let donor = archive::read_entry_at(&da, doff, dsize)?;
    let mut assets = std::fs::read(backup_path(&assets_path)).map_err(|e| format!("Read failed: {e}"))?;
    let mut stream = std::fs::read(backup_path(&stream_path)).map_err(|e| format!("Read failed: {e}"))?;
    let glyphs = font_patch::patch_runtime_fonts(&mut assets, &mut stream, &donor)?;
    if glyphs == 0 {
        return Err("No runtime font was patched (unexpected game data) — files left untouched.".into());
    }
    std::fs::write(&stream_path, &stream).map_err(|e| format!("Write failed ({}): {e}", stream_path.display()))?;
    std::fs::write(&assets_path, &assets).map_err(|e| format!("Write failed ({}): {e}", assets_path.display()))?;
    Ok(glyphs)
}

/// Undo `apply_runtime_font_fix` by restoring the pristine backups.
pub fn revert_runtime_font_fix(game_dir: &str) -> Result<(), String> {
    let start_data = find_start_data(game_dir).ok_or("Could not find START_Data.")?;
    let (assets_path, stream_path) = font_patch::runtime_font_paths(&start_data);
    for p in [&assets_path, &stream_path] {
        let b = backup_path(p);
        if !b.exists() {
            return Err(format!("No backup found ({}) — font fix was never applied here.", b.display()));
        }
        std::fs::copy(&b, p).map_err(|e| format!("Restore failed ({}): {e}", p.display()))?;
    }
    Ok(())
}

/// Prefab bundles an earlier build font-patched into exports. The game draws
/// dialogue with START_Data's runtime fonts, so these copies did nothing but
/// add ~59 MB; upgrades drop them (the base archives still provide originals).
const LEGACY_FONT_BUNDLES: &[&str] = &["ui_message", "ui_history"];

/// Prefab that older exports added with the (crashing) language buttons enabled.
const LANG_BUTTON_PREFAB: &str = "ui_config_basic";

/// Where the official game serves each file: (path, name) → (arc, offset, size)
/// of the highest-priority OFFICIAL copy, ignoring `exclude` (the patch being
/// processed, which may predate the marker entry).
fn official_file_sources(
    archives_dir: &std::path::Path,
    exclude: &str,
) -> std::collections::HashMap<(String, String), (String, u64, u64)> {
    let same_file = |a: &str| a.replace('\\', "/").eq_ignore_ascii_case(&exclude.replace('\\', "/"));
    let mut sources = std::collections::HashMap::new();
    for (_tier, arc_path, _label) in collect_archives(archives_dir) {
        if same_file(&arc_path) {
            continue;
        }
        let Ok(index) = archive::read_index(&arc_path) else { continue };
        for e in index.entries {
            sources.insert((e.path, e.name), (arc_path.clone(), e.offset, e.size));
        }
    }
    sources
}

/// True if `raw` is byte-identical to the official copy the game would load
/// anyway — such entries are dead weight in a patch.
fn matches_official(
    raw: &[u8],
    official: Option<&(String, u64, u64)>,
) -> bool {
    match official {
        Some((arc_path, offset, size)) if *size == raw.len() as u64 => {
            archive::read_entry_at(arc_path, *offset, *size).map(|o| o == raw).unwrap_or(false)
        }
        _ => false,
    }
}

/// Re-apply the engine-level fixes to an ALREADY-EXPORTED patch arc without
/// re-exporting any scenes: fill LTSINIT's English placeholders, drop bundles
/// older exports added that are crash-prone (the language-button prefab) or
/// dead weight (prefab font copies — the dialogue renders with the RUNTIME
/// fonts in START_Data, see `apply_runtime_font_fix`), SLIM it by dropping
/// entries byte-identical to the installed official copy (old full-size
/// exports carried all of ver103), then re-stamp + mark it (`patch_stamp`).
/// Scene translations are kept byte-for-byte. Returns how many entries were dropped.
pub fn upgrade_existing_patch(patch_arc: &str, out_path: &str) -> Result<usize, String> {
    let data = std::fs::read(patch_arc).map_err(|e| format!("Failed to read {patch_arc}: {e}"))?;
    let arc = archive::unpack_arc(&data)?;
    let archives_dir = archives_dir_for(patch_arc);
    let official = official_file_sources(&archives_dir, patch_arc);
    let mut out_files = Vec::with_capacity(arc.entries.len());
    let mut have_ltsinit = false;
    let mut dropped = 0;
    for entry in &arc.entries {
        let is_marker = entry.path == patch_stamp::MARKER_PATH && entry.name == patch_stamp::MARKER_NAME;
        if is_marker {
            continue; // re-written by `write_patch`
        }
        if entry.name == LANG_BUTTON_PREFAB || LEGACY_FONT_BUNDLES.contains(&entry.name.as_str()) {
            dropped += 1;
            continue;
        }
        let raw = archive::entry_data(&data, entry)?;
        if entry.name != LTSINIT_FILE
            && matches_official(raw, official.get(&(entry.path.clone(), entry.name.clone())))
        {
            dropped += 1;
            continue;
        }
        let data = if entry.name == LTSINIT_FILE {
            have_ltsinit = true;
            rebundle(raw, fill_ltsinit_english)?
        } else {
            raw.to_vec()
        };
        out_files.push(archive::PackFile { name: entry.name.clone(), path: entry.path.clone(), data });
    }
    if !have_ltsinit {
        if let Some((a, o, s)) = find_file_across(&archives_dir, LTSINIT_FILE) {
            let raw = archive::read_entry_at(&a, o, s)?;
            out_files.push(archive::PackFile { name: LTSINIT_FILE.into(), path: "/src/sys".into(), data: rebundle(&raw, fill_ltsinit_english)? });
        }
    }
    patch_stamp::write_patch(out_path, &[&arc.timestamp], out_files)?;
    Ok(dropped)
}

/// The game's `Archives` dir for a primary arc (which may sit in `Patch`/`Append`).
fn archives_dir_for(primary_arc: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(primary_arc);
    let parent = p.parent().unwrap_or(std::path::Path::new("."));
    match parent.file_name().and_then(|s| s.to_str()) {
        Some("Patch") | Some("Append") => parent.parent().unwrap_or(parent).to_path_buf(),
        _ => parent.to_path_buf(),
    }
}

// --- English (non-destructive, slot-2) patch -------------------------------

/// Patch `system.txt`'s `text()` to render the English slot (arg 2) when the
/// player has selected English (`LangNo==2`, set by the in-config language
/// button we enable) and the slot is filled; otherwise the Japanese (slot 1).
fn english_patch_system(t: &str) -> String {
    let t = t.replacen("function text(str)", "function text(str,k2,k3,k4)", 1);
    t.replacen(
        "LuaAdvMsg=str",
        "if(LangNo==2 and k2~=nil and k2~=\"\") then str=k2 end LuaAdvMsg=str",
        1,
    )
}

/// Patch `LangArray()` to return English (key2) when present under English mode,
/// else Japanese (key1) — the JP fallback so partial translations never blank.
/// Also patch `DrawChapter`'s per-language image lookups to fall back to the JP
/// image so English mode doesn't hang on the chapter-title sequence (see below).
fn english_patch_systemex(t: &str) -> String {
    let t = t.replacen(
        "if(LangNo==1) then return key1 end",
        "if(LangNo==2 and key2~=nil and key2~=\"\") then return key2 end return key1",
        1,
    );
    let t = t.replacen("if(LangNo==2) then return key2 end", "", 1);
    let t = t.replacen("if(LangNo==3) then return key3 end", "", 1);
    let t = t.replacen("if(LangNo==4) then return key4 end", "", 1);
    // Chapter-title images (`DrawChapter`) are per-language PNGs, but ONLY slot 1
    // (Japanese) is filled — slots 2-4 are "". Under English (LangNo==2) the raw
    // `ckeyX[key][LangNo]` yields "", and `set(C,"")` stalls the title sequence
    // (never reaching EndProcessSkip / isMenu=0) → the game hangs with input
    // frozen. Fall back to the JP image (index 1) when the language slot is
    // empty/nil. Idempotent: a re-export won't re-match the already-rewritten line.
    let t = patch_chapter_lang_fallback(&t, "C2", "ckeyA");
    let t = patch_chapter_lang_fallback(&t, "C3", "ckeyB");
    patch_chapter_lang_fallback(&t, "C4", "ckeyC")
}

/// Rewrite `set(<cvar>,<arr>[key][LangNo])` so an empty/nil per-language slot
/// falls back to the Japanese image (`<arr>[key][1]`).
fn patch_chapter_lang_fallback(t: &str, cvar: &str, arr: &str) -> String {
    let from = format!("set({cvar},{arr}[key][LangNo])");
    let to = format!(
        "set({cvar},({arr}[key][LangNo]~=nil and {arr}[key][LangNo]~=\"\" and {arr}[key][LangNo]) or {arr}[key][1])"
    );
    t.replacen(&from, &to, 1)
}

/// Placeholder the developers left in unfilled `LTSINIT` language slots.
const LTSINIT_PLACEHOLDER: &str = "Non";
/// UI string table (`LTSINIT[key][LangNo]`, read by `Framework.GetLangTable`).
const LTSINIT_FILE: &str = "ltsinit.txt";

/// Byte ranges of the quoted string contents in a Lua table row body
/// (`{"a","b",...}`), honouring `\"` escapes. Pure helper for LTSINIT rows.
fn quoted_ranges(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                j += if bytes[j] == b'\\' { 2 } else { 1 };
            }
            if j >= bytes.len() {
                break;
            }
            ranges.push((start, j));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    ranges
}

/// Fill the English slot of every `LTSINIT["key"]={jp,en,cn,tw}` UI-string row
/// whose English is the dev placeholder `"Non"` or empty, with slot 1's text.
/// All 45 such rows in the shipped table already carry English (or a neutral
/// placeholder like `－－`) in slot 1 — the developers wrote English into the
/// wrong column — so English mode stops rendering literal "Non" in popups and
/// the battle UI. Pure; idempotent; lines other than LTSINIT rows untouched.
fn fill_ltsinit_english(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let Some(body_start) = trimmed.starts_with("LTSINIT[").then(|| line.find("={")).flatten() else {
            out.push_str(line);
            continue;
        };
        let ranges = quoted_ranges(&line[body_start..]);
        let (Some(&(s1, e1)), Some(&(s2, e2))) = (ranges.first(), ranges.get(1)) else {
            out.push_str(line);
            continue;
        };
        let body = &line[body_start..];
        let slot1 = &body[s1..e1];
        let slot2 = &body[s2..e2];
        if slot1.is_empty() || !(slot2 == LTSINIT_PLACEHOLDER || slot2.is_empty()) {
            out.push_str(line);
            continue;
        }
        out.push_str(&line[..body_start + s2]);
        out.push_str(slot1);
        out.push_str(&line[body_start + e2..]);
    }
    out
}

/// Re-bundle a file after transforming its decoded script text.
fn rebundle(raw: &[u8], transform: impl FnOnce(&str) -> String) -> Result<Vec<u8>, String> {
    let b = bundle::unpack_bundle(raw)?;
    let (text, enc) = serialized::read_textasset(&b.blob)?;
    let new_blob = serialized::patch_textasset(&b.blob, &transform(&text), enc)?;
    Ok(bundle::pack_bundle(&b, &new_blob))
}

/// Locate the effective (highest-priority) version of a file across the game's
/// archives, returning (arc_path, offset, size).
fn find_file_across(archives_dir: &std::path::Path, name: &str) -> Option<(String, u64, u64)> {
    let mut found: Option<(String, u64, u64)> = None;
    for (_tier, arc_path, _label) in collect_archives(archives_dir) {
        if let Ok(a) = archive::read_index(&arc_path) {
            if let Some(e) = a.entries.iter().find(|e| e.name == name) {
                found = Some((arc_path.clone(), e.offset, e.size));
            }
        }
    }
    found
}

/// Build the SLIM NON-DESTRUCTIVE English patch: English written into slot 2 of
/// every edited scene (Japanese preserved in slot 1), the `system.txt` /
/// `systemex.txt` functions patched so the engine renders slot 2, and LTSINIT's
/// English placeholders filled. Each file comes from `primary_arc` (the current
/// official patch) when it has it, else from elsewhere in the game. Untouched
/// files are NOT copied — the export-time header timestamp (see `patch_stamp`)
/// makes this arc win per file while the official archives serve the rest.
pub fn export_english(primary_arc: &str, files: &[ParentePatchFile], out_path: &str) -> Result<usize, String> {
    let data = std::fs::read(primary_arc).map_err(|e| format!("Failed to read {}: {}", primary_arc, e))?;
    let arc = archive::unpack_arc(&data)?;

    use std::collections::{HashMap, HashSet};
    let mut by_key: HashMap<(String, String), &ParentePatchFile> = HashMap::new();
    for f in files {
        by_key.insert((f.path.clone(), f.name.clone()), f);
    }

    let mut out_files = Vec::with_capacity(arc.entries.len());
    let mut used: HashSet<(String, String)> = HashSet::new();
    let mut have_system = false;
    let mut have_systemex = false;
    let mut have_ltsinit = false;
    let mut changed = 0usize;
    for entry in &arc.entries {
        let raw = archive::entry_data(&data, entry)?;
        let fdata = if entry.name == LTSINIT_FILE {
            have_ltsinit = true;
            rebundle(raw, fill_ltsinit_english)?
        } else if entry.name == "system.txt" {
            have_system = true;
            rebundle(raw, english_patch_system)?
        } else if entry.name == "systemex.txt" {
            have_systemex = true;
            rebundle(raw, english_patch_systemex)?
        } else if let Some(f) = by_key.get(&(entry.path.clone(), entry.name.clone())) {
            used.insert((entry.path.clone(), entry.name.clone()));
            changed += 1;
            let trans = f.translations.clone();
            rebundle(raw, move |t| script::apply_english(t, &trans))?
        } else {
            continue; // untouched: the official archive keeps serving it
        };
        out_files.push(archive::PackFile { name: entry.name.clone(), path: entry.path.clone(), data: fdata });
    }

    // Edited scenes not in the primary archive (base-only) → pull + add.
    for f in files {
        if used.contains(&(f.path.clone(), f.name.clone())) {
            continue;
        }
        let raw = archive::read_entry_at(&f.arc_path, f.offset, f.size)?;
        let trans = f.translations.clone();
        let data = rebundle(&raw, move |t| script::apply_english(t, &trans))?;
        out_files.push(archive::PackFile { name: f.name.clone(), path: f.path.clone(), data });
        changed += 1;
    }

    // Ensure the two patched engine functions are present (add from elsewhere).
    let archives_dir = archives_dir_for(primary_arc);
    if !have_system {
        let (a, o, s) = find_file_across(&archives_dir, "system.txt").ok_or("system.txt not found in game")?;
        let raw = archive::read_entry_at(&a, o, s)?;
        out_files.push(archive::PackFile { name: "system.txt".into(), path: "/src/sys".into(), data: rebundle(&raw, english_patch_system)? });
    }
    if !have_systemex {
        let (a, o, s) = find_file_across(&archives_dir, "systemex.txt").ok_or("systemex.txt not found in game")?;
        let raw = archive::read_entry_at(&a, o, s)?;
        out_files.push(archive::PackFile { name: "systemex.txt".into(), path: "/src/sys".into(), data: rebundle(&raw, english_patch_systemex)? });
    }
    if !have_ltsinit {
        // Non-fatal: without it English mode just shows the dev "Non" placeholders.
        if let Some((a, o, s)) = find_file_across(&archives_dir, LTSINIT_FILE) {
            let raw = archive::read_entry_at(&a, o, s)?;
            out_files.push(archive::PackFile { name: LTSINIT_FILE.into(), path: "/src/sys".into(), data: rebundle(&raw, fill_ltsinit_english)? });
        }
    }
    // The in-config language buttons (`bt_langN` in `ui_config_basic`) are
    // deliberately LEFT HIDDEN: clicking them crashes the game
    // (NullReferenceException in ui_allmap.UpdateString via
    // ui_config_basic.OnClickBut → input frozen). Language is set via
    // CONFIG.DAT instead (`config::set_language_at`, "Set game language").

    patch_stamp::write_patch(out_path, &[&arc.timestamp], out_files)?;
    Ok(changed)
}

/// Default patch file name. The loader takes every `*.arc` recursively, so the
/// name is free — put the version in it so players can tell which one they have.
pub const DEFAULT_PATCH_FILE_NAME: &str = "moist English TL v1.0.arc";

/// If the dropped folder is a game install, return the path where a patch
/// should be written (`…\Start\Archives\Patch\<DEFAULT_PATCH_FILE_NAME>`).
pub fn suggested_patch_path(dropped_dir: &str) -> Option<String> {
    let archives = find_archives_dir(dropped_dir)?;
    let patch = archives.join("Patch");
    if patch.is_dir() {
        patch.join(DEFAULT_PATCH_FILE_NAME).to_str().map(|s| s.to_string())
    } else {
        archives.join(DEFAULT_PATCH_FILE_NAME).to_str().map(|s| s.to_string())
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn parente_list_arc(path: String) -> Result<Vec<ParenteGameFile>, String> {
    list_arc_scene_files(&path)
}

#[tauri::command]
pub async fn parente_load_entry(arc_path: String, index: usize) -> Result<ParenteEntryLoad, String> {
    load_entry(&arc_path, index)
}

#[tauri::command]
pub async fn parente_export_arc(
    arc_path: String,
    edits: Vec<ParenteEntryEdit>,
    out_path: String,
) -> Result<usize, String> {
    export_arc(&arc_path, &edits, &out_path)
}

/// Apply the monospace dialogue font to the game's runtime fonts (START_Data).
#[tauri::command]
pub async fn parente_apply_font_fix(game_dir: String) -> Result<usize, String> {
    apply_runtime_font_fix(&game_dir)
}

/// Restore the game's original runtime fonts from the pre-fix backups.
#[tauri::command]
pub async fn parente_revert_font_fix(game_dir: String) -> Result<(), String> {
    revert_runtime_font_fix(&game_dir)
}

/// Upgrade an exported patch IN PLACE (see `upgrade_existing_patch`): writes a
/// temp file, keeps the first original as `<arc>.pre-upgrade.bak`, then swaps.
#[tauri::command]
pub async fn parente_upgrade_patch(patch_arc: String) -> Result<usize, String> {
    let tmp = format!("{patch_arc}.upgrading");
    let dropped = upgrade_existing_patch(&patch_arc, &tmp)?;
    let backup = format!("{patch_arc}.pre-upgrade.bak");
    if !std::path::Path::new(&backup).exists() {
        std::fs::copy(&patch_arc, &backup).map_err(|e| format!("Backup failed ({backup}): {e}"))?;
    }
    std::fs::rename(&tmp, &patch_arc).map_err(|e| format!("Replace failed ({patch_arc}): {e}"))?;
    Ok(dropped)
}

/// Saved game language from CONFIG.DAT (1 = Japanese, 2 = English). `None` if
/// the file has no LangNo yet. Errors if the game was never launched.
#[tauri::command]
pub async fn parente_get_game_language() -> Result<Option<u8>, String> {
    config::get_language_at(&config::default_config_path()?)
}

/// Write the game language straight into CONFIG.DAT (bypasses the crashing
/// in-game language button). Returns the one-time backup path.
#[tauri::command]
pub async fn parente_set_game_language(lang: u8) -> Result<String, String> {
    let backup = config::set_language_at(&config::default_config_path()?, lang)?;
    Ok(backup.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn parente_list_game(dir: String) -> Result<Vec<ParenteGameFile>, String> {
    list_game_scene_files(&dir)
}

#[tauri::command]
pub async fn parente_load_game_entry(
    arc_path: String,
    name: String,
    path: String,
    offset: u64,
    size: u64,
) -> Result<ParenteEntryLoad, String> {
    load_game_entry(&arc_path, &name, &path, offset, size)
}

#[tauri::command]
pub async fn parente_export_patch(files: Vec<ParentePatchFile>, out_path: String) -> Result<usize, String> {
    export_patch(&files, &out_path)
}

#[tauri::command]
pub async fn parente_export_full(
    primary_arc: String,
    files: Vec<ParentePatchFile>,
    out_path: String,
) -> Result<usize, String> {
    export_full(&primary_arc, &files, &out_path)
}

#[tauri::command]
pub async fn parente_export_english(
    primary_arc: String,
    files: Vec<ParentePatchFile>,
    out_path: String,
) -> Result<usize, String> {
    export_english(&primary_arc, &files, &out_path)
}

#[tauri::command]
pub async fn parente_suggested_patch_path(dir: String) -> Result<Option<String>, String> {
    Ok(suggested_patch_path(&dir))
}

/// Legacy directory where Parente per-scene project files lived (under the app
/// config dir, next to settings.json). Still used as the read-only-folder save
/// fallback and to migrate projects saved by older builds. New saves go to a
/// visible "Parente Projects" folder next to the game's `.arc` instead.
fn projects_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    let d = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("config dir: {}", e))?
        .join("parente-projects");
    std::fs::create_dir_all(&d).map_err(|e| format!("create projects dir: {}", e))?;
    Ok(d)
}

/// Sanitize a scene name into a safe JSON filename component: keep
/// `[A-Za-z0-9._-]`, map everything else to `_`. Same character class the
/// frontend's legacy project key used, so migrated filenames line up.
fn sanitize_project_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect()
}

/// Persist one Parente scene's translation project. Preferred target is a
/// visible `<base_dir>/Parente Projects/<clean-name>.json` next to the game's
/// `.arc`; if that folder can't be created/written (e.g. an admin-locked game
/// install) we fall back to the legacy app-data dir. A save must NEVER error out
/// — the user's in-session translations depend on it — so this always returns Ok.
#[tauri::command]
pub async fn parente_save_project(
    app: tauri::AppHandle,
    base_dir: String,
    name: String,
    content: String,
) -> Result<(), String> {
    let clean = sanitize_project_name(&name);
    // Preferred: the visible folder beside the .arc.
    if !base_dir.is_empty() {
        let dir = std::path::Path::new(&base_dir).join("Parente Projects");
        if std::fs::create_dir_all(&dir).is_ok() {
            let target = dir.join(format!("{}.json", clean));
            if std::fs::write(&target, &content).is_ok() {
                return Ok(());
            }
        }
    }
    // Fallback: the legacy app-data dir (read-only game folder, etc.). Reloadable
    // via `parente_load_project`'s sanitized-name check below. Swallow errors so a
    // failed save never surfaces as lost work in the UI.
    if let Ok(dir) = projects_dir(&app) {
        let target = dir.join(format!("{}.json", clean));
        let _ = std::fs::write(&target, &content);
    }
    Ok(())
}

/// Load one Parente scene's saved project, or `None`. Checks the new visible
/// folder first, then the legacy app-data dir — by `legacy_key` (migrates
/// projects saved by older builds keyed on `path/name`) and by the sanitized
/// `name` (reloads anything the read-only-folder save fallback wrote).
#[tauri::command]
pub async fn parente_load_project(
    app: tauri::AppHandle,
    base_dir: String,
    name: String,
    legacy_key: String,
) -> Result<Option<String>, String> {
    let clean = sanitize_project_name(&name);
    // 1) New visible folder next to the .arc.
    if !base_dir.is_empty() {
        let p = std::path::Path::new(&base_dir)
            .join("Parente Projects")
            .join(format!("{}.json", clean));
        if p.exists() {
            return Ok(Some(
                std::fs::read_to_string(&p).map_err(|e| format!("load project: {}", e))?,
            ));
        }
    }
    // 2) Legacy app-data dir: legacy_key (older-build filenames) then sanitized
    //    name (read-only-folder save fallback). First hit wins; re-saved to the
    //    new folder on the next save.
    if let Ok(dir) = projects_dir(&app) {
        for fname in [format!("{}.json", legacy_key), format!("{}.json", clean)] {
            let p = dir.join(&fname);
            if p.exists() {
                return Ok(Some(
                    std::fs::read_to_string(&p).map_err(|e| format!("load project: {}", e))?,
                ));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ltsinit_placeholder_slots_take_slot1() {
        let src = concat!(
            "LTSINIT ={}\n",
            "LTSINIT[\"bm4\"]={\"Terrain\",\"Non\",\"Non\",\"Non\",}\n",
            "LTSINIT[\"1\"]={\"はい\", \"Yes\", \"是\", \"是\"}\n",
            "LTSINIT[\"ope2\"]={\"\",\"\",\"\",\"\",}\n",
            "LTSINIT[\"x\"]={\"A \\\"q\\\"\",\"\",\"\",\"\",}\n",
        );
        let out = super::fill_ltsinit_english(src);
        assert!(out.contains("LTSINIT[\"bm4\"]={\"Terrain\",\"Terrain\",\"Non\",\"Non\",}"), "{out}");
        assert!(out.contains("{\"はい\", \"Yes\","), "real English untouched");
        assert!(out.contains("LTSINIT[\"ope2\"]={\"\",\"\","), "empty slot 1 left alone");
        assert!(out.contains("{\"A \\\"q\\\"\",\"A \\\"q\\\"\","), "escaped quotes copied verbatim: {out}");
        assert_eq!(super::fill_ltsinit_english(&out), out, "idempotent");
    }

    #[test]
    fn english_patch_systemex_falls_back_for_chapter_images() {
        // Under English (LangNo==2) the per-language chapter images are empty, so
        // the raw `ckeyX[key][LangNo]` must fall back to the JP image (index 1) or
        // the chapter-title sequence stalls and freezes input.
        let out = super::english_patch_systemex(
            "set(C2,ckeyA[key][LangNo])\nset(C3,ckeyB[key][LangNo])\nset(C4,ckeyC[key][LangNo])",
        );
        assert!(!out.contains("set(C2,ckeyA[key][LangNo])"), "C2 not patched: {out}");
        assert!(out.contains("or ckeyA[key][1])"), "no JP fallback for ckeyA: {out}");
        assert!(out.contains("or ckeyB[key][1])"), "no JP fallback for ckeyB: {out}");
        assert!(out.contains("or ckeyC[key][1])"), "no JP fallback for ckeyC: {out}");
    }

    #[test]
    fn apply_english_escapes_lua_string_specials() {
        // Regression: an embedded newline/quote/backslash in a translation must be
        // escaped, or Lua aborts in-game with `unfinished string` (sc0001 SyntaxError).
        let out = script::apply_english(
            "text(\"x\",\"\",\"\",\"\")",
            &[(0, "a\nb \"q\" c\\d".to_string())],
        );
        assert!(out.contains("a\\nb"), "newline not escaped: {out}");
        assert!(!out.contains("a\nb"), "raw newline leaked into Lua string: {out}");
        assert!(out.contains("\\\"q\\\""), "quote not escaped: {out}");
        assert!(out.contains("c\\\\d"), "backslash not escaped: {out}");
    }

    #[test]
    fn apply_english_converts_double_hyphen_to_em_dash() {
        // The game's Lua treats `--` as a comment EVEN INSIDE a string, truncating
        // the closing quote → `unfinished string` (sc0001 "...father too--was I..."),
        // so `--` must never survive into the emitted Lua string.
        let out = script::apply_english("text(\"x\",\"\",\"\",\"\")", &[(0, "too--was".to_string())]);
        assert!(!out.contains("--"), "double-hyphen survived into Lua: {out}");
        assert!(out.contains("too—was"), "not converted to em-dash: {out}");
    }

    #[test]
    fn script_round_trip_identity() {
        let src = "text(\"こんにちは\",\"\",\"\",\"\")\nplayVoice(\"a.ogg\")\n";
        let strings = script::parse_script(src);
        assert_eq!(strings.len(), 1);
        assert_eq!(strings[0].text, "こんにちは");
        // No edits → identical output.
        let out = script::apply_translations(src, &[]);
        assert_eq!(out, src);
    }

    #[test]
    fn script_apply_edit() {
        let src = "text(\"こんにちは\",\"\")\n";
        let out = script::apply_translations(src, &[(0, "Hello".to_string())]);
        assert_eq!(out, "text(\"Hello\",\"\")\n");
    }

    #[test]
    fn script_escapes_quotes() {
        let src = "text(\"あ\")\n";
        let out = script::apply_translations(src, &[(0, "He said \"hi\"".to_string())]);
        assert_eq!(out, "text(\"He said \\\"hi\\\"\")\n");
    }

    #[test]
    fn script_speaker_from_comment() {
        let src = "-- イリーナ --\ntext(\"ようこそ\")\n";
        let strings = script::parse_script(src);
        assert_eq!(strings[0].speaker.as_deref(), Some("イリーナ"));
    }
}
