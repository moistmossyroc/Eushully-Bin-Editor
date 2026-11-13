//! `assets_*` Tauri commands. Heavy work runs on `spawn_blocking`; results are
//! plain serde structs (camelCase) mirrored in `src/workspaces/assets/types.ts`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eushully_assets::archive::{AgeArchive, EntryTier};
use eushully_assets::entry_index::{build_entry_index, EntryFilter};
use eushully_assets::extract::{start_extraction, AgfConvertFormat, ExtractMsg, ExtractOptions};
use eushully_assets::image::{ImageFormat, RgbaImage};
use eushully_assets::preview::{preview_entry, PreviewContent};
use eushully_assets::utils::entry_kind::EntryKind;
use eushully_assets::utils::game_discovery::{discover, Discovery, GameKind};
use eushully_assets::utils::layer_overrides::extraction_order;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use super::{AssetSession, AssetsState};

/// Progress events at most this often (a 20k-file extraction would otherwise flood the UI).
const PROGRESS_EVENT_INTERVAL: Duration = Duration::from_millis(100);
pub const EXTRACT_PROGRESS_EVENT: &str = "assets://extract-progress";
pub const EXTRACT_DONE_EVENT: &str = "assets://extract-done";

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| format!("background task failed: {e}"))?
}

fn tier_name(tier: EntryTier) -> &'static str {
    match tier {
        EntryTier::Original => "original",
        EntryTier::OfficialUpdate => "officialUpdate",
        EntryTier::Mod => "mod",
    }
}

