//! SYS3/SYS4 master catalog (`SYS3INI.BIN` / `SYS4INI.BIN`) — the engine's
//! file table + startup settings. Layout in docs/engine/age-sys.md §2.

use crate::alf;

/// Keys that start the settings trailer in every installed SYS3/SYS4 build
/// (SYS3 begins at SCREENX; SYS4 at CREATEOBJECT).
const SETTINGS_ANCHORS: [&str; 3] = ["CREATEOBJECT", "DRAWMODE", "SCREENX"];
const ENTRY_LEN: usize = 0x50;
const VOLUME_NAME_LEN: usize = 0x100;

/// Index geometry per generation (docs/engine/age-sys.md §2).
#[derive(Clone, Copy)]
struct Layout {
    volume_name_len: usize,
    entry_len: usize,
    /// Bytes reserved for the entry name (SJIS for SYS3/4, UTF-16LE for SYS5).
    name_len: usize,
    wide_names: bool,
}

const LAYOUT_SYS34: Layout = Layout { volume_name_len: VOLUME_NAME_LEN, entry_len: ENTRY_LEN, name_len: 0x40, wide_names: false };
const LAYOUT_SYS5: Layout = Layout { volume_name_len: 0x200, entry_len: 0x90, name_len: 0x80, wide_names: true };

fn layout_of(data: &[u8]) -> Layout {
    if alf::is_sys5_catalog(data) { LAYOUT_SYS5 } else { LAYOUT_SYS34 }
}

/// Parsed master catalog: identity + startup settings (no file data).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MasterCatalog {
    /// e.g. `S4IC` (compressed SYS4 master) or `S3IC`.
    pub magic: String,
    /// 3-digit engine build, e.g. `443`.
    pub build: String,
    pub title: String,
    pub entry_count: u32,
    /// Startup settings trailer, in file order (keys may repeat, e.g. JUSTIFYCHAR).
    pub settings: Vec<(String, String)>,
}

