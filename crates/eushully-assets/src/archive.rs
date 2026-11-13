// archive.rs — Opens a game folder: parses SYS3/4/5INI.BIN via `age_index`,
// then resolves file entries into their respective .ALF containers.
// Also discovers loose asset files in the game directory that are not
// referenced by the index.
// Unity-era games (@ARCH000 .arc) are delegated to `crate::parente`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::age_index::{self, EntryTable};


/// Where a file entry's data actually lives.
#[derive(Debug, Clone)]
pub enum EntrySource {
    /// File is inside an ALF/AAI archive at a given offset.
    Alf {
        /// Index into `AgeArchive::alf_paths`
        archive_index: usize,
        offset: u64,
        size: u64,
    },
    /// Unity-era `@ARCH000` .arc entry (a UnityFS bundle); unwrapped to its
    /// payload on read by `crate::parente::decode_entry_payload`.
    Parente {
        /// Index into `AgeArchive::alf_paths`
        archive_index: usize,
        offset: u64,
        size: u64,
    },
    /// Entry of a `.fanpatch` mod archive (ZIP) served by the patch loader.
    Fanpatch {
        /// Index into `AgeArchive::alf_paths` (the .fanpatch file)
        archive_index: usize,
        zip_index: usize,
        size: u64,
    },
    /// File sits loose on disk at this path.
    Loose {
        path: PathBuf,
        size: u64,
    },
}

/// A single file entry — either inside an ALF archive or a loose file.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Display name
    pub name: String,
    /// Where the data lives
    pub source: EntrySource,
}

impl FileEntry {
    /// Size in bytes.
    pub fn size(&self) -> u64 {
        match &self.source {
            EntrySource::Alf { size, .. } => *size,
            EntrySource::Parente { size, .. } => *size,
            EntrySource::Fanpatch { size, .. } => *size,
            EntrySource::Loose { size, .. } => *size,
        }
    }

    /// Byte offset (0 for loose files).
    pub fn offset(&self) -> u64 {
        match &self.source {
            EntrySource::Alf { offset, .. } => *offset,
            EntrySource::Parente { offset, .. } => *offset,
            EntrySource::Fanpatch { .. } | EntrySource::Loose { .. } => 0,
        }
    }

    /// Archive index (usize::MAX sentinel for loose files).
    pub fn archive_index(&self) -> usize {
        match &self.source {
            EntrySource::Alf { archive_index, .. } => *archive_index,
            EntrySource::Parente { archive_index, .. } => *archive_index,
            EntrySource::Fanpatch { archive_index, .. } => *archive_index,
            EntrySource::Loose { .. } => usize::MAX,
        }
    }

    /// True if this entry lives loose on disk.
    pub fn is_loose(&self) -> bool {
        matches!(&self.source, EntrySource::Loose { .. })
    }
}

/// Top-level parsed archive, ready for display and extraction.
pub struct AgeArchive {
    /// Base directory containing SYS*INI.BIN and ALF files
    pub base_dir: PathBuf,
    /// Resolved paths to every referenced data container (ALF/AAI, or
    /// Parente .arc files in engine load order)
    pub alf_paths: Vec<PathBuf>,
    /// All file entries (indexed + loose)
    pub entries: Vec<FileEntry>,
    /// Version string derived from the magic header
    pub version: String,
    /// Per entry: index of the later entry that shadows it in a layered set
    /// (Parente base < Append < Patch). Empty for AGE titles.
    pub overridden_by: Vec<Option<usize>>,
    /// Per container (aligned with `alf_paths`): why the game does NOT load it,
    /// if it doesn't (Parente: only the version-matched patch is used). Its
    /// entries are still listed and comparable, but shadow nothing.
    /// Empty = every container is loaded.
    pub inactive_containers: Vec<Option<String>>,
    /// Per container (aligned with `alf_paths`): its tier, when the loader
    /// knows it from content (Parente translation-patch marker). Empty = derive
    /// from the container (AGE: APPEND* = official update, .fanpatch = mod).
    pub container_tiers: Vec<EntryTier>,
}

/// Extensions we skip when scanning for generic loose files.
/// ALF and AAI are handled separately in append_loose_files().
/// `.fanpatch` files are listed by content (see `append_fanpatches`).
const SKIP_EXTENSIONS: &[&str] = &["bin", "exe", "dll", "ini", "fanpatch"];

