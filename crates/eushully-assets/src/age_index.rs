// age_index.rs — Pure parsers for AGE index files (SYS3INI / SYS4INI / SYS5INI.BIN
// and per-archive *.AAI). No file system access: bytes in, entry table out.
// `archive.rs` resolves the archive names to paths on disk.
//
// Every index wraps the same table:
//   u32 archive_count
//   archive_count × name block        (SJIS 0x100 bytes | UTF-16LE 0x200 bytes)
//   u32 entry_count
//   entry_count × entry block         (0x50 | 0x90 bytes):
//       name (0x40 SJIS | 0x80 UTF-16LE), then u32 archive_index,
//       u32 (unused), u32 offset, u32 size
// Entries named "" or "@" are placeholders and are skipped, as are entries
// pointing at an archive index past the list.
//
// The table is either stored raw after the header, or as an LZSS sector whose
// compressed size sits just before the data (see `TableLocation`).

use encoding_rs::SHIFT_JIS;

use crate::lzss;

const LZSS_INITIAL_FILL: u8 = 0x00;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameEncoding {
    ShiftJis,
    Utf16Le,
}

/// Sizes of one index generation's table records.
#[derive(Debug)]
pub struct EntryTableLayout {
    pub archive_name_block_len: usize,
    pub entry_block_len: usize,
    /// Name field at the start of each entry block; the numeric fields follow it.
    pub entry_name_len: usize,
    pub name_encoding: NameEncoding,
    /// Sanity limits so garbage input fails fast instead of allocating wildly.
    pub max_archive_count: usize,
    pub max_entry_count: usize,
}

pub const SYS3_SYS4_LAYOUT: EntryTableLayout = EntryTableLayout {
    archive_name_block_len: 0x100,
    entry_block_len: 0x50,
    entry_name_len: 0x40,
    name_encoding: NameEncoding::ShiftJis,
    max_archive_count: 512,
    max_entry_count: 1 << 20,
};

pub const SYS5_LAYOUT: EntryTableLayout = EntryTableLayout {
    archive_name_block_len: 0x200,
    entry_block_len: 0x90,
    entry_name_len: 0x80,
    name_encoding: NameEncoding::Utf16Le,
    max_archive_count: 512,
    max_entry_count: 1 << 20,
};

/// AAI files index one ALF; a raw (magic-less) AAI is guessed at, so the
/// limits are tighter to reject non-AAI data.
pub const AAI_LAYOUT: EntryTableLayout = EntryTableLayout {
    max_archive_count: 64,
    max_entry_count: 65_536,
    ..SYS3_SYS4_LAYOUT
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableEntry {
    pub name: String,
    pub archive_index: usize,
    pub offset: u64,
    pub size: u64,
}

#[derive(Debug)]
pub struct EntryTable {
    /// Archive (ALF) names as stored, in index order.
    pub archive_names: Vec<String>,
    pub entries: Vec<TableEntry>,
}

/// Where the table lives inside the index file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableLocation {
    /// u32 compressed size at this offset, LZSS data right after it.
    Lzss { compressed_size_offset: usize },
    /// Table stored uncompressed from this offset to the end of the file.
    Raw { start: usize },
}

