//! Eushully `@ARCH000` archive container (used by the Unity-era titles such as
//! Yamiyo no Parente). Port of the community `extract.py` / `pack.py` tools.
//!
//! Layout:
//! ```text
//!   "@ARCH000"            8 bytes magic
//!   ts_len                u8
//!   timestamp             ts_len bytes
//!   <file data blobs concatenated>
//!   index:
//!     file_count          u32 LE
//!     per entry:
//!       name_len          u8
//!       name              name_len bytes (utf-8)
//!       offset            u64 LE   (absolute, into the file)
//!       size              u64 LE
//!       0x4E              marker byte
//!       path_len          u8
//!       path              path_len bytes ("/src/game" style, leading slash)
//!   index_offset          u64 LE   (last 8 bytes of file; points at file_count)
//! ```

const MAGIC: &[u8; 8] = b"@ARCH000";
const ENTRY_MARKER: u8 = 0x4E;

/// One file inside an `.arc`.
#[derive(Debug, Clone)]
pub struct ArcEntry {
    /// File name, e.g. `$1$sc0601.txt`.
    pub name: String,
    /// Directory path with leading slash, e.g. `/src/game`. May be empty.
    pub path: String,
    /// Absolute offset of the file data in the source archive.
    pub offset: u64,
    /// Length of the file data.
    pub size: u64,
}

/// Parsed archive: header preamble + entry table. Data is sliced lazily from
/// the original byte buffer via [`ArcEntry::offset`]/`size`.
pub struct ArcArchive {
    pub timestamp: Vec<u8>,
    pub entries: Vec<ArcEntry>,
}

fn rd_u32(d: &[u8], pos: usize) -> Result<u32, String> {
    d.get(pos..pos + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| "archive truncated (u32)".to_string())
}
fn rd_u64(d: &[u8], pos: usize) -> Result<u64, String> {
    d.get(pos..pos + 8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| "archive truncated (u64)".to_string())
}

/// Parse the archive header + index. Does not copy file data.
pub fn unpack_arc(d: &[u8]) -> Result<ArcArchive, String> {
    if d.len() < 17 || &d[0..8] != MAGIC {
        return Err("not an @ARCH000 archive".to_string());
    }
    let ts_len = d[8] as usize;
    let timestamp = d
        .get(9..9 + ts_len)
        .ok_or("archive truncated (timestamp)")?
        .to_vec();

    let index_offset = rd_u64(d, d.len() - 8)? as usize;
    let mut pos = index_offset;
    let file_count = rd_u32(d, pos)?;
    pos += 4;

    let mut entries = Vec::with_capacity(file_count as usize);
    for i in 0..file_count {
        let name_len = *d.get(pos).ok_or("archive truncated (name_len)")? as usize;
        pos += 1;
        let name = String::from_utf8_lossy(
            d.get(pos..pos + name_len).ok_or("archive truncated (name)")?,
        )
        .into_owned();
        pos += name_len;
        let offset = rd_u64(d, pos)?;
        pos += 8;
        let size = rd_u64(d, pos)?;
        pos += 8;
        let marker = *d.get(pos).ok_or("archive truncated (marker)")?;
        if marker != ENTRY_MARKER {
            return Err(format!("entry {} missing 0x4E marker", i));
        }
        pos += 1;
        let path_len = *d.get(pos).ok_or("archive truncated (path_len)")? as usize;
        pos += 1;
        let path = String::from_utf8_lossy(
            d.get(pos..pos + path_len).ok_or("archive truncated (path)")?,
        )
        .into_owned();
        pos += path_len;

        entries.push(ArcEntry { name, path, offset, size });
    }

    Ok(ArcArchive { timestamp, entries })
}

/// Borrow a single entry's file data from the original archive bytes.
pub fn entry_data<'a>(d: &'a [u8], e: &ArcEntry) -> Result<&'a [u8], String> {
    d.get(e.offset as usize..(e.offset + e.size) as usize)
        .ok_or_else(|| format!("entry {} data out of range", e.name))
}

