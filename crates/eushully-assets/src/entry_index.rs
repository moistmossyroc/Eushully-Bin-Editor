// entry_index.rs — Per-archive lookup data computed ONCE on the loading
// thread, so filtering/sorting 15k+ entries never re-lowercases names or
// re-classifies types per keystroke.

use std::collections::HashMap;

use crate::archive::{AgeArchive, EntryTier};
use crate::utils::entry_kind::{classify_entry, EntryKind};
use crate::utils::folder_tree::{build_folder_tree, FolderNode};

pub struct EntryIndex {
    pub lower_names: Vec<String>,
    pub kinds: Vec<EntryKind>,
    pub kind_counts: HashMap<EntryKind, usize>,
    pub folder_tree: FolderNode,
    /// effective entry → its shadowed copies, earliest tier first.
    pub shadowed_copies: HashMap<usize, Vec<usize>>,
    pub has_agf_entries: bool,
    /// Entries the game does not load (shadowed, or in a skipped patch).
    pub overridden_count: usize,
    /// Original / official update / mod per entry.
    pub tiers: Vec<EntryTier>,
    pub mod_count: usize,
}

pub fn build_entry_index(archive: &AgeArchive) -> EntryIndex {
    let lower_names: Vec<String> = archive.entries.iter().map(|e| e.name.to_lowercase()).collect();
    let kinds: Vec<EntryKind> = archive.entries.iter().map(|e| classify_entry(&e.name)).collect();

    let mut kind_counts: HashMap<EntryKind, usize> = HashMap::new();
    for kind in &kinds {
        *kind_counts.entry(*kind).or_default() += 1;
    }

    let mut shadowed_copies: HashMap<usize, Vec<usize>> = HashMap::new();
    for (index, overriding) in archive.overridden_by.iter().enumerate() {
        if let Some(effective) = overriding {
            shadowed_copies.entry(*effective).or_default().push(index);
        }
    }

    let tiers: Vec<EntryTier> = (0..archive.entries.len()).map(|index| archive.entry_tier(index)).collect();
    EntryIndex {
        folder_tree: build_folder_tree(archive.entries.iter().map(|e| e.name.as_str())),
        has_agf_entries: lower_names.iter().any(|name| name.ends_with(".agf")),
        overridden_count: (0..archive.entries.len()).filter(|&index| !archive.is_effective(index)).count(),
        mod_count: tiers.iter().filter(|&&tier| tier == EntryTier::Mod).count(),
        tiers,
        lower_names,
        kinds,
        kind_counts,
        shadowed_copies,
    }
}

impl EntryIndex {
    /// (left, right) copies to compare for `entry_index`, if the file has
    /// other versions (see `AgeArchive::default_comparison`).
    pub fn comparison_pair(&self, archive: &AgeArchive, entry_index: usize) -> Option<(usize, usize)> {
        archive.default_comparison(entry_index)
    }
}

/// List column to sort by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortColumn {
    #[default]
    Name,
    Kind,
    Archive,
    Offset,
    Size,
}

/// What the file list shows (shared by every front-end so they filter alike).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EntryFilter {
    /// Case-insensitive substring of the name.
    pub search: String,
    pub kind: Option<EntryKind>,
    /// Lower-cased folder prefix (see `utils::folder_tree`); empty = all.
    pub folder: String,
    /// Only copies the game loads.
    pub effective_only: bool,
    /// Only mod-archive files.
    pub mods_only: bool,
    pub sort: SortColumn,
    pub descending: bool,
}

/// Entry indices passing `filter`, sorted as requested.
pub fn filter_and_sort(archive: &AgeArchive, index: &EntryIndex, filter: &EntryFilter) -> Vec<usize> {
    let query = filter.search.trim().to_lowercase();
    let mut indices: Vec<usize> = (0..archive.entries.len())
        .filter(|&i| {
            let lower_name = &index.lower_names[i];
            (!filter.effective_only || archive.is_effective(i))
                && (!filter.mods_only || index.tiers[i] == EntryTier::Mod)
                && filter.kind.map_or(true, |kind| index.kinds[i] == kind)
                && crate::utils::folder_tree::is_in_folder(lower_name, &filter.folder)
                && (query.is_empty() || lower_name.contains(&query))
        })
        .collect();
    let entries = &archive.entries;
    match filter.sort {
        SortColumn::Name => indices.sort_by(|&a, &b| index.lower_names[a].cmp(&index.lower_names[b])),
        SortColumn::Kind => indices.sort_by(|&a, &b| {
            index.kinds[a].cmp(&index.kinds[b]).then_with(|| index.lower_names[a].cmp(&index.lower_names[b]))
        }),
        SortColumn::Archive => indices.sort_by_key(|&i| entries[i].archive_index()),
        SortColumn::Offset => indices.sort_by_key(|&i| entries[i].offset()),
        SortColumn::Size => indices.sort_by_key(|&i| entries[i].size()),
    }
    if filter.descending {
        indices.reverse();
    }
    indices
}