#[derive(Debug)]
pub struct IndexFormat {
    pub version: &'static str,
    pub location: TableLocation,
    pub layout: &'static EntryTableLayout,
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub fn decode_sjis_nullterm(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    SHIFT_JIS.decode(&bytes[..end]).0.into_owned()
}

pub fn decode_utf16_le_nullterm(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|&unit| unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn decode_name(bytes: &[u8], encoding: NameEncoding) -> String {
    match encoding {
        NameEncoding::ShiftJis => decode_sjis_nullterm(bytes),
        NameEncoding::Utf16Le => decode_utf16_le_nullterm(bytes),
    }
}

/// SYS3/SYS4 compressed magics (shared by SYS*INI.BIN and AAI files).
fn sys3_sys4_compressed_format(magic: &[u8]) -> Option<(&'static str, TableLocation)> {
    let (version, compressed_size_offset) = match magic {
        b"S4IC" => ("SYS4 Compressed (S4IC)", 0x134),
        b"S4AC" => ("SYS4 Append (S4AC)", 0x114),
        b"S3IC" => ("SYS3 Compressed (S3IC)", 0x134),
        _ => return None,
    };
    Some((version, TableLocation::Lzss { compressed_size_offset }))
}

/// SYS5 magics are UTF-16LE ("S\05\0I\0C\0"); plain ASCII is accepted too.
fn sys5_format(raw: &[u8]) -> Option<(&'static str, TableLocation)> {
    let utf16_magic = raw.get(0..8).map(decode_utf16_le_nullterm);
    let magic = match (raw.get(0..4), utf16_magic.as_deref()) {
        (Some(ascii), _) if ascii.starts_with(b"S5") => std::str::from_utf8(ascii).ok()?,
        (_, Some(wide)) if wide.starts_with("S5") => wide,
        _ => return None,
    };
    Some(match magic {
        "S5IC" => ("SYS5 Compressed (S5IC)", TableLocation::Lzss { compressed_size_offset: 0x224 }),
        "S5AC" => ("SYS5 Append (S5AC)", TableLocation::Lzss { compressed_size_offset: 0x21C }),
        _ => ("SYS5 Raw (S5IN)", TableLocation::Raw { start: 0x200 }),
    })
}

/// Identify a SYS*INI.BIN by its magic.
pub fn classify_index(raw: &[u8]) -> Result<IndexFormat, String> {
    let magic = raw.get(0..4).ok_or("index file is too small to be valid")?;
    if let Some((version, location)) = sys3_sys4_compressed_format(magic) {
        return Ok(IndexFormat { version, location, layout: &SYS3_SYS4_LAYOUT });
    }
    match magic {
        b"S4IN" => return Ok(IndexFormat { version: "SYS4 Raw (S4IN)", location: TableLocation::Raw { start: 4 }, layout: &SYS3_SYS4_LAYOUT }),
        b"S3IN" => return Ok(IndexFormat { version: "SYS3 (S3IN)", location: TableLocation::Raw { start: 0x12C }, layout: &SYS3_SYS4_LAYOUT }),
        _ => {}
    }
    if let Some((version, location)) = sys5_format(raw) {
        return Ok(IndexFormat { version, location, layout: &SYS5_LAYOUT });
    }
    Err(format!("unknown index format (magic {:02X?})", magic))
}

/// The table bytes, decompressed if needed.
pub fn read_table_bytes(raw: &[u8], location: TableLocation) -> Result<Vec<u8>, String> {
    match location {
        TableLocation::Raw { start } => raw
            .get(start..)
            .filter(|table| !table.is_empty())
            .map(<[u8]>::to_vec)
            .ok_or_else(|| format!("index too short: table starts at 0x{start:X}, file is {} bytes", raw.len())),
        TableLocation::Lzss { compressed_size_offset } => {
            let compressed_size = read_u32(raw, compressed_size_offset)
                .ok_or_else(|| format!("index header truncated (need 0x{:X} bytes)", compressed_size_offset + 4))?
                as usize;
            let data_start = compressed_size_offset + 4;
            let compressed = data_start
                .checked_add(compressed_size)
                .and_then(|end| raw.get(data_start..end))
                .ok_or_else(|| format!(
                    "compressed table truncated: need {compressed_size} bytes at 0x{data_start:X}, file has {}",
                    raw.len()
                ))?;
            lzss::decompress(compressed, LZSS_INITIAL_FILL).map_err(|error| format!("LZSS decompression failed: {error}"))
        }
    }
}

/// Parse a decompressed entry table.
pub fn parse_entry_table(data: &[u8], layout: &EntryTableLayout) -> Result<EntryTable, String> {
    let archive_count = read_u32(data, 0).ok_or("table too small for archive count")? as usize;
    if archive_count == 0 || archive_count > layout.max_archive_count {
        return Err(format!("suspicious archive count {archive_count}"));
    }
    let names_start = 4;
    let names_end = archive_count
        .checked_mul(layout.archive_name_block_len)
        .and_then(|len| len.checked_add(names_start))
        .filter(|&end| end <= data.len())
        .ok_or("table truncated in archive name list")?;
    let archive_names: Vec<String> = data[names_start..names_end]
        .chunks_exact(layout.archive_name_block_len)
        .map(|block| decode_name(block, layout.name_encoding))
        .collect();

    let entry_count = read_u32(data, names_end).ok_or("table truncated before entry count")? as usize;
    if entry_count > layout.max_entry_count {
        return Err(format!("suspicious entry count {entry_count}"));
    }
    let entries_start = names_end + 4;
    let entries_end = entry_count
        .checked_mul(layout.entry_block_len)
        .and_then(|len| len.checked_add(entries_start))
        .ok_or("entry table size overflows")?;
    let entry_bytes = data.get(entries_start..entries_end).ok_or_else(|| {
        format!("table truncated in entry list: need {entries_end} bytes, have {}", data.len())
    })?;

    let name_len = layout.entry_name_len;
    let entries = entry_bytes
        .chunks_exact(layout.entry_block_len)
        .filter_map(|block| {
            let name = decode_name(&block[..name_len], layout.name_encoding);
            let archive_index = read_u32(block, name_len)? as usize;
            if name.is_empty() || name == "@" || archive_index >= archive_count {
                return None;
            }
            Some(TableEntry {
                name,
                archive_index,
                offset: read_u32(block, name_len + 8)? as u64,
                size: read_u32(block, name_len + 12)? as u64,
            })
        })
        .collect();

    Ok(EntryTable { archive_names, entries })
}

/// SYS*INI.BIN bytes → (version label, table).
pub fn parse_index_bytes(raw: &[u8]) -> Result<(&'static str, EntryTable), String> {
    let format = classify_index(raw)?;
    let table_bytes = read_table_bytes(raw, format.location)?;
    let table = parse_entry_table(&table_bytes, format.layout).map_err(|error| format!("{}: {error}", format.version))?;
    Ok((format.version, table))
}

/// *.AAI bytes → table. AAIs carry SYS3/SYS4 compressed magics, or are a bare table.
pub fn parse_aai_bytes(raw: &[u8]) -> Result<EntryTable, String> {
    let location = raw
        .get(0..4)
        .and_then(sys3_sys4_compressed_format)
        .map_or(TableLocation::Raw { start: 0 }, |(_, location)| location);
    parse_entry_table(&read_table_bytes(raw, location)?, &AAI_LAYOUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_name(name: &str, layout: &EntryTableLayout, block_len: usize) -> Vec<u8> {
        let mut bytes: Vec<u8> = match layout.name_encoding {
            NameEncoding::ShiftJis => name.as_bytes().to_vec(),
            NameEncoding::Utf16Le => name.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        };
        bytes.resize(block_len, 0);
        bytes
    }

    fn build_table(layout: &EntryTableLayout, archives: &[&str], entries: &[(&str, u32, u32, u32)]) -> Vec<u8> {
        let mut table = (archives.len() as u32).to_le_bytes().to_vec();
        for archive in archives {
            table.extend(encode_name(archive, layout, layout.archive_name_block_len));
        }
        table.extend((entries.len() as u32).to_le_bytes());
        for &(name, archive_index, offset, size) in entries {
            let mut block = encode_name(name, layout, layout.entry_name_len);
            for value in [archive_index, 0, offset, size] {
                block.extend(value.to_le_bytes());
            }
            block.resize(layout.entry_block_len, 0);
            table.extend(block);
        }
        table
    }

    /// LZSS stream of literals only (flag byte 0xFF per 8 bytes).
    fn lzss_literals(bytes: &[u8]) -> Vec<u8> {
        bytes.chunks(8).flat_map(|chunk| std::iter::once(0xFF).chain(chunk.iter().copied())).collect()
    }

    const SAMPLE_ENTRIES: [(&str, u32, u32, u32); 4] =
        [("A.AGF", 0, 0, 10), ("@", 0, 0, 0), ("B.WAV", 1, 10, 20), ("BAD.AGF", 7, 0, 1)];

    #[test]
    fn parses_both_layouts_and_skips_placeholders() {
        for layout in [&SYS3_SYS4_LAYOUT, &SYS5_LAYOUT] {
            let table = parse_entry_table(&build_table(layout, &["DATA1.ALF", "DATA2.ALF"], &SAMPLE_ENTRIES), layout).unwrap();
            assert_eq!(table.archive_names, ["DATA1.ALF", "DATA2.ALF"]);
            assert_eq!(
                table.entries,
                [
                    TableEntry { name: "A.AGF".into(), archive_index: 0, offset: 0, size: 10 },
                    TableEntry { name: "B.WAV".into(), archive_index: 1, offset: 10, size: 20 },
                ]
            );
        }
    }

    #[test]
    fn compressed_sys4_and_sys5_indexes_round_trip() {
        let sys4_table = build_table(&SYS3_SYS4_LAYOUT, &["DATA.ALF"], &SAMPLE_ENTRIES[..1]);
        let mut sys4 = b"S4IC".to_vec();
        sys4.resize(0x134, 0);
        let compressed = lzss_literals(&sys4_table);
        sys4.extend((compressed.len() as u32).to_le_bytes());
        sys4.extend(compressed);
        let (version, table) = parse_index_bytes(&sys4).unwrap();
        assert_eq!(version, "SYS4 Compressed (S4IC)");
        assert_eq!(table.entries.len(), 1);

        let sys5_table = build_table(&SYS5_LAYOUT, &["DATA.ALF"], &SAMPLE_ENTRIES[..1]);
        let mut sys5: Vec<u8> = "S5IC".encode_utf16().flat_map(u16::to_le_bytes).collect();
        sys5.resize(0x224, 0);
        let compressed = lzss_literals(&sys5_table);
        sys5.extend((compressed.len() as u32).to_le_bytes());
        sys5.extend(compressed);
        let (version, table) = parse_index_bytes(&sys5).unwrap();
        assert_eq!(version, "SYS5 Compressed (S5IC)");
        assert_eq!(table.entries[0].name, "A.AGF");
    }

    #[test]
    fn raw_aai_table_parses() {
        let table = parse_aai_bytes(&build_table(&AAI_LAYOUT, &["X.ALF"], &SAMPLE_ENTRIES[..1])).unwrap();
        assert_eq!(table.entries.len(), 1);
    }

    #[test]
    fn hostile_counts_and_truncation_are_errors() {
        let mut huge_entry_count = build_table(&SYS3_SYS4_LAYOUT, &["A.ALF"], &[]);
        let count_at = 4 + 0x100;
        huge_entry_count[count_at..count_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_entry_table(&huge_entry_count, &SYS3_SYS4_LAYOUT).is_err());

        let mut truncated = build_table(&SYS5_LAYOUT, &["A.ALF"], &SAMPLE_ENTRIES);
        truncated.truncate(truncated.len() - 1);
        assert!(parse_entry_table(&truncated, &SYS5_LAYOUT).is_err());

        assert!(parse_entry_table(&[0, 0, 0, 0], &SYS3_SYS4_LAYOUT).is_err());
        assert!(parse_index_bytes(b"S4IC").is_err());
        assert!(parse_index_bytes(b"XXXXXXXX").is_err());
        let mut lying_size = b"S4IC".to_vec();
        lying_size.resize(0x134, 0);
        lying_size.extend(u32::MAX.to_le_bytes());
        assert!(parse_index_bytes(&lying_size).is_err());
    }
}