/// Who a file version comes from, for "original vs mod" views.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryTier {
    /// Shipped with the game (AGE DATA*.ALF / loose, Parente base archives).
    Original,
    /// Official patch/append from Eushully (AGE APPEND*.ALF, Parente Append or
    /// version patches in Patch).
    OfficialUpdate,
    /// Fan modification (`.fanpatch`, Parente `ver999.arc` built by the BIN editor).
    Mod,
}

impl EntryTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::OfficialUpdate => "Official update",
            Self::Mod => "Mod",
        }
    }
    pub fn is_original_content(self) -> bool {
        self != Self::Mod
    }
}

const AGE_UPDATE_ARCHIVE_PREFIX: &str = "append";

impl AgeArchive {
    /// The entry that shadows `entry_index`, if any (see `overridden_by`).
    pub fn overriding_entry(&self, entry_index: usize) -> Option<usize> {
        self.overridden_by.get(entry_index).copied().flatten()
    }

    /// Original / official update / mod, from the container the entry lives in.
    pub fn entry_tier(&self, entry_index: usize) -> EntryTier {
        let Some(entry) = self.entries.get(entry_index) else { return EntryTier::Original };
        let container = |archive_index: usize| self.alf_paths.get(archive_index).map(PathBuf::as_path);
        let lower_stem = |path: &Path| path.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()).unwrap_or_default();
        match &entry.source {
            EntrySource::Fanpatch { .. } => EntryTier::Mod,
            EntrySource::Loose { .. } => EntryTier::Original,
            EntrySource::Alf { archive_index, .. } => match container(*archive_index) {
                Some(path) if lower_stem(path).starts_with(AGE_UPDATE_ARCHIVE_PREFIX) => EntryTier::OfficialUpdate,
                _ => EntryTier::Original,
            },
            EntrySource::Parente { archive_index, .. } => {
                self.container_tiers.get(*archive_index).copied().unwrap_or(EntryTier::Original)
            }
        }
    }

    /// Why the game does not load the container at `archive_index`, if it doesn't.
    pub fn container_inactive_reason(&self, archive_index: usize) -> Option<&str> {
        self.inactive_containers.get(archive_index).and_then(Option::as_deref)
    }

    /// Why the game does not load this entry's container, if it doesn't.
    pub fn entry_inactive_reason(&self, entry_index: usize) -> Option<&str> {
        let entry = self.entries.get(entry_index)?;
        if entry.is_loose() {
            return None;
        }
        self.container_inactive_reason(entry.archive_index())
    }

    /// The copy the game actually loads: not shadowed, and in a loaded container.
    pub fn is_effective(&self, entry_index: usize) -> bool {
        self.overriding_entry(entry_index).is_none() && self.entry_inactive_reason(entry_index).is_none()
    }

    /// Name under which copies of one file are matched across tiers.
    fn version_key(&self, entry_index: usize) -> String {
        let entry = &self.entries[entry_index];
        match entry.source {
            EntrySource::Parente { .. } => entry.name.to_lowercase(),
            _ => crate::fanpatch::normalized_name(&entry.name),
        }
    }

    /// Every copy of the entry's file across tiers (loaded or not), in load
    /// order. Just `[entry_index]` when there are no other copies.
    pub fn version_history(&self, entry_index: usize) -> Vec<usize> {
        if entry_index >= self.entries.len() || self.entries[entry_index].is_loose() {
            return vec![entry_index];
        }
        let key = self.version_key(entry_index);
        (0..self.entries.len())
            .filter(|&index| !self.entries[index].is_loose() && self.version_key(index) == key)
            .collect()
    }

    /// Default pair for comparing `entry_index` with its other versions:
    ///   * the file has a mod version → (newest loaded original, that mod),
    ///     whichever copy was picked (the mod the game uses, else the last one);
    ///   * otherwise → (the entry's predecessor or first other copy, the entry).
    /// `None` without other copies.
    pub fn default_comparison(&self, entry_index: usize) -> Option<(usize, usize)> {
        let history = self.version_history(entry_index);
        if history.len() < 2 {
            return None;
        }
        let is_mod = |index: usize| self.entry_tier(index) == EntryTier::Mod;
        let mod_copies: Vec<usize> = history.iter().copied().filter(|&index| is_mod(index)).collect();
        let chosen_mod = if is_mod(entry_index) {
            Some(entry_index)
        } else {
            mod_copies.iter().copied().find(|&index| self.is_effective(index)).or(mod_copies.last().copied())
        };
        if let Some(mod_entry) = chosen_mod {
            let newest_loaded_original = history
                .iter()
                .copied()
                .filter(|&index| !is_mod(index) && self.entry_inactive_reason(index).is_none())
                .last();
            let left = newest_loaded_original.or_else(|| history.iter().copied().filter(|&index| index != mod_entry).last())?;
            return Some((left, mod_entry));
        }
        let others = || history.iter().copied().filter(|&index| index != entry_index);
        let left = others().filter(|&index| index < entry_index).last().or_else(|| others().next())?;
        Some((left, entry_index))
    }

    /// Attempt to open any recognised index file found in `dir`,
    /// then append any loose asset files found in the directory.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let candidates = ["SYS5INI.BIN", "sys5ini.bin",
                          "SYS4INI.BIN", "sys4ini.bin",
                          "SYS3INI.BIN", "sys3ini.bin"];

        for name in &candidates {
            let path = dir.join(name);
            if path.exists() {
                let mut arc = Self::parse_index(&path, dir)?;
                arc.append_loose_files();
                arc.append_fanpatches();
                arc.overridden_by = age_overrides(&arc.entries);
                return Ok(arc);
            }
        }
        if let Some(parente_result) = crate::parente::open(dir) {
            return parente_result;
        }
        Err(format!(
            "No recognisable index file found in {}\n\
             Expected SYS5INI.BIN, SYS4INI.BIN, SYS3INI.BIN, or Parente .arc files \
             (pick the game's Start or Archives folder).",
            dir.display()
        ))
    }

    /// List every `.fanpatch` entry (patch loader order, see `fanpatch.rs`).
    /// Unreadable files are skipped: the loader skips them too.
    fn append_fanpatches(&mut self) {
        for path in crate::fanpatch::find_fanpatches(&self.base_dir) {
            let Ok(fanpatch_entries) = crate::fanpatch::list_entries(&path) else { continue };
            let archive_index = self.alf_paths.len();
            self.alf_paths.push(path);
            self.entries.extend(fanpatch_entries.into_iter().map(|entry| FileEntry {
                name: entry.name,
                source: EntrySource::Fanpatch { archive_index, zip_index: entry.zip_index, size: entry.size },
            }));
        }
    }

    /// Scan `base_dir` for:
    ///   1. AAI+ALF pairs — parse the AAI as a SYS4-style index and enumerate contents
    ///   2. Lone ALF files (no AAI, not in main index) — add as a single raw entry
    ///   3. Other loose asset files not already indexed
    fn append_loose_files(&mut self) {
        let indexed_alfs: HashSet<PathBuf> = self.alf_paths.iter().cloned().collect();

        let skip_names: HashSet<String> = [
            "sys5ini.bin", "sys4ini.bin", "sys3ini.bin",
            "SYS5INI.BIN", "SYS4INI.BIN", "SYS3INI.BIN",
        ].iter().map(|s| s.to_lowercase()).collect();

        let Ok(dir_entries) = fs::read_dir(&self.base_dir) else { return };

        // Read the directory once. Use file_type() from the DirEntry — on most
        // platforms (Windows, Linux) this is returned for free alongside the
        // directory listing without an extra stat() call per entry.
        struct FileInfo { path: PathBuf, size: u64, ext: String, name_lower: String }
        let mut files: Vec<FileInfo> = Vec::new();

        for entry in dir_entries.flatten() {
            // file_type() reuses the dirent result — no extra syscall on Win/Linux
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_file() { continue; }

            let fname_os  = entry.file_name();
            let fname     = fname_os.to_string_lossy();
            let name_lower = fname.to_lowercase();

            if skip_names.contains(name_lower.as_str()) { continue; }

            let ext = Path::new(fname.as_ref())
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default()
                .to_string();

            if SKIP_EXTENSIONS.contains(&ext.as_str()) { continue; }

            let path = entry.path();

            // metadata() is needed for size — one stat() per file, unavoidable.
            // We batch this inside the already-running background thread so it
            // never blocks the UI.
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);

            files.push(FileInfo { path, size, ext, name_lower });
        }

        // Sort for deterministic display
        files.sort_by(|a, b| a.name_lower.cmp(&b.name_lower));

        let mut alf_consumed: HashSet<PathBuf> = indexed_alfs;
        let mut aai_files: Vec<usize> = Vec::new();
        let mut alf_indices: Vec<usize> = Vec::new();

        for (i, f) in files.iter().enumerate() {
            match f.ext.as_str() {
                "aai" => aai_files.push(i),
                "alf" => alf_indices.push(i),
                _ => {}
            }
        }

        // Build a lowercase filename → path lookup for the whole directory,
        // used by resolve_alf_path to avoid repeated read_dir calls.
        let dir_cache: HashMap<String, PathBuf> = files.iter()
            .map(|f| {
                let name_lower = f.path.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default()
                    .to_string();
                (name_lower, f.path.clone())
            })
            .collect();

        // --- 1. AAI+ALF pairs ---
        for &ai in &aai_files {
            let aai_path = &files[ai].path;
            let stem_lower = aai_path.file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            // Find matching ALF using the dir cache (no extra syscall)
            let alf_path = {
                let direct = aai_path.with_extension("alf");
                if direct.exists() {
                    direct
                } else if let Some(p) = dir_cache.get(&format!("{stem_lower}.alf")) {
                    p.clone()
                } else {
                    continue; // no matching ALF
                }
            };

            alf_consumed.insert(alf_path.clone());

            let aai_raw = match fs::read(aai_path) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let label = aai_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| aai_path.display().to_string());

            if let Ok(mut temp_arc) = Self::parse_aai_raw(&aai_raw, &alf_path, &label) {
                let base_idx = self.alf_paths.len();
                self.alf_paths.extend(temp_arc.alf_paths.drain(..));
                for entry in temp_arc.entries.drain(..) {
                    if let EntrySource::Alf { archive_index, offset, size } = entry.source {
                        self.entries.push(FileEntry {
                            name: entry.name,
                            source: EntrySource::Alf {
                                archive_index: archive_index + base_idx,
                                offset,
                                size,
                            },
                        });
                    }
                }
            }
        }

        // --- 2. Lone ALF files ---
        for &ai in &alf_indices {
            let f = &files[ai];
            if alf_consumed.contains(&f.path) { continue; }
            let fname = f.path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            self.entries.push(FileEntry {
                name: fname,
                source: EntrySource::Loose { path: f.path.clone(), size: f.size },
            });
        }

        // --- 3. Other loose asset files ---
        for f in &files {
            if matches!(f.ext.as_str(), "aai" | "alf") { continue; }
            let fname = f.path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            self.entries.push(FileEntry {
                name: fname,
                source: EntrySource::Loose { path: f.path.clone(), size: f.size },
            });
        }
    }

    /// Parse an AAI file (a SYS4-style index for a single ALF).
    /// Every `alf_paths` slot points at `alf_path`; entries keep their stored index.
    fn parse_aai_raw(raw: &[u8], alf_path: &Path, label: &str) -> Result<Self, String> {
        let table = age_index::parse_aai_bytes(raw).map_err(|error| format!("{label}: {error}"))?;
        let alf_paths = vec![alf_path.to_path_buf(); table.archive_names.len()];
        let base_dir = alf_path.parent().unwrap_or(alf_path);
        Ok(Self::from_entry_table(base_dir, alf_paths, table, String::new()))
    }

    /// Read and parse SYS3INI / SYS4INI / SYS5INI.BIN (format details in `age_index.rs`).
    fn parse_index(ini_path: &Path, base_dir: &Path) -> Result<Self, String> {
        let raw = fs::read(ini_path).map_err(|e| format!("Cannot read {}: {}", ini_path.display(), e))?;
        let (version, table) = age_index::parse_index_bytes(&raw)?;
        let alf_paths = table.archive_names.iter().map(|name| resolve_alf_path(base_dir, name)).collect();
        Ok(Self::from_entry_table(base_dir, alf_paths, table, version.to_string()))
    }

    fn from_entry_table(base_dir: &Path, alf_paths: Vec<PathBuf>, table: EntryTable, version: String) -> Self {
        let entries = table
            .entries
            .into_iter()
            .map(|entry| FileEntry {
                name: entry.name,
                source: EntrySource::Alf { archive_index: entry.archive_index, offset: entry.offset, size: entry.size },
            })
            .collect();
        AgeArchive { base_dir: base_dir.to_path_buf(), alf_paths, entries, version, overridden_by: Vec::new(), inactive_containers: Vec::new(), container_tiers: Vec::new() }
    }

    /// Read the raw bytes for a specific entry.
    /// Parente entries are unwrapped to their payload (PNG, script, FSB5, MP4…).
    pub fn read_entry(&self, entry: &FileEntry) -> io::Result<Vec<u8>> {
        self.read_entry_with(entry, &mut ContainerHandles::default())
    }

    /// `read_entry`, reusing open container files from `handles` (bulk reads:
    /// extraction would otherwise reopen the same ALF thousands of times).
    pub fn read_entry_with(&self, entry: &FileEntry, handles: &mut ContainerHandles) -> io::Result<Vec<u8>> {
        match &entry.source {
            EntrySource::Alf { archive_index, offset, size } => {
                read_range(handles.file(*archive_index, &self.alf_paths[*archive_index])?, *offset, *size)
            }
            EntrySource::Parente { archive_index, offset, size } => {
                let raw = read_range(handles.file(*archive_index, &self.alf_paths[*archive_index])?, *offset, *size)?;
                crate::parente::decode_entry_payload(&entry.name, raw)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
            }
            EntrySource::Fanpatch { archive_index, zip_index, .. } => {
                crate::fanpatch::read_entry(&self.alf_paths[*archive_index], *zip_index)
            }
            EntrySource::Loose { path, size } => {
                let mut f = fs::File::open(path)?;
                let mut buf = vec![0u8; *size as usize];
                f.read_exact(&mut buf)?;
                Ok(buf)
            }
        }
    }
}

