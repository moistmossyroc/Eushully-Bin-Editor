// parente.rs — Backend for Unity-era Eushully games (Yamiyo no Parente
// Remastered and siblings): `@ARCH000` .arc containers whose entries are
// UnityFS bundles.
//
// Format parsing lives in the shared `parente-arc` crate (also used by the bin
// editor); this module only does discovery, naming, and payload unwrapping.
//
// Payload rules (verified against the retail archives):
//   * `/img/**/*.bytes`, `/src/**/*.txt` — TextAsset; payload is the original
//     file verbatim (PNG images, Lua-style scripts).
//   * `/ogg/**`, `/mp4/**` — AudioClip/VideoClip; the media lives in the
//     bundle's `.resource` node (FMOD FSB5 audio, plain MP4 video).
//   * everything else (prefabs with textures, manifests) — returned as-is, so
//     an extracted file opens directly in AssetStudio / UABEA.

use std::fs;
use std::path::{Path, PathBuf};

use parente_arc::{archive as arc_format, bundle, serialized};

use crate::archive::{AgeArchive, EntrySource, EntryTier, FileEntry};
use crate::utils::layer_overrides::find_overriding_entries;

const ARCHIVES_DIR_NAME: &str = "Archives";
const ARC_EXTENSION: &str = "arc";
const APPEND_DIR: &str = "append";
const PATCH_DIR: &str = "patch";
/// Marker entry the BIN editor stamps into every translation patch
/// (editor `parente/patch_stamp.rs`); never requested by the game.
const TRANSLATION_MARKER_DIR: &str = "/tlpatch";
const TRANSLATION_MARKER_NAME: &str = "patch_info.txt";
/// Name every pre-marker translation export used.
const LEGACY_TRANSLATION_FILE_NAME: &str = "ver999.arc";

// Load model (BIN editor parente/PARENTE_ENGINE.md §7, from the decompiled
// `Donskoy.Filemapper.AddArchives`/`AddArchive`, corrected 2026-11-10):
//   * EVERY `*.arc` under `Archives` is loaded, recursively; names don't matter;
//   * per contained file, a copy replaces the loaded one only if its archive's
//     header timestamp (ASCII YYYYMMDDhhmmss after `@ARCH000`) is strictly
//     newer; a tie keeps the archive enumerated first.
// Enumeration (`Directory.GetFiles(.., AllDirectories)`) is approximated as a
// folder's files by name, then its subfolders by name, depth-first. For a
// standard install that is root < Append < Patch, the same tie order the
// editor's `collect_archives` uses.

/// All `.arc` files under `root`, in (approximate) engine enumeration order.
/// Disabled patches (`*.arc.bak`) are skipped by the extension check.
fn collect_arc_paths_in_enumeration_order(root: &Path) -> Vec<PathBuf> {
    let Ok(dir_entries) = fs::read_dir(root) else { return Vec::new() };
    let (mut files, mut folders): (Vec<PathBuf>, Vec<PathBuf>) = (Vec::new(), Vec::new());
    for entry in dir_entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            folders.push(entry.path());
        } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case(ARC_EXTENSION)) {
            files.push(entry.path());
        }
    }
    let lower_name = |path: &PathBuf| path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    files.sort_by_key(lower_name);
    folders.sort_by_key(lower_name);
    files.into_iter().chain(folders.iter().flat_map(|folder| collect_arc_paths_in_enumeration_order(folder))).collect()
}

/// Header timestamp as a number (0 if not all digits, like a failed `ToUInt64`).
fn header_timestamp_value(timestamp: &[u8]) -> u64 {
    std::str::from_utf8(timestamp).ok().and_then(|text| text.trim_end_matches('\0').parse().ok()).unwrap_or(0)
}

