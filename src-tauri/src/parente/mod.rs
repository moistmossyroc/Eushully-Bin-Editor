//! Yamiyo no Parente (Unity-era Eushully) translation support.
//!
//! Pipeline: `.arc` archive → UnityFS bundle (`.txt`) → TextAsset →
//! Lua-style scene script. We crack all four layers natively so the user can
//! drag in an `.arc`, pick scene files to translate, and export a rebuilt
//! `.arc` — no external tools (UABEA / python) required.

pub mod archive;
pub mod bundle;
pub mod script;
pub mod serialized;

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

/// Collect every `.arc` under the Archives dir with a load-order priority
/// (base root < Append < Patch; ties broken by filename ascending).
fn collect_archives(archives_dir: &std::path::Path) -> Vec<(u32, String, String)> {
    let mut arcs: Vec<(u32, String, String)> = Vec::new(); // (tier, path, label)
    let tiers = [
        (0u32, archives_dir.to_path_buf(), ""),
        (1, archives_dir.join("Append"), "Append/"),
        (2, archives_dir.join("Patch"), "Patch/"),
    ];
    for (tier, dir, prefix) in tiers {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut found: Vec<String> = rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x == "arc").unwrap_or(false))
                .filter_map(|p| p.to_str().map(|s| s.to_string()))
                .collect();
            found.sort();
            for p in found {
                let fname = std::path::Path::new(&p)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                arcs.push((tier, p, format!("{}{}", prefix, fname)));
            }
        }
    }
    // Stable order: tier, then filename — assign final rank by this order.
    arcs.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));
    arcs
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

/// Build a single patch archive (`ver999.arc`) containing only the translated
/// scenes, each pulled from its source archive and re-encoded with edits. Drop
/// this in `Start\Archives\Patch\` — it outranks the official patches per-file
/// (highest archive wins) without removing anything.
pub fn export_patch(files: &[ParentePatchFile], out_path: &str) -> Result<usize, String> {
    if files.is_empty() {
        return Err("Nothing translated to export.".to_string());
    }
    // Carry a timestamp from one source archive's header.
    let timestamp = archive::read_index(&files[0].arc_path)
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
    let packed = archive::pack_arc(&timestamp, &pack);
    std::fs::write(out_path, &packed).map_err(|e| format!("Failed to write patch: {}", e))?;
    Ok(changed)
}