impl MasterCatalog {
    pub fn setting(&self, key: &str) -> Option<&str> {
        self.settings.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Logical canvas from `SCREENX`/`SCREENY` (the engine's own 640×480
    /// fallback applies only when they are absent).
    pub fn screen_size(&self) -> (u32, u32) {
        let parse = |key: &str| self.setting(key).and_then(|v| v.parse().ok());
        (parse("SCREENX").unwrap_or(640), parse("SCREENY").unwrap_or(480))
    }

    /// SYS generation from the magic (`3`, `4`).
    pub fn generation(&self) -> u8 {
        self.magic.as_bytes().get(1).map(|b| b - b'0').unwrap_or(0)
    }
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn sjis(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    encoding_rs::SHIFT_JIS.decode(&bytes[..end]).0.into_owned()
}

/// Decompressed index blob + where its entry table ends.
struct IndexBlob {
    triple_offset: usize,
    blob: Vec<u8>,
    count_offset: usize,
    entry_count: u32,
}

fn inflate(data: &[u8], layout: Layout) -> Result<IndexBlob, String> {
    let triple_offset = alf::find_sys4_triple(data).ok_or("catalog: no LZSS block found")?;
    let blob = alf::inflate_sys4_index(data, triple_offset).ok_or("catalog: LZSS inflate failed")?;
    let volume_count = read_u32(&blob, 0).ok_or("catalog: empty index")? as usize;
    let count_offset = 4 + volume_count * layout.volume_name_len;
    let entry_count = read_u32(&blob, count_offset).ok_or("catalog: truncated index")?;
    if count_offset + 4 + entry_count as usize * layout.entry_len > blob.len() {
        return Err("catalog: entry table exceeds index".into());
    }
    Ok(IndexBlob { triple_offset, blob, count_offset, entry_count })
}

/// One NUL-terminated string in the settings trailer, with its byte span.
struct TrailerString {
    text: String,
    /// Byte just past the terminator.
    end: usize,
}

fn trailer_string(tail: &[u8], position: usize, wide: bool) -> Option<TrailerString> {
    if position >= tail.len() {
        return None;
    }
    let (text, terminator_end) = if wide {
        let units: Vec<u16> = tail[position..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let length = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        (String::from_utf16_lossy(&units[..length]), position + (length + 1) * 2)
    } else {
        let length = tail[position..].iter().position(|&b| b == 0).unwrap_or(tail.len() - position);
        (sjis(&tail[position..position + length]), position + length + 1)
    };
    Some(TrailerString { text, end: terminator_end.min(tail.len()) })
}

fn encode_trailer_string(text: &str, wide: bool) -> Vec<u8> {
    if wide {
        text.encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()).collect()
    } else {
        let mut bytes = encoding_rs::SHIFT_JIS.encode(text).0.into_owned();
        bytes.push(0);
        bytes
    }
}

/// Settings key/value list in the trailer: the pairs + the byte span they occupy
/// (up to, not including, the empty-key terminator). Keys may repeat (JUSTIFYCHAR).
fn settings_list(tail: &[u8], wide: bool) -> Option<(Vec<(String, String)>, std::ops::Range<usize>)> {
    let step = if wide { 2 } else { 1 };
    let start = SETTINGS_ANCHORS
        .iter()
        .filter_map(|key| {
            let needle = encode_trailer_string(key, wide);
            (0..tail.len().saturating_sub(needle.len() - 1))
                .step_by(step)
                .find(|&p| tail[p..].starts_with(&needle) && (p == 0 || tail[p - step..p].iter().all(|&b| b == 0)))
        })
        .min()?;
    let mut position = start;
    let mut pairs = Vec::new();
    while let Some(key) = trailer_string(tail, position, wide) {
        if key.text.is_empty() {
            break;
        }
        let Some(value) = trailer_string(tail, key.end, wide) else { break };
        pairs.push((key.text, value.text));
        position = value.end;
    }
    Some((pairs, start..position))
}

/// SYS5 (`S5IC`, UTF-16) catalog: identity + settings (Hyakusen, build 502).
fn read_sys5_catalog(data: &[u8]) -> Result<MasterCatalog, String> {
    let header = alf::utf16_cstr(&data[..0x10]);
    let index = inflate(data, LAYOUT_SYS5)?;
    let entries_end = index.count_offset + 4 + index.entry_count as usize * LAYOUT_SYS5.entry_len;
    Ok(MasterCatalog {
        magic: header.chars().take(4).collect(),
        build: header.chars().skip(4).take(3).collect(),
        title: alf::utf16_cstr(data.get(0x10..0x210).unwrap_or_default()),
        entry_count: index.entry_count,
        settings: settings_list(index.blob.get(entries_end..).unwrap_or_default(), true).map(|(pairs, _)| pairs).unwrap_or_default(),
    })
}

/// Parse a compressed SYS3/SYS4/SYS5 master catalog.
pub fn read_catalog(data: &[u8]) -> Result<MasterCatalog, String> {
    if alf::is_sys5_catalog(data) {
        return read_sys5_catalog(data);
    }
    let magic = data.get(..4).map(|m| String::from_utf8_lossy(m).into_owned()).ok_or("catalog: file too short")?;
    if !matches!(magic.as_str(), "S3IC" | "S4IC") {
        return Err(format!("catalog: unsupported magic {magic:?} (SYS5/uncompressed not handled yet)"));
    }
    let build = String::from_utf8_lossy(&data[4..7]).into_owned();
    let title = sjis(data.get(8..0x100).unwrap_or_default());
    let index = inflate(data, LAYOUT_SYS34)?;
    let entries_end = index.count_offset + 4 + index.entry_count as usize * ENTRY_LEN;
    Ok(MasterCatalog {
        magic,
        build,
        title,
        entry_count: index.entry_count,
        settings: settings_list(&index.blob[entries_end..], false).map(|(pairs, _)| pairs).unwrap_or_default(),
    })
}

/// A record to add to the catalog: served loose (by name) by the fanpatch loader.
pub struct NewRecord<'a> {
    pub name: &'a str,
    pub length: u32,
}

/// Append records after the last entry (ids `entry_count..`), keep the
/// settings trailer, re-compress (store mode), rewrite the LZSS triple.
/// Existing ids are unchanged, so saves stay compatible. Returns
/// (new catalog bytes, id of the first added record). Verified in-game on
/// Madou (443) and Tenmei (482); SYS5 uses the same scheme with its wider layout.
pub fn extend_catalog(original: &[u8], records: &[NewRecord]) -> Result<(Vec<u8>, u32), String> {
    let layout = layout_of(original);
    let index = inflate(original, layout)?;
    let insert_at = index.count_offset + 4 + index.entry_count as usize * layout.entry_len;
    let mut added = Vec::with_capacity(records.len() * layout.entry_len);
    for record in records {
        let name: Vec<u8> = if layout.wide_names {
            record.name.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
        } else {
            encoding_rs::SHIFT_JIS.encode(record.name).0.into_owned()
        };
        if name.len() + if layout.wide_names { 2 } else { 1 } > layout.name_len {
            return Err(format!("catalog: record name too long: {}", record.name));
        }
        let mut entry = vec![0u8; layout.entry_len];
        entry[..name.len()].copy_from_slice(&name);
        // volume 0 / offset 0 (served loose); length is the last u32 of the entry.
        entry[layout.entry_len - 4..].copy_from_slice(&record.length.to_le_bytes());
        added.extend(entry);
    }
    let mut blob = index.blob;
    let new_count = index.entry_count + records.len() as u32;
    blob[index.count_offset..index.count_offset + 4].copy_from_slice(&new_count.to_le_bytes());
    blob.splice(insert_at..insert_at, added);
    Ok((repack(original, index.triple_offset, &blob), index.entry_count))
}

/// Re-compress (store mode) an edited index blob behind the original header.
fn repack(original: &[u8], triple_offset: usize, blob: &[u8]) -> Vec<u8> {
    let compressed = alf::lzss_store(blob);
    let mut out = original[..triple_offset].to_vec();
    for value in [blob.len() as u32, blob.len() as u32, compressed.len() as u32] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&compressed);
    out
}

/// Set startup settings (e.g. SYS5 `LANGUAGE`, `FONT`): replaces the first
/// occurrence of each key, appends missing keys at the end of the list.
/// Entries and ids are untouched, so saves and archives stay compatible.
pub fn set_catalog_settings(original: &[u8], updates: &[(&str, &str)]) -> Result<Vec<u8>, String> {
    let layout = layout_of(original);
    let index = inflate(original, layout)?;
    let entries_end = index.count_offset + 4 + index.entry_count as usize * layout.entry_len;
    let tail = &index.blob[entries_end..];
    let (mut pairs, span) = settings_list(tail, layout.wide_names).ok_or("catalog: settings list not found")?;
    for &(key, value) in updates {
        match pairs.iter_mut().find(|(k, _)| k == key) {
            Some(pair) => pair.1 = value.to_string(),
            None => pairs.push((key.to_string(), value.to_string())),
        }
    }
    let encoded: Vec<u8> = pairs
        .iter()
        .flat_map(|(k, v)| [encode_trailer_string(k, layout.wide_names), encode_trailer_string(v, layout.wide_names)])
        .flatten()
        .collect();
    let mut blob = index.blob.clone();
    blob.splice(entries_end + span.start..entries_end + span.end, encoded);
    Ok(repack(original, index.triple_offset, &blob))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal synthetic S4IC master: 1 volume, 2 entries, settings trailer.
    fn synthetic_master() -> Vec<u8> {
        let mut blob = Vec::new();
        blob.extend_from_slice(&1u32.to_le_bytes());
        let mut volume = vec![0u8; VOLUME_NAME_LEN];
        volume[..9].copy_from_slice(b"DATA1.ALF");
        blob.extend(volume);
        blob.extend_from_slice(&2u32.to_le_bytes());
        for name in ["A.BIN", "B.AGF"] {
            let mut entry = vec![0u8; ENTRY_LEN];
            entry[..name.len()].copy_from_slice(name.as_bytes());
            blob.extend(entry);
        }
        blob.extend_from_slice(&[0u8; 12]); // metadata block stand-in
        blob.extend_from_slice(b"CREATEOBJECT\x002\x00SCREENX\x001280\x00SCREENY\x00720\x00");
        let compressed = alf::lzss_store(&blob);
        let mut data = b"S4IC443 ".to_vec();
        data.resize(0x12c, 0);
        for value in [blob.len() as u32, blob.len() as u32, compressed.len() as u32] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend(compressed);
        data
    }

    #[test]
    fn reads_identity_and_settings() {
        let catalog = read_catalog(&synthetic_master()).unwrap();
        assert_eq!((catalog.magic.as_str(), catalog.build.as_str(), catalog.entry_count), ("S4IC", "443", 2));
        assert_eq!(catalog.screen_size(), (1280, 720));
        assert_eq!(catalog.setting("CREATEOBJECT"), Some("2"));
        assert_eq!(catalog.generation(), 4);
    }

    #[test]
    fn extend_appends_records_and_keeps_settings() {
        let original = synthetic_master();
        let (extended, first_id) = extend_catalog(&original, &[NewRecord { name: "FPBANNER.AGF", length: 99 }]).unwrap();
        assert_eq!(first_id, 2);
        let index = alf::read_sys4_master(&extended).unwrap();
        assert_eq!(index.entries.len(), 3);
        assert_eq!(index.entries[2].name, "FPBANNER.AGF");
        assert_eq!(index.entries[2].length, 99);
        assert_eq!(read_catalog(&extended).unwrap().screen_size(), (1280, 720));
    }

    #[test]
    fn set_settings_replaces_and_appends() {
        let original = synthetic_master();
        let patched = set_catalog_settings(&original, &[("SCREENX", "1600"), ("LANGUAGE", "1")]).unwrap();
        let catalog = read_catalog(&patched).unwrap();
        assert_eq!(catalog.screen_size(), (1600, 720));
        assert_eq!(catalog.setting("LANGUAGE"), Some("1"));
        assert_eq!(catalog.setting("CREATEOBJECT"), Some("2"));
        assert_eq!(alf::read_sys4_master(&patched).unwrap().entries.len(), 2);
    }
}
