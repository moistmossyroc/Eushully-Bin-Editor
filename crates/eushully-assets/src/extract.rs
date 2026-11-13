// extract.rs — Background extraction with progress reporting and optional
// conversion (AGF → image format, FSB5 → WAV). Parallel read/decode/encode,
// sequential naming + writing (see `run_extraction`).
//
// Each entry is read once; what happens to it is decided by its CONTENT
// (utils::file_magic), not its name: AGE movies are named `.AGF` but are MPEG,
// Parente PNGs are named `.bytes`. Per-entry failures — including panics from
// malformed files — become error lines; they never stall the job.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::archive::{AgeArchive, ContainerHandles, FileEntry};
use crate::utils::output_paths::OutputPathClaims;
use crate::utils::parallel::parallel_map_with;

/// How to handle AGF files during extraction.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum AgfConvertFormat {
    AsIs,   // extract raw .AGF bytes unchanged
    Png,
    Bmp,
    Jpeg,
    Tga,
    Tiff,
}

impl AgfConvertFormat {
    pub const ALL: [AgfConvertFormat; 6] = [Self::AsIs, Self::Png, Self::Bmp, Self::Jpeg, Self::Tga, Self::Tiff];

    pub fn label(self) -> &'static str {
        match self {
            Self::AsIs => "As-Is (AGF)",
            Self::Png  => "PNG",
            Self::Bmp  => "BMP",
            Self::Jpeg => "JPEG",
            Self::Tga  => "TGA",
            Self::Tiff => "TIFF",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::AsIs => "agf",
            Self::Png  => "png",
            Self::Bmp  => "bmp",
            Self::Jpeg => "jpg",
            Self::Tga  => "tga",
            Self::Tiff => "tif",
        }
    }
    /// JPEG has no alpha channel; everything else here keeps transparency.
    pub fn supports_alpha(self) -> bool {
        self != Self::Jpeg
    }
}

/// User-chosen extraction settings.
#[derive(Clone, Copy, Debug)]
pub struct ExtractOptions {
    /// Drop directory structure, write every file straight into the output dir.
    pub flatten_paths: bool,
    pub agf_format: AgfConvertFormat,
    /// Transcode FMOD FSB5 audio (Parente) to WAV via vgmstream-cli.
    pub convert_fsb_to_wav: bool,
}

/// Messages sent from extraction thread to UI.
pub enum ExtractMsg {
    Progress { done: usize, total: usize, current_file: String },
    Done { extracted: usize, errors: Vec<String> },
    Cancelled,
}

/// Handle for a running extraction job.
pub struct ExtractionHandle {
    pub receiver: Receiver<ExtractMsg>,
    cancel: Arc<AtomicBool>,
}

impl ExtractionHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Start extracting `entries` from `archive` into `output_dir`.
pub fn start_extraction(
    archive: Arc<AgeArchive>,
    entries: Vec<FileEntry>,
    output_dir: PathBuf,
    options: ExtractOptions,
) -> ExtractionHandle {
    let (tx, rx) = bounded::<ExtractMsg>(128);
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    std::thread::spawn(move || {
        run_extraction(archive, entries, output_dir, options, tx, cancel_clone);
    });

    ExtractionHandle { receiver: rx, cancel }
}

/// Worker threads for read + decode + encode (PNG encoding dominates).
const MAX_EXTRACT_WORKERS: usize = 12;
/// Entries prepared per batch, per worker. Bounds memory held for writing.
const BATCH_ENTRIES_PER_WORKER: usize = 4;