/// Build a FULL replacement patch: rebuild `primary_arc` (the current official
/// patch, e.g. ver103) applying every edit by (path,name), copying all other
/// files verbatim, and appending any edited file that isn't in the primary arc
/// (pulled from its own source). Output replaces the official patch (drop as
/// ver999.arc and move ver102/ver103 out) — the proven ZAP-style install.
pub fn export_full(primary_arc: &str, files: &[ParentePatchFile], out_path: &str) -> Result<usize, String> {
    let data = std::fs::read(primary_arc).map_err(|e| format!("Failed to read {}: {}", primary_arc, e))?;
    let arc = archive::unpack_arc(&data)?;

    use std::collections::{HashMap, HashSet};
    let mut by_key: HashMap<(String, String), &ParentePatchFile> = HashMap::new();
    for f in files {
        by_key.insert((f.path.clone(), f.name.clone()), f);
    }

    let mut out_files = Vec::with_capacity(arc.entries.len());
    let mut used: HashSet<(String, String)> = HashSet::new();
    let mut changed = 0usize;
    for entry in &arc.entries {
        let raw = archive::entry_data(&data, entry)?;
        let key = (entry.path.clone(), entry.name.clone());
        let fdata = if let Some(f) = by_key.get(&key) {
            let b = bundle::unpack_bundle(raw)?;
            let (text, enc) = serialized::read_textasset(&b.blob)?;
            let new_text = script::apply_translations(&text, &f.translations);
            let new_blob = serialized::patch_textasset(&b.blob, &new_text, enc)?;
            used.insert(key);
            changed += 1;
            bundle::pack_bundle(&b, &new_blob)
        } else {
            raw.to_vec()
        };
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

    let packed = archive::pack_arc(&arc.timestamp, &out_files);
    std::fs::write(out_path, &packed).map_err(|e| format!("Failed to write patch: {}", e))?;
    Ok(changed)
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
fn english_patch_systemex(t: &str) -> String {
    let t = t.replacen(
        "if(LangNo==1) then return key1 end",
        "if(LangNo==2 and key2~=nil and key2~=\"\") then return key2 end return key1",
        1,
    );
    let t = t.replacen("if(LangNo==2) then return key2 end", "", 1);
    let t = t.replacen("if(LangNo==3) then return key3 end", "", 1);
    t.replacen("if(LangNo==4) then return key4 end", "", 1)
}

/// Enable the dormant in-config language buttons by flipping `bt_lang1..4`
/// `m_IsActive` false→true inside the `ui_config_basic` prefab (in-place byte
/// edits; blob size unchanged). The button sets the native `LangNo` and the
/// engine persists it in CONFIG.DAT, giving a native, persistent language menu.
fn enable_lang_buttons(raw: &[u8]) -> Result<Vec<u8>, String> {
    let mut b = bundle::unpack_bundle(raw)?;
    let mut blob = b.blob.clone();
    // Force the EXACT desired state: Japanese (1) + English (2) ON, Chinese (3)
    // and Traditional (4) OFF — set explicitly so we don't inherit a wrong
    // state if the source prefab was already edited (e.g. a prior ver999).
    for n in 1..=4u8 {
        let want: u8 = if n == 1 || n == 2 { 1 } else { 0 };
        let needle = format!("bt_lang{}", n).into_bytes();
        let mut i = 0;
        while i + needle.len() <= blob.len() {
            if blob[i..i + needle.len()] == needle[..]
                && i >= 4
                && blob[i - 4..i] == [8, 0, 0, 0]
            {
                // GameObject layout: name(8) + m_Tag(u16) + m_IsActive(u8).
                let active = i + 8 + 2;
                if active < blob.len() {
                    blob[active] = want;
                }
                break;
            }
            i += 1;
        }
    }
    b.blob = blob;
    let new = b.blob.clone();
    Ok(bundle::pack_bundle(&b, &new))
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

/// Build the NON-DESTRUCTIVE English patch: rebuild `primary_arc` with English
/// written into slot 2 of every edited scene (Japanese preserved in slot 1),
/// and the `system.txt` / `systemex.txt` functions patched so the engine renders
/// slot 2. Pulls `system.txt`/`systemex.txt` from elsewhere in the game if the
/// primary archive doesn't contain them. Output is a complete `ver999.arc`.
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
    let mut changed = 0usize;
    for entry in &arc.entries {
        let raw = archive::entry_data(&data, entry)?;
        let fdata = if entry.name == "system.txt" {
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
            raw.to_vec()
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
    let archives_dir = {
        let p = std::path::Path::new(primary_arc);
        let parent = p.parent().unwrap_or(std::path::Path::new("."));
        match parent.file_name().and_then(|s| s.to_str()) {
            Some("Patch") | Some("Append") => parent.parent().unwrap_or(parent).to_path_buf(),
            _ => parent.to_path_buf(),
        }
    };
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
    // Enable the in-config language buttons (native persistent toggle). The
    // prefab lives in a base archive; pull it, flip the buttons, and add it so
    // our copy overrides. Non-fatal if absent (just no in-game toggle then).
    let already_basic = out_files.iter().any(|f| f.name == "ui_config_basic");
    if !already_basic {
        if let Some((a, o, s)) = find_file_across(&archives_dir, "ui_config_basic") {
            if let Ok(raw) = archive::read_entry_at(&a, o, s) {
                if let Ok(data) = enable_lang_buttons(&raw) {
                    out_files.push(archive::PackFile { name: "ui_config_basic".into(), path: "/pfb/ui".into(), data });
                }
            }
        }
    }

    let packed = archive::pack_arc(&arc.timestamp, &out_files);
    std::fs::write(out_path, &packed).map_err(|e| format!("Failed to write patch: {}", e))?;
    Ok(changed)
}

/// If the dropped folder is a game install, return the path where a patch
/// should be written (`…\Start\Archives\Patch\ver999.arc`).
pub fn suggested_patch_path(dropped_dir: &str) -> Option<String> {
    let archives = find_archives_dir(dropped_dir)?;
    let patch = archives.join("Patch");
    if patch.is_dir() {
        patch.join("ver999.arc").to_str().map(|s| s.to_string())
    } else {
        archives.join("ver999.arc").to_str().map(|s| s.to_string())
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

/// Directory where Parente per-scene project files live (translations survive
/// closing/reopening). Under the app config dir, same as settings.json.
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

#[tauri::command]
pub async fn parente_save_project(app: tauri::AppHandle, key: String, content: String) -> Result<(), String> {
    let path = projects_dir(&app)?.join(format!("{}.json", key));
    std::fs::write(&path, content).map_err(|e| format!("save project: {}", e))
}

#[tauri::command]
pub async fn parente_load_project(app: tauri::AppHandle, key: String) -> Result<Option<String>, String> {
    let path = projects_dir(&app)?.join(format!("{}.json", key));
    if path.exists() {
        Ok(Some(std::fs::read_to_string(&path).map_err(|e| format!("load project: {}", e))?))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