/// Archive indices (into the enumeration order) sorted lowest → highest
/// priority: older timestamp first; on a tie the FIRST enumerated wins, so it
/// goes later.
fn load_priority_order(timestamps_in_enumeration_order: &[u64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..timestamps_in_enumeration_order.len()).collect();
    order.sort_by_key(|&index| (timestamps_in_enumeration_order[index], std::cmp::Reverse(index)));
    order
}

/// A translation patch built by the BIN editor (marker entry, or legacy name).
pub fn is_translation_patch(file_name: &str, arc: &arc_format::ArcArchive) -> bool {
    file_name.eq_ignore_ascii_case(LEGACY_TRANSLATION_FILE_NAME)
        || arc.entries.iter().any(|e| e.path == TRANSLATION_MARKER_DIR && e.name == TRANSLATION_MARKER_NAME)
}

/// Mod for translation patches; official update for anything under Append /
/// Patch; original for the base archives.
fn container_tier(path: &Path, root: &Path, is_translation: bool) -> EntryTier {
    if is_translation {
        return EntryTier::Mod;
    }
    let first_folder = path
        .strip_prefix(root)
        .ok()
        .and_then(|relative| relative.parent())
        .and_then(|parent| parent.components().next())
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase());
    match first_folder.as_deref() {
        Some(APPEND_DIR | PATCH_DIR) => EntryTier::OfficialUpdate,
        _ => EntryTier::Original,
    }
}

const UNITYFS_MAGIC: &[u8] = b"UnityFS\0";
const MEDIA_NODE_SUFFIX: &str = ".resource";
const TEXTASSET_EXTENSIONS: [&str; 2] = ["bytes", "txt"];

pub const VERSION_LABEL: &str = "Parente @ARCH000";

/// `dir/Archives` when present (user picked the game's `Start` folder),
/// otherwise `dir` itself (user picked `Archives` or a tier folder directly).
fn find_archive_root(dir: &Path) -> PathBuf {
    let nested = dir.join(ARCHIVES_DIR_NAME);
    if nested.is_dir() { nested } else { dir.to_path_buf() }
}

/// Display name for an entry: `/img/ev` + `ev157bd.bytes` -> `img/ev/ev157bd.bytes`.
pub fn entry_display_name(entry_dir: &str, entry_file_name: &str) -> String {
    let trimmed_dir = entry_dir.trim_matches('/');
    if trimmed_dir.is_empty() {
        entry_file_name.to_string()
    } else {
        format!("{trimmed_dir}/{entry_file_name}")
    }
}

/// Try to open `dir` as a Parente game. Returns `None` when no `.arc` files
/// exist there (so the caller can report "unrecognised folder"), or
/// `Some(Err)` when `.arc` files exist but none are valid `@ARCH000`.
/// Containers (and their entries) are listed in load priority order, so the
/// generic "last copy wins" shadowing matches the engine's timestamp rule.
pub fn open(dir: &Path) -> Option<Result<AgeArchive, String>> {
    let root = find_archive_root(dir);
    let candidate_paths = collect_arc_paths_in_enumeration_order(&root);
    if candidate_paths.is_empty() {
        return None;
    }

    let mut readable: Vec<(PathBuf, arc_format::ArcArchive)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for path in candidate_paths {
        match path.to_str().ok_or_else(|| "non-UTF-8 path".to_string()).and_then(arc_format::read_index) {
            Ok(index) => readable.push((path, index)),
            Err(e) => skipped.push(format!("{}: {e}", path.display())),
        }
    }
    if readable.is_empty() {
        return Some(Err(format!(
            "Found .arc files in {} but none are @ARCH000 archives:\n{}",
            root.display(),
            skipped.join("\n")
        )));
    }

    let timestamps: Vec<u64> = readable.iter().map(|(_, index)| header_timestamp_value(&index.timestamp)).collect();
    let mut slots: Vec<Option<(PathBuf, arc_format::ArcArchive)>> = readable.into_iter().map(Some).collect();
    let mut arc_paths: Vec<PathBuf> = Vec::with_capacity(slots.len());
    let mut container_tiers: Vec<EntryTier> = Vec::with_capacity(slots.len());
    let mut entries: Vec<FileEntry> = Vec::new();
    for enumeration_index in load_priority_order(&timestamps) {
        let (path, index) = slots[enumeration_index].take().expect("each archive is placed once");
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        container_tiers.push(container_tier(&path, &root, is_translation_patch(&file_name, &index)));
        let archive_index = arc_paths.len();
        entries.extend(index.entries.into_iter().map(|e| FileEntry {
            name: entry_display_name(&e.path, &e.name),
            source: EntrySource::Parente { archive_index, offset: e.offset, size: e.size },
        }));
        arc_paths.push(path);
    }

    let overridden_by = find_overriding_entries(entries.iter().map(|e| e.name.as_str()));
    Some(Ok(AgeArchive {
        base_dir: root,
        alf_paths: arc_paths,
        entries,
        version: VERSION_LABEL.to_string(),
        overridden_by,
        inactive_containers: Vec::new(),
        container_tiers,
    }))
}