fn container_name(archive: &AgeArchive, archive_index: usize) -> String {
    archive
        .alf_paths
        .get(archive_index)
        .and_then(|path| path.file_name())
        .map_or_else(|| format!("#{archive_index}"), |name| name.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Open / close
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameRootInfo {
    pub path: String,
    pub kind: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedGame {
    pub session: u64,
    pub root: String,
    pub version: String,
    pub entry_count: usize,
    pub archive_count: usize,
    /// Entries the game does not load (shadowed or in a skipped container).
    pub not_effective_count: usize,
    pub mod_count: usize,
    pub kind_counts: HashMap<EntryKind, usize>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OpenGameResult {
    Opened(OpenedGame),
    /// Several games below the picked folder: the user picks one.
    Choose { roots: Vec<GameRootInfo> },
}

fn game_kind_label(kind: GameKind) -> &'static str {
    match kind {
        GameKind::Age => "age",
        GameKind::Parente => "parente",
    }
}

/// Open a game folder (any level: subfolders are searched like the AGE Extractor).
#[tauri::command]
pub async fn assets_open_game(state: State<'_, AssetsState>, path: String) -> Result<OpenGameResult, String> {
    let picked = PathBuf::from(&path);
    let opened = blocking(move || match discover(&picked) {
        Discovery::NotFound => Err(format!(
            "No AGE (SYS3/4/5INI.BIN) or Parente (Archives\\*.arc) game found in or below {}",
            picked.display()
        )),
        Discovery::Multiple(roots) => Ok(Err(roots
            .into_iter()
            .map(|root| GameRootInfo { path: root.path.to_string_lossy().into_owned(), kind: game_kind_label(root.kind) })
            .collect::<Vec<_>>())),
        Discovery::Single(root) => {
            let archive = AgeArchive::open(&root.path)?;
            let index = build_entry_index(&archive);
            Ok(Ok(AssetSession::new(root.path, archive, index)))
        }
    })
    .await?;
    let session = match opened {
        Err(roots) => return Ok(OpenGameResult::Choose { roots }),
        Ok(session) => session,
    };
    let summary = OpenedGame {
        session: 0,
        root: session.root.to_string_lossy().into_owned(),
        version: session.archive.version.clone(),
        entry_count: session.archive.entries.len(),
        archive_count: session.archive.alf_paths.len(),
        not_effective_count: session.index.overridden_count,
        mod_count: session.index.mod_count,
        kind_counts: session.index.kind_counts.clone(),
    };
    let id = state.add_session(session);
    Ok(OpenGameResult::Opened(OpenedGame { session: id, ..summary }))
}

/// `--browse <folder> [--show <file>]` given at launch, if any.
#[tauri::command]
pub fn assets_startup_request() -> Option<super::route::BrowseRequest> {
    super::route::browse_request_from_args(std::env::args())
}

/// Entry named `name` (case-insensitive); the copy the game loads wins.
#[tauri::command]
pub async fn assets_find_entry(state: State<'_, AssetsState>, session: u64, name: String) -> Result<Option<usize>, String> {
    let session = state.session(session)?;
    blocking(move || {
        let archive = &session.archive;
        let matches: Vec<usize> = (0..archive.entries.len()).filter(|&i| archive.entries[i].name.eq_ignore_ascii_case(&name)).collect();
        Ok(matches.iter().copied().find(|&i| archive.is_effective(i)).or(matches.last().copied()))
    })
    .await
}

#[tauri::command]
pub fn assets_close_game(state: State<'_, AssetsState>, session: u64) {
    state.close_session(session);
}

// ---------------------------------------------------------------------------
// Listing (paged)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryRow {
    pub index: usize,
    pub name: String,
    pub kind: EntryKind,
    pub archive: String,
    pub offset: u64,
    pub size: u64,
    pub tier: &'static str,
    /// The game loads this copy.
    pub effective: bool,
    /// Why not (shadowed by …, or the container's reason); `None` when effective.
    pub note: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryPage {
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<EntryRow>,
}

fn entry_row(session: &AssetSession, index: usize) -> EntryRow {
    let archive = &session.archive;
    let entry = &archive.entries[index];
    let note = match (archive.entry_inactive_reason(index), archive.overriding_entry(index)) {
        (Some(reason), _) => Some(reason.to_string()),
        (None, Some(winner)) => Some(format!("shadowed: the game loads the copy in {}", container_name(archive, archive.entries[winner].archive_index()))),
        (None, None) => None,
    };
    EntryRow {
        index,
        name: entry.name.clone(),
        kind: session.index.kinds[index],
        archive: if entry.is_loose() { "(loose)".into() } else { container_name(archive, entry.archive_index()) },
        offset: entry.offset(),
        size: entry.size(),
        tier: tier_name(session.index.tiers[index]),
        effective: note.is_none(),
        note,
    }
}

/// One page of the filtered, sorted list.
#[tauri::command]
pub async fn assets_list(
    state: State<'_, AssetsState>,
    session: u64,
    filter: EntryFilter,
    offset: usize,
    limit: usize,
) -> Result<EntryPage, String> {
    let session = state.session(session)?;
    blocking(move || {
        let listing = session.listing(&filter);
        let rows = listing.iter().skip(offset).take(limit).map(|&index| entry_row(&session, index)).collect();
        Ok(EntryPage { total: listing.len(), offset, rows })
    })
    .await
}

/// Every entry index matching the filter (for "select all shown").
#[tauri::command]
pub async fn assets_list_indices(state: State<'_, AssetsState>, session: u64, filter: EntryFilter) -> Result<Vec<usize>, String> {
    let session = state.session(session)?;
    blocking(move || Ok(session.listing(&filter).as_ref().clone())).await
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PreviewResult {
    /// Fetch the pixels from `eushully-asset://<session>/<index>/image`.
    #[serde(rename_all = "camelCase")]
    Image { width: u32, height: u32 },
    Text { text: String },
    #[serde(rename_all = "camelCase")]
    Audio { format: String, duration_secs: Option<f32>, size: usize },
    Video { description: String },
    Hex { text: String },
    Error { message: String },
}

fn encode_png(width: u32, height: u32, rgba: Vec<u8>) -> Result<Vec<u8>, String> {
    let image = RgbaImage::from_raw(width, height, rgba).ok_or("image buffer size mismatch")?;
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).map_err(|e| format!("PNG encode failed: {e}"))?;
    Ok(png.into_inner())
}

/// Decode an entry for the preview pane. Images are encoded to PNG once and
/// cached for the protocol; everything else comes back inline.
pub fn build_preview(session: &AssetSession, index: usize) -> PreviewResult {
    let Some(entry) = session.archive.entries.get(index) else {
        return PreviewResult::Error { message: format!("no entry #{index}") };
    };
    match preview_entry(&session.archive, entry) {
        PreviewContent::Image(image) => {
            let (width, height) = (image.width, image.height);
            match encode_png(width, height, image.pixels) {
                Ok(png) => {
                    session.cache_image(index, Arc::new(png));
                    PreviewResult::Image { width, height }
                }
                Err(message) => PreviewResult::Error { message },
            }
        }
        PreviewContent::Text(text) => PreviewResult::Text { text },
        PreviewContent::Audio(audio) => PreviewResult::Audio {
            format: format!("{:?}", audio.format),
            duration_secs: audio.duration_secs,
            size: audio.bytes.len(),
        },
        PreviewContent::Video(video) => PreviewResult::Video { description: video.description },
        PreviewContent::HexDump(text) => PreviewResult::Hex { text },
        PreviewContent::Error(message) => PreviewResult::Error { message },
        PreviewContent::None | PreviewContent::Loading => PreviewResult::Error { message: "nothing to preview".into() },
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryPreview {
    pub row: EntryRow,
    pub preview: PreviewResult,
}

#[tauri::command]
pub async fn assets_preview(state: State<'_, AssetsState>, session: u64, index: usize) -> Result<EntryPreview, String> {
    let session = state.session(session)?;
    blocking(move || {
        if index >= session.archive.entries.len() {
            return Err(format!("no entry #{index}"));
        }
        Ok(EntryPreview { row: entry_row(&session, index), preview: build_preview(&session, index) })
    })
    .await
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractRequest {
    pub indices: Vec<usize>,
    pub output_dir: String,
    pub flatten: bool,
    pub agf_format: AgfConvertFormat,
    pub fsb_to_wav: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExtractProgress {
    pub job: u64,
    pub done: usize,
    pub total: usize,
    pub current_file: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExtractDone {
    pub job: u64,
    pub extracted: usize,
    pub errors: Vec<String>,
    pub cancelled: bool,
    pub output_dir: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractStarted {
    pub job: u64,
    /// Files that will be written (shadowed copies of chosen files skipped).
    pub total: usize,
    pub skipped_shadowed: usize,
}

/// Start extracting; progress/done arrive as events. Writes in load order and
/// skips copies shadowed by another chosen copy (same rules as the extractor).
#[tauri::command]
pub fn assets_extract(app: AppHandle, state: State<'_, AssetsState>, session: u64, request: ExtractRequest) -> Result<ExtractStarted, String> {
    let session = state.session(session)?;
    let chosen = request.indices.len();
    let order = extraction_order(request.indices, &session.archive.overridden_by);
    if order.is_empty() {
        return Err("nothing selected to extract".into());
    }
    let entries = order.iter().map(|&index| session.archive.entries[index].clone()).collect::<Vec<_>>();
    let total = entries.len();
    let options = ExtractOptions { flatten_paths: request.flatten, agf_format: request.agf_format, convert_fsb_to_wav: request.fsb_to_wav };
    let output_dir = request.output_dir.clone();
    let handle = Arc::new(start_extraction(session.archive.clone(), entries, PathBuf::from(&request.output_dir), options));
    let job = state.add_extraction(handle.clone());

    std::thread::spawn(move || {
        let mut last_progress = Instant::now() - PROGRESS_EVENT_INTERVAL;
        let done_event = loop {
            match handle.receiver.recv() {
                Ok(ExtractMsg::Progress { done, total, current_file }) => {
                    if last_progress.elapsed() >= PROGRESS_EVENT_INTERVAL {
                        last_progress = Instant::now();
                        let _ = app.emit(EXTRACT_PROGRESS_EVENT, ExtractProgress { job, done, total, current_file });
                    }
                }
                Ok(ExtractMsg::Done { extracted, errors }) => {
                    break ExtractDone { job, extracted, errors, cancelled: false, output_dir };
                }
                Ok(ExtractMsg::Cancelled) => break ExtractDone { job, extracted: 0, errors: Vec::new(), cancelled: true, output_dir },
                Err(_) => break ExtractDone { job, extracted: 0, errors: vec!["extraction stopped unexpectedly".into()], cancelled: false, output_dir },
            }
        };
        if let Some(state) = tauri::Manager::try_state::<AssetsState>(&app) {
            state.finish_extraction(job);
        }
        let _ = app.emit(EXTRACT_DONE_EVENT, done_event);
    });

    Ok(ExtractStarted { job, total, skipped_shadowed: chosen - total })
}

#[tauri::command]
pub fn assets_cancel_extract(state: State<'_, AssetsState>, job: u64) {
    if let Some(handle) = state.extraction(job) {
        handle.cancel();
    }
}