/// Pipeline, per batch of entries:
///   1. parallel: read, decode, encode to memory (each worker keeps its own
///      container files open);
///   2. this thread: name every output in input order (`OutputPathClaims`),
///      so `_2` suffixes and tier overwrites match a one-at-a-time run;
///   3. parallel: write. When several entries of the batch claimed the same
///      path (tier copies), only the last in load order is written — it is
///      the one that would have survived the overwrites anyway.
fn run_extraction(
    archive: Arc<AgeArchive>,
    entries: Vec<FileEntry>,
    output_dir: PathBuf,
    options: ExtractOptions,
    tx: Sender<ExtractMsg>,
    cancel: Arc<AtomicBool>,
) {
    let total = entries.len();
    let mut errors: Vec<String> = Vec::new();
    let mut output_claims = OutputPathClaims::default();
    let workers = std::thread::available_parallelism().map_or(4, usize::from).clamp(1, MAX_EXTRACT_WORKERS);
    let mut done = 0;

    for batch in entries.chunks(workers * BATCH_ENTRIES_PER_WORKER) {
        if cancel.load(Ordering::Relaxed) {
            let _ = tx.send(ExtractMsg::Cancelled);
            return;
        }
        let prepared = parallel_map_with(batch, workers, ContainerHandles::default, |handles, entry| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare_entry(&archive, entry, options, handles)))
                .unwrap_or_else(|_| Err("internal error while decoding (malformed file?)".into()))
        });

        let mut pending_writes: Vec<PendingWrite> = Vec::with_capacity(batch.len());
        for (entry, outcome) in batch.iter().zip(prepared) {
            let _ = tx.send(ExtractMsg::Progress { done, total, current_file: entry.name.clone() });
            done += 1;
            match outcome {
                Ok(output) => {
                    let natural_path = resolve_output_path(&output_dir, &entry.name, options.flatten_paths, output.extension);
                    let path = output_claims.claim(natural_path, &entry.name);
                    pending_writes.push(PendingWrite { path, bytes: output.bytes, entry_name: entry.name.clone() });
                }
                Err(reason) => errors.push(format!("{}: {reason}", entry.name)),
            }
        }

        let writes = last_write_per_path(pending_writes);
        let write_results = parallel_map_with(&writes, workers, || (), |_, write| write_file(&write.path, &write.bytes));
        for (write, result) in writes.iter().zip(write_results) {
            if let Err(reason) = result {
                errors.push(format!("{}: {reason}", write.entry_name));
            }
        }
    }

    let extracted = total - errors.len();
    let _ = tx.send(ExtractMsg::Done { extracted, errors });
}

struct PendingWrite {
    path: PathBuf,
    bytes: Vec<u8>,
    entry_name: String,
}

/// Drop writes that a later write in the same list would overwrite
/// (paths compare case-insensitively, like the file system).
fn last_write_per_path(writes: Vec<PendingWrite>) -> Vec<PendingWrite> {
    let mut last_index_by_path: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (index, write) in writes.iter().enumerate() {
        last_index_by_path.insert(write.path.to_string_lossy().to_lowercase(), index);
    }
    writes
        .into_iter()
        .enumerate()
        .filter(|(index, write)| last_index_by_path.get(&write.path.to_string_lossy().to_lowercase()) == Some(index))
        .map(|(_, write)| write)
        .collect()
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    fs::write(path, bytes).map_err(|e| format!("write failed: {e}"))
}

/// An entry's final bytes, ready to write.
struct PreparedOutput {
    bytes: Vec<u8>,
    /// Extension override for the output name (`None` keeps the entry's).
    extension: Option<&'static str>,
}

fn prepare_entry(archive: &AgeArchive, entry: &FileEntry, options: ExtractOptions, handles: &mut ContainerHandles) -> Result<PreparedOutput, String> {
    let raw = archive.read_entry_with(entry, handles).map_err(|e| format!("read error: {e}"))?;
    match plan_output(&entry.name, raw, options)? {
        OutputPayload::Raw { bytes, extension } => Ok(PreparedOutput { bytes, extension }),
        OutputPayload::Image(image, format) => {
            Ok(PreparedOutput { bytes: encode_image(image, format)?, extension: Some(format.extension()) })
        }
    }
}

