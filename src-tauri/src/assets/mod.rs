//! Browse & Extract workspace (Eushully suite, merge phase 1).
//!
//! Thin Tauri layer over the shared `eushully-assets` engine — the same code
//! the egui AGE Extractor and the `eushully` CLI use (archives for AGE SYS3-5
//! and Parente, `.fanpatch` mods, previews, extraction). This module only
//! keeps per-game sessions, converts results to JSON, and serves binary
//! payloads (decoded images) through the `eushully-asset://` protocol so they
//! never travel as JSON. Plan: Sys Decompiler `age-extractor/docs/SUITE_MERGE_PLAN.md`.
//!
//! Self-contained on purpose: its own managed state (`AssetsState`), its own
//! commands (`assets_*`), no changes to the editor's `AppState`/`commands.rs`.

pub mod commands;
pub mod protocol;
mod route;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use eushully_assets::archive::AgeArchive;
use eushully_assets::entry_index::{EntryFilter, EntryIndex};
use eushully_assets::extract::ExtractionHandle;

/// Decoded preview images kept per session (PNG bytes), served by the protocol.
const IMAGE_CACHE_LIMIT: usize = 48;

/// One opened game.
pub struct AssetSession {
    pub root: PathBuf,
    pub archive: Arc<AgeArchive>,
    pub index: EntryIndex,
    /// Last list query and its result, so paging doesn't re-filter.
    last_listing: Mutex<Option<(EntryFilter, Arc<Vec<usize>>)>>,
    /// entry index → encoded PNG of its preview.
    image_cache: Mutex<HashMap<usize, Arc<Vec<u8>>>>,
}

impl AssetSession {
    pub fn new(root: PathBuf, archive: AgeArchive, index: EntryIndex) -> Self {
        Self {
            root,
            archive: Arc::new(archive),
            index,
            last_listing: Mutex::new(None),
            image_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Filtered + sorted entry indices, reusing the previous result when the
    /// filter is unchanged (the table pages through one listing).
    pub fn listing(&self, filter: &EntryFilter) -> Arc<Vec<usize>> {
        let mut last = self.last_listing.lock().unwrap();
        if let Some((cached_filter, indices)) = last.as_ref() {
            if cached_filter == filter {
                return indices.clone();
            }
        }
        let indices = Arc::new(eushully_assets::entry_index::filter_and_sort(&self.archive, &self.index, filter));
        *last = Some((filter.clone(), indices.clone()));
        indices
    }

    pub fn cached_image(&self, entry_index: usize) -> Option<Arc<Vec<u8>>> {
        self.image_cache.lock().unwrap().get(&entry_index).cloned()
    }

    pub fn cache_image(&self, entry_index: usize, png: Arc<Vec<u8>>) {
        let mut cache = self.image_cache.lock().unwrap();
        if cache.len() >= IMAGE_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(entry_index, png);
    }
}

/// Managed state for the workspace.
#[derive(Default)]
pub struct AssetsState {
    sessions: Mutex<HashMap<u64, Arc<AssetSession>>>,
    extractions: Mutex<HashMap<u64, Arc<ExtractionHandle>>>,
    next_id: AtomicU64,
}

impl AssetsState {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn add_session(&self, session: AssetSession) -> u64 {
        let id = self.next_id();
        self.sessions.lock().unwrap().insert(id, Arc::new(session));
        id
    }

    pub fn session(&self, id: u64) -> Result<Arc<AssetSession>, String> {
        self.sessions.lock().unwrap().get(&id).cloned().ok_or_else(|| format!("game session {id} is not open"))
    }

    pub fn close_session(&self, id: u64) {
        self.sessions.lock().unwrap().remove(&id);
    }

    pub fn add_extraction(&self, handle: Arc<ExtractionHandle>) -> u64 {
        let id = self.next_id();
        self.extractions.lock().unwrap().insert(id, handle);
        id
    }

    pub fn extraction(&self, id: u64) -> Option<Arc<ExtractionHandle>> {
        self.extractions.lock().unwrap().get(&id).cloned()
    }

    pub fn finish_extraction(&self, id: u64) {
        self.extractions.lock().unwrap().remove(&id);
    }
}
