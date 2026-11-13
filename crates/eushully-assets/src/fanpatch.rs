// fanpatch.rs — `.fanpatch` mod archives served by the eushully patch loader
// (version.dll proxy; source: "eushully fanpatch zips tool/- eushully patch
// loader[…].zip", src/patch_archive.cpp). Mirrors its rules exactly:
//   * every `*.fanpatch` in the game folder, a plain ZIP (stored or deflate);
//   * sorted by file name (std::sort, byte order) and indexed in that order,
//     later archives — and later entries within one — replace earlier ones;
//   * entry names are reduced to their base name and compared upper-cased;
//     directories are skipped;
//   * a matching name replaces the game's own file of that name.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const FANPATCH_EXTENSION: &str = "fanpatch";

/// The loader's name key: base name, ASCII upper-case.
pub fn normalized_name(name: &str) -> String {
    name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_uppercase()
}

pub fn is_fanpatch_file(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(FANPATCH_EXTENSION))
}

/// `*.fanpatch` files in `game_dir`, in the loader's load order.
pub fn find_fanpatches(game_dir: &Path) -> Vec<PathBuf> {
    let Ok(dir_entries) = fs::read_dir(game_dir) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = dir_entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| is_fanpatch_file(path))
        .collect();
    paths.sort_by(|a, b| a.file_name().map(|n| n.as_encoded_bytes()).cmp(&b.file_name().map(|n| n.as_encoded_bytes())));
    paths
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanpatchEntry {
    /// Base name as stored (the game sees it under this name).
    pub name: String,
    /// Index inside the ZIP (for reading).
    pub zip_index: usize,
    pub size: u64,
}

fn open_zip(path: &Path) -> io::Result<zip::ZipArchive<fs::File>> {
    zip::ZipArchive::new(fs::File::open(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Every file entry of one `.fanpatch`, in ZIP order.
pub fn list_entries(path: &Path) -> io::Result<Vec<FanpatchEntry>> {
    let mut archive = open_zip(path)?;
    let mut entries = Vec::with_capacity(archive.len());
    for zip_index in 0..archive.len() {
        let file = archive.by_index_raw(zip_index).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if file.is_dir() {
            continue;
        }
        let stored_name = file.name();
        let name = stored_name.rsplit(['/', '\\']).next().unwrap_or(stored_name).to_string();
        entries.push(FanpatchEntry { name, zip_index, size: file.size() });
    }
    Ok(entries)
}

/// One entry's (decompressed) bytes.
pub fn read_entry(path: &Path, zip_index: usize) -> io::Result<Vec<u8>> {
    let mut archive = open_zip(path)?;
    let mut file = archive.by_index(zip_index).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut bytes = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Write a `.fanpatch` (deflate; the loader reads it via miniz). Names should
/// be bare file names — the loader ignores folders anyway.
pub fn write_fanpatch(path: &Path, files: &[(String, Vec<u8>)]) -> io::Result<()> {
    let mut writer = zip::ZipWriter::new(fs::File::create(path)?);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in files {
        writer.start_file(name.as_str(), options).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        writer.write_all(bytes)?;
    }
    writer.finish().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_normalize_like_the_loader() {
        assert_eq!(normalized_name("sub/dir\\Fpbanner.agf"), "FPBANNER.AGF");
        assert_eq!(normalized_name("SC0000.BIN"), "SC0000.BIN");
    }

    #[test]
    fn write_list_read_round_trip_and_load_order() {
        let dir = crate::utils::temp_files::unique_temp_path("fanpatch_test", "dir");
        fs::create_dir_all(&dir).unwrap();
        write_fanpatch(&dir.join("02_b.fanpatch"), &[("X.AGF".into(), vec![2; 10])]).unwrap();
        write_fanpatch(&dir.join("01_a.fanpatch"), &[("x.agf".into(), vec![1; 10]), ("SC.BIN".into(), b"hello".to_vec())]).unwrap();
        fs::write(dir.join("01_a.fanpatch.bak"), b"not loaded").unwrap();

        let order: Vec<String> = find_fanpatches(&dir).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(order, ["01_a.fanpatch", "02_b.fanpatch"]);
        let entries = list_entries(&dir.join("01_a.fanpatch")).unwrap();
        assert_eq!(entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["x.agf", "SC.BIN"]);
        assert_eq!(read_entry(&dir.join("01_a.fanpatch"), entries[1].zip_index).unwrap(), b"hello");
        fs::remove_dir_all(&dir).unwrap();
    }
}