impl AgeArchive {
    /// First `max_len` bytes of an entry's stored data, for previews of
    /// containers too big to load (lone ALF archives are several GB).
    /// Parente entries must be unwrapped whole, so they ignore the cap.
    pub fn read_entry_prefix(&self, entry: &FileEntry, max_len: u64) -> io::Result<Vec<u8>> {
        match &entry.source {
            EntrySource::Alf { archive_index, offset, size } => {
                read_file_range(&self.alf_paths[*archive_index], *offset, (*size).min(max_len))
            }
            EntrySource::Loose { path, size } => read_file_range(path, 0, (*size).min(max_len)),
            EntrySource::Parente { .. } | EntrySource::Fanpatch { .. } => self.read_entry(entry),
        }
    }
}

/// Read `size` bytes at `offset` from `path` (seek; never loads the whole file).
fn read_file_range(path: &Path, offset: u64, size: u64) -> io::Result<Vec<u8>> {
    read_range(&mut fs::File::open(path)?, offset, size)
}

fn read_range(file: &mut fs::File, offset: u64, size: u64) -> io::Result<Vec<u8>> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; size as usize];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

/// Container files (ALF / .arc) kept open between reads, keyed by archive index.
/// One per thread: a `File`'s seek position is not shareable.
#[derive(Default)]
pub struct ContainerHandles {
    open_files: HashMap<usize, fs::File>,
}