/// What to write for one entry, decided from its bytes.
enum OutputPayload {
    /// Bytes as stored (possibly unwrapped), saved under `extension` if set.
    Raw { bytes: Vec<u8>, extension: Option<&'static str> },
    /// A decoded image to encode in the chosen format.
    Image(image::RgbaImage, AgfConvertFormat),
}

fn plan_output(entry_name: &str, raw: Vec<u8>, options: ExtractOptions) -> Result<OutputPayload, String> {
    use crate::utils::file_magic::{detect_extension, name_extension};

    let is_agf_image = name_extension(entry_name) == "agf" && crate::agf::has_agf_magic(&raw);
    if is_agf_image && options.agf_format != AgfConvertFormat::AsIs {
        let image = crate::agf::decode_agf(&raw)?;
        return Ok(OutputPayload::Image(image, options.agf_format));
    }

    if options.convert_fsb_to_wav && detect_extension(&raw) == Some("fsb") {
        let wav = crate::utils::vgmstream::fsb_to_wav(&raw).map_err(|e| format!("FSB→WAV: {e}"))?;
        return Ok(OutputPayload::Raw { bytes: wav, extension: Some("wav") });
    }

    let extension = corrected_extension(entry_name, &raw);
    Ok(OutputPayload::Raw { bytes: raw, extension })
}

/// One entry, start to finish (real-data test).
#[cfg(test)]
fn extract_entry(
    archive: &AgeArchive,
    entry: &FileEntry,
    output_dir: &Path,
    options: ExtractOptions,
    output_claims: &mut OutputPathClaims,
) -> Result<(), String> {
    let output = prepare_entry(archive, entry, options, &mut ContainerHandles::default())?;
    let natural_path = resolve_output_path(output_dir, &entry.name, options.flatten_paths, output.extension);
    write_file(&output_claims.claim(natural_path, &entry.name), &output.bytes)
}

/// Encode in memory with the same encoders (and defaults) `RgbaImage::save` uses.
fn encode_image(image: image::RgbaImage, format: AgfConvertFormat) -> Result<Vec<u8>, String> {
    let image_format = match format {
        AgfConvertFormat::Png => image::ImageFormat::Png,
        AgfConvertFormat::Bmp => image::ImageFormat::Bmp,
        AgfConvertFormat::Jpeg => image::ImageFormat::Jpeg,
        AgfConvertFormat::Tga => image::ImageFormat::Tga,
        AgfConvertFormat::Tiff => image::ImageFormat::Tiff,
        AgfConvertFormat::AsIs => return Err("AGF images are written as-is, not encoded".into()),
    };
    let mut encoded = std::io::Cursor::new(Vec::new());
    let result = if format.supports_alpha() {
        image.write_to(&mut encoded, image_format)
    } else {
        image::DynamicImage::ImageRgba8(image).into_rgb8().write_to(&mut encoded, image_format)
    };
    result.map_err(|e| format!("image encode error: {e}"))?;
    Ok(encoded.into_inner())
}

/// Sniffed extension when it disagrees with the entry name (Parente PNGs named
/// `.bytes`, FSB5 named `.ogg`, extension-less bundles); `None` keeps the name.
fn corrected_extension(entry_name: &str, data: &[u8]) -> Option<&'static str> {
    use crate::utils::file_magic::{detect_extension, extension_matches, name_extension};
    let name_ext = name_extension(entry_name);
    detect_extension(data).filter(|sniffed| !extension_matches(sniffed, &name_ext))
}

/// Build the output path, optionally overriding the file extension.
fn resolve_output_path(
    output_dir: &Path,
    entry_name: &str,
    flatten: bool,
    override_ext: Option<&str>,
) -> PathBuf {
    let rel: PathBuf = if flatten {
        PathBuf::from(sanitize_filename(
            Path::new(entry_name)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| entry_name.to_string())
                .as_str()
        ))
    } else {
        entry_name.split(['/', '\\'])
            .filter(|c| !c.is_empty() && *c != ".." && *c != ".")
            .map(sanitize_filename)
            .collect()
    };

    let out_path = output_dir.join(&rel);
    if let Some(ext) = override_ext {
        out_path.with_extension(ext)
    } else {
        out_path
    }
}

fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_last_write_to_a_path_survives() {
        let write = |path: &str, entry: &str| PendingWrite { path: PathBuf::from(path), bytes: Vec::new(), entry_name: entry.into() };
        let kept = last_write_per_path(vec![
            write("out/a.png", "base/a"),
            write("out/b.png", "b"),
            write("OUT/A.png", "patch/a"),
        ]);
        let names: Vec<&str> = kept.iter().map(|write| write.entry_name.as_str()).collect();
        assert_eq!(names, ["b", "patch/a"]);
    }

    /// `AGE_DIR=<game> AGE_EXTRACT=<name,name> AGE_EXTRACT_OUT=<dir>`: runs real
    /// entries through `extract_entry` with AGF→PNG, FSB→WAV off.
    #[test]
    #[ignore]
    fn real_extract_entries() {
        let dir = std::env::var("AGE_DIR").expect("set AGE_DIR");
        let names = std::env::var("AGE_EXTRACT").expect("set AGE_EXTRACT");
        let out_dir = PathBuf::from(std::env::var("AGE_EXTRACT_OUT").expect("set AGE_EXTRACT_OUT"));
        let archive = AgeArchive::open(Path::new(&dir)).expect("open failed");
        let options = ExtractOptions { flatten_paths: true, agf_format: AgfConvertFormat::Png, convert_fsb_to_wav: false };
        for name in names.split(',') {
            let entry = archive.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)).expect("entry not found");
            extract_entry(&archive, entry, &out_dir, options, &mut OutputPathClaims::default()).expect("extract failed");
        }
    }
}