/// Read only an archive's header + index via seeks — avoids loading the whole
/// file (base archives are ~1 GB). Returns the timestamp and entry table.
pub fn read_index(path: &str) -> Result<ArcArchive, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|e| format!("open {}: {}", path, e))?;
    let total = f
        .seek(SeekFrom::End(0))
        .map_err(|e| format!("seek: {}", e))?;

    let mut head = [0u8; 9];
    f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    f.read_exact(&mut head).map_err(|e| e.to_string())?;
    if &head[0..8] != MAGIC {
        return Err("not an @ARCH000 archive".to_string());
    }
    let ts_len = head[8] as usize;
    let mut timestamp = vec![0u8; ts_len];
    f.read_exact(&mut timestamp).map_err(|e| e.to_string())?;

    let mut off_buf = [0u8; 8];
    f.seek(SeekFrom::Start(total - 8)).map_err(|e| e.to_string())?;
    f.read_exact(&mut off_buf).map_err(|e| e.to_string())?;
    let index_offset = u64::from_le_bytes(off_buf);

    // Read the whole index region (small) in one shot.
    let mut index = vec![0u8; (total - 8 - index_offset) as usize];
    f.seek(SeekFrom::Start(index_offset)).map_err(|e| e.to_string())?;
    f.read_exact(&mut index).map_err(|e| e.to_string())?;

    let file_count = rd_u32(&index, 0)?;
    let mut pos = 4usize;
    let mut entries = Vec::with_capacity(file_count as usize);
    for i in 0..file_count {
        let name_len = *index.get(pos).ok_or("index truncated")? as usize;
        pos += 1;
        let name = String::from_utf8_lossy(&index[pos..pos + name_len]).into_owned();
        pos += name_len;
        let offset = rd_u64(&index, pos)?;
        pos += 8;
        let size = rd_u64(&index, pos)?;
        pos += 8;
        if *index.get(pos).ok_or("index truncated")? != ENTRY_MARKER {
            return Err(format!("entry {} missing marker", i));
        }
        pos += 1;
        let path_len = *index.get(pos).ok_or("index truncated")? as usize;
        pos += 1;
        let path = String::from_utf8_lossy(&index[pos..pos + path_len]).into_owned();
        pos += path_len;
        entries.push(ArcEntry { name, path, offset, size });
    }
    Ok(ArcArchive { timestamp, entries })
}

/// Read one entry's bytes via seek (no whole-file load).
pub fn read_entry_at(path: &str, offset: u64, size: u64) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|e| format!("open {}: {}", path, e))?;
    f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; size as usize];
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// A file to write into a packed archive.
pub struct PackFile {
    pub name: String,
    pub path: String,
    pub data: Vec<u8>,
}

/// Build a fresh `.arc` from the given files (already ordered by the caller).
/// `timestamp` is carried over from the source archive's header.
pub fn pack_arc(timestamp: &[u8], files: &[PackFile]) -> Vec<u8> {
    let header_end = 9 + timestamp.len();

    // Reserve offsets for each file's data block.
    let mut offsets = Vec::with_capacity(files.len());
    let mut cur = header_end as u64;
    for f in files {
        offsets.push(cur);
        cur += f.data.len() as u64;
    }
    let index_start = cur;

    // Build index.
    let mut index: Vec<u8> = Vec::new();
    index.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (f, &off) in files.iter().zip(&offsets) {
        let name_b = f.name.as_bytes();
        let path_b = f.path.as_bytes();
        index.push(name_b.len() as u8);
        index.extend_from_slice(name_b);
        index.extend_from_slice(&off.to_le_bytes());
        index.extend_from_slice(&(f.data.len() as u64).to_le_bytes());
        index.push(ENTRY_MARKER);
        index.push(path_b.len() as u8);
        index.extend_from_slice(path_b);
    }

    let mut out: Vec<u8> = Vec::with_capacity(index_start as usize + index.len() + 8);
    out.extend_from_slice(MAGIC);
    out.push(timestamp.len() as u8);
    out.extend_from_slice(timestamp);
    for f in files {
        out.extend_from_slice(&f.data);
    }
    out.extend_from_slice(&index);
    out.extend_from_slice(&index_start.to_le_bytes());
    out
}