impl ContainerHandles {
    fn file(&mut self, archive_index: usize, path: &Path) -> io::Result<&mut fs::File> {
        match self.open_files.entry(archive_index) {
            std::collections::hash_map::Entry::Occupied(open) => Ok(open.into_mut()),
            std::collections::hash_map::Entry::Vacant(slot) => Ok(slot.insert(fs::File::open(path)?)),
        }
    }
}



fn resolve_alf_path(base_dir: &Path, stored_name: &str) -> PathBuf {
    resolve_alf_path_cached(base_dir, stored_name, None)
}

/// Resolve an ALF path using an optional pre-built lowercase filename→path map.
fn resolve_alf_path_cached(
    base_dir: &Path,
    stored_name: &str,
    dir_cache: Option<&HashMap<String, PathBuf>>,
) -> PathBuf {
    let file_name = Path::new(stored_name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| stored_name.to_string());

    let direct = base_dir.join(&file_name);
    if direct.exists() {
        return direct;
    }

    let lower = file_name.to_lowercase();

    // Use pre-built cache if provided (avoids repeated fs::read_dir calls)
    if let Some(cache) = dir_cache {
        if let Some(p) = cache.get(&lower) {
            return p.clone();
        }
        return base_dir.join(file_name);
    }

    // Fallback: single fs::read_dir scan
    if let Ok(entries) = fs::read_dir(base_dir) {
        for entry in entries.flatten() {
            let ename = entry.file_name();
            if ename.to_string_lossy().to_lowercase() == lower {
                return entry.path();
            }
        }
    }

    base_dir.join(file_name)
}

/// Shadowing for AGE games: index entries < APPEND (AAI) entries < `.fanpatch`
/// entries, which is the order they are listed in. Names compare like the
/// patch loader (base name, case-insensitive). Loose files are not layered.
fn age_overrides(entries: &[FileEntry]) -> Vec<Option<usize>> {
    let keys: Vec<String> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| match entry.source {
            EntrySource::Loose { .. } => format!("\0loose\0{index}"),
            _ => crate::fanpatch::normalized_name(&entry.name),
        })
        .collect();
    crate::utils::layer_overrides::find_overriding_entries(keys.iter().map(String::as_str))
}