fn has_textasset_extension(entry_name: &str) -> bool {
    let ext = crate::utils::file_magic::name_extension(entry_name);
    TEXTASSET_EXTENSIONS.contains(&ext.as_str())
}

fn node_bytes<'a>(blob: &'a [u8], node: &bundle::BundleNode) -> Result<&'a [u8], String> {
    let start = usize::try_from(node.offset).map_err(|_| "negative node offset")?;
    let len = usize::try_from(node.size).map_err(|_| "negative node size")?;
    blob.get(start..start + len)
        .ok_or_else(|| format!("bundle node {} out of range", node.path))
}

/// Unwrap a raw `.arc` entry into its useful payload (see module header for
/// the rules). Takes ownership so pass-through cases don't copy.
pub fn decode_entry_payload(entry_name: &str, raw_entry: Vec<u8>) -> Result<Vec<u8>, String> {
    if !raw_entry.starts_with(UNITYFS_MAGIC) {
        return Ok(raw_entry); // e.g. `*.manifest` text files
    }
    let parsed = bundle::unpack_bundle(&raw_entry)?;

    if let Some(media_node) = parsed.nodes.iter().find(|n| n.path.ends_with(MEDIA_NODE_SUFFIX)) {
        return node_bytes(&parsed.blob, media_node).map(<[u8]>::to_vec);
    }
    if has_textasset_extension(entry_name) {
        if let Ok(payload) = serialized::read_textasset_bytes(&parsed.blob) {
            return Ok(payload.to_vec());
        }
    }
    Ok(raw_entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_joins_dir_and_file() {
        assert_eq!(entry_display_name("/img/ev", "ev157bd.bytes"), "img/ev/ev157bd.bytes");
        assert_eq!(entry_display_name("", "assetbundle"), "assetbundle");
        assert_eq!(entry_display_name("/", "abd"), "abd");
    }

    #[test]
    fn non_bundle_entries_pass_through() {
        let manifest = b"ManifestFileVersion: 0".to_vec();
        assert_eq!(decode_entry_payload("pfb/abd.manifest", manifest.clone()).unwrap(), manifest);
    }

    /// Against a real install: `PARENTE_DIR=<game Start or Archives dir>
    /// [PARENTE_SAMPLE_OUT=<dir>] cargo test --release -- --ignored --nocapture`.
    /// Decodes the first entry of every top-level folder and checks the
    /// payload type the rules promise.
    #[test]
    #[ignore]
    fn real_archives_smoke() {
        use crate::utils::file_magic::effective_extension;
        use std::collections::HashSet;

        let dir = std::env::var("PARENTE_DIR").expect("set PARENTE_DIR");
        let sample_out = std::env::var("PARENTE_SAMPLE_OUT").ok().map(PathBuf::from);
        let archive = open(Path::new(&dir)).expect("no .arc found").expect("open failed");
        let overridden_count = archive.overridden_by.iter().filter(|o| o.is_some()).count();
        println!(
            "{} archives, {} entries, {overridden_count} overridden by a later tier",
            archive.alf_paths.len(),
            archive.entries.len()
        );

        let mut seen_groups: HashSet<String> = HashSet::new();
        for entry in &archive.entries {
            let group: String = entry.name.split('/').take(2).collect::<Vec<_>>().join("/");
            if !seen_groups.insert(group.clone()) { continue; }

            let payload = archive.read_entry(entry).expect("read failed");
            let ext = effective_extension(&entry.name, &payload);
            println!("{group:<24} {:<40} -> {ext:<7} {} bytes", entry.name, payload.len());

            let expected = match group.split('/').next().unwrap_or("") {
                "img" => Some("png"),
                "ogg" => Some("fsb"),
                "mp4" => Some("mp4"),
                "src" => Some("txt"),
                _ => None,
            };
            if let Some(expected) = expected {
                assert_eq!(ext, expected, "{}", entry.name);
            }
            if ext == "fsb" {
                match crate::utils::vgmstream::fsb_to_wav(&payload) {
                    Ok(wav) => println!("{:<24} FSB→WAV ok, {} bytes", "", wav.len()),
                    Err(reason) => println!("{:<24} FSB→WAV skipped: {reason}", ""),
                }
            }
            if let Some(out_dir) = &sample_out {
                let file_name = format!("{}.{ext}", group.replace('/', "_"));
                fs::create_dir_all(out_dir).unwrap();
                fs::write(out_dir.join(file_name), &payload).unwrap();
            }
        }
    }

    #[test]
    fn textasset_rule_matches_bytes_and_txt_only() {
        assert!(has_textasset_extension("img/ev/ev157bd.bytes"));
        assert!(has_textasset_extension("src/game/sc0001.TXT"));
        assert!(!has_textasset_extension("ogg/bgm/bgm002.ogg"));
        assert!(!has_textasset_extension("pfb/ui/ui_title"));
    }
}

#[cfg(test)]
mod load_order_tests {
    use super::*;

    #[test]
    fn newest_timestamp_wins_and_ties_keep_the_first_enumerated() {
        // enumeration: base, append (ties with base), patch102, patch103, translation (newest)
        let order = load_priority_order(&[100, 100, 200, 300, 400]);
        assert_eq!(order, vec![1, 0, 2, 3, 4]); // base outranks append on the tie
        assert_eq!(header_timestamp_value(b"20260519152611"), 20260519152611);
        assert_eq!(header_timestamp_value(b"garbage"), 0);
    }

    #[test]
    fn tiers_from_folder_and_translation_marker() {
        let root = Path::new("A");
        assert_eq!(container_tier(Path::new("A/ver00d0.arc"), root, false), EntryTier::Original);
        assert_eq!(container_tier(Path::new("A/Append/ver00x1.arc"), root, false), EntryTier::OfficialUpdate);
        assert_eq!(container_tier(Path::new("A/Patch/ver103_x1.arc"), root, false), EntryTier::OfficialUpdate);
        assert_eq!(container_tier(Path::new("A/Patch/moist English TL v1.0.arc"), root, true), EntryTier::Mod);
    }

    #[test]
    fn enumeration_lists_folder_files_then_subfolders() {
        let root = crate::utils::temp_files::unique_temp_path("parente_enum", "dir");
        for path in ["b.arc", "a.arc", "Patch/z.arc", "Append/x.arc", "notes.txt"] {
            let full = root.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, b"x").unwrap();
        }
        let names: Vec<String> = collect_arc_paths_in_enumeration_order(&root)
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(names, ["a.arc", "b.arc", "Append/x.arc", "Patch/z.arc"]);
        fs::remove_dir_all(root).unwrap();
    }
}
