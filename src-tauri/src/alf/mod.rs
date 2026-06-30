//! AGE `.alf` archive reader (SYS4; SYS5 to follow).
//!
//! Eushully packs the base game in `.alf` data archives indexed by a master
//! `SYS4INI.BIN` (magic `S4IC`/`S4IN`) or per-append `*.AAI` (`S4AC`/`S4AN`).
//! The compressed index (`*IC`/`*AC`) is LZSS-compressed; once inflated it is:
//!   u32 arc_count, arc_count × 0x200-byte archive-name blocks (SJIS),
//!   u32 entry_count, entry_count × 0x90-byte entries (SJIS name at start;
//!   at +0x80: u32 archive_index, file_index, offset, length).
//! Entry data lives in the named `DATA*.ALF`/`APPEND*.ALF` volumes (header
//! `SYS4424 `) at (offset, length).
//!
//! Ported from Eushully-Decompiler/extract_alf.py (which covers SYS5) + RE of a
//! real SYS4 SYS4INI.BIN. The LZSS is classic Okumura (4K window, 18-byte max
//! match, threshold 2) with the ring buffer pre-filled with 0x00.

/// Classic LZSS decompression (Okumura variant, initial ring-buffer value 0).
pub fn lzss_decompress(input: &[u8], out_size: usize) -> Vec<u8> {
    const N: usize = 4096;
    const F: usize = 18;
    const THRESHOLD: usize = 2;

    // Cap the pre-allocation — a misread out_size must never trigger a multi-GB
    // allocation. The loop is input-bounded, so this only limits the upfront
    // reserve; legitimate indexes are a few MB.
    let mut out = Vec::with_capacity(out_size.min(16 * 1024 * 1024));
    let mut window = vec![0u8; N];
    let mut r = N - F;
    let mut ip = 0usize;
    let mut flags: u32 = 0;

    while ip < input.len() && out.len() < out_size {
        flags >>= 1;
        if flags & 0x100 == 0 {
            if ip >= input.len() {
                break;
            }
            flags = input[ip] as u32 | 0xff00; // high byte = bit counter
            ip += 1;
        }
        if flags & 1 == 1 {
            // literal byte
            if ip >= input.len() {
                break;
            }
            let c = input[ip];
            ip += 1;
            out.push(c);
            window[r] = c;
            r = (r + 1) % N;
        } else {
            // back-reference: 12-bit position + 4-bit length
            if ip + 1 >= input.len() {
                break;
            }
            let b0 = input[ip] as usize;
            let b1 = input[ip + 1] as usize;
            ip += 2;
            let pos = b0 | ((b1 & 0xf0) << 4);
            let len = (b1 & 0x0f) + THRESHOLD + 1;
            for k in 0..len {
                let c = window[(pos + k) % N];
                out.push(c);
                window[r] = c;
                r = (r + 1) % N;
                if out.len() >= out_size {
                    break;
                }
            }
        }
    }
    out
}

fn read_u32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// Encode `data` as a valid LZSS stream using LITERALS ONLY (no back-references).
/// Not real compression (≈ +1/8 from flag bytes), but it produces a stream our
/// `lzss_decompress` — and the engine — read back exactly, which is enough to
/// WRITE a working index without a full match-finder. Index blobs are small, so
/// the size cost is negligible. Flag byte: each set LSB-first bit = a literal.
pub fn lzss_store(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 8 + 1);
    let mut i = 0;
    while i < data.len() {
        let n = (data.len() - i).min(8);
        out.push(((1u16 << n) - 1) as u8); // n low bits set → n literals follow
        out.extend_from_slice(&data[i..i + n]);
        i += n;
    }
    out
}

/// Locate the `(uncomp, uncomp2, comp)` LZSS triple in a SYS4 index header.
/// The header size varies per game (VERITA's is at 0x12c), so we scan for it:
/// `uncomp == uncomp2`, a sane uncompressed size, and a compressed size that is
/// the tail of the file (the LZSS stream). Avoids hardcoding a wrong offset →
/// garbage size → giant allocation hang.
fn find_sys4_triple(data: &[u8]) -> Option<usize> {
    let mut o = 8usize;
    while o + 12 <= data.len() && o < 0x400 {
        let a = read_u32(data, o) as usize;
        let b = read_u32(data, o + 4) as usize;
        let c = read_u32(data, o + 8) as usize;
        let remaining = data.len() - (o + 12);
        if a == b && a > 0 && a < 256 * 1024 * 1024 && c > 0 && c <= remaining && remaining - c < 0x40 {
            return Some(o);
        }
        o += 4;
    }
    None
}

/// Inflate the index blob from a compressed SYS4 index file (`S4IC`/`S4AC`).
/// The `(uncomp, uncomp2, comp)` triple sits at `triple_off`; LZSS data follows.
pub fn inflate_sys4_index(data: &[u8], triple_off: usize) -> Option<Vec<u8>> {
    if data.len() < triple_off + 12 {
        return None;
    }
    let uncomp = read_u32(data, triple_off) as usize;
    let comp = read_u32(data, triple_off + 8) as usize;
    let start = triple_off + 12;
    // Guard against a misread triple (wrong offset / non-SYS4 file): bail rather
    // than attempt a huge allocation or read past the buffer.
    if uncomp == 0 || uncomp > 256 * 1024 * 1024 || comp == 0 || start + comp > data.len() {
        return None;
    }
    Some(lzss_decompress(&data[start..start + comp], uncomp))
}

/// One file packed in a `.alf` data volume.
#[derive(Debug, Clone)]
pub struct AlfEntry {
    pub name: String,
    /// Index into `AlfIndex.archives` — which data volume holds the bytes.
    pub archive_index: u32,
    pub offset: u64,
    pub length: u64,
}

/// A parsed SYS4 archive index (master `SYS4INI.BIN` or append `*.AAI`).
#[derive(Debug, Clone)]
pub struct AlfIndex {
    /// Data volume filenames (e.g. `DATA1.ALF` … `DATA7.ALF`), `archive_index`-aligned.
    pub archives: Vec<String>,
    pub entries: Vec<AlfEntry>,
}

fn sjis_cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    encoding_rs::SHIFT_JIS.decode(&bytes[..end]).0.into_owned()
}

/// Parse a *decompressed* SYS4 index blob. Layout (RE'd from a real SYS4INI):
///   u32 arc_count; arc_count × 0x100-byte archive-name blocks (SJIS name @start);
///   u32 entry_count; entry_count × 0x50-byte entries —
///     name (SJIS, 0x40) then u32 archive_index, file_index, offset, length.
pub fn parse_sys4_index_blob(blob: &[u8]) -> Option<AlfIndex> {
    if blob.len() < 4 {
        return None;
    }
    let arc_count = read_u32(blob, 0) as usize;
    if arc_count == 0 || arc_count > 256 {
        return None;
    }
    let mut archives = Vec::with_capacity(arc_count);
    for i in 0..arc_count {
        let off = 4 + i * 0x100;
        if off + 0x100 > blob.len() {
            return None;
        }
        archives.push(sjis_cstr(&blob[off..off + 0x100]));
    }
    let ec_off = 4 + arc_count * 0x100;
    if ec_off + 4 > blob.len() {
        return None;
    }
    let entry_count = read_u32(blob, ec_off) as usize;
    let mut entries = Vec::with_capacity(entry_count.min(1 << 20));
    let mut p = ec_off + 4;
    for _ in 0..entry_count {
        if p + 0x50 > blob.len() {
            break;
        }
        entries.push(AlfEntry {
            name: sjis_cstr(&blob[p..p + 0x40]),
            archive_index: read_u32(blob, p + 0x40),
            offset: read_u32(blob, p + 0x48) as u64,
            length: read_u32(blob, p + 0x4c) as u64,
        });
        p += 0x50;
    }
    Some(AlfIndex { archives, entries })
}

/// Read + parse a SYS4 master index (`SYS4INI.BIN`). Handles `S4IC` (LZSS-
/// compressed) and `S4IN` (uncompressed). Returns the volume list + entries.
pub fn read_sys4_master(data: &[u8]) -> Option<AlfIndex> {
    if data.len() < 8 {
        return None;
    }
    match &data[..4] {
        // S4IC = compressed master, S4AC = compressed append (.aai) — same body.
        b"S4IC" | b"S4AC" => {
            let triple = find_sys4_triple(data)?;
            parse_sys4_index_blob(&inflate_sys4_index(data, triple)?)
        }
        // S4IN / S4AN = uncompressed (best-effort offset; not yet seen in the wild).
        b"S4IN" | b"S4AN" => parse_sys4_index_blob(&data[0x12c..]),
        _ => None,
    }
}

/// Build a SYS4 single-volume append: the `.alf` (translated BINs concatenated)
/// and the `.aai` index (`S4AC` compressed-via-store). `entries` is (name, bytes)
/// — names should carry the `$N$` layer prefix to override lower layers/base.
/// `version` is the 3-byte tag (e.g. `b"443"`); `title` is the SJIS header title
/// (copy the game's for authenticity). Returns (alf_bytes, aai_bytes).
pub fn build_sys4_append(
    arc_filename: &str,
    entries: &[(String, Vec<u8>)],
    version: [u8; 3],
    title: &[u8],
) -> (Vec<u8>, Vec<u8>) {
    // .alf = concatenated BIN data; record each (offset, length).
    let mut alf = Vec::new();
    let mut placed: Vec<(String, u32, u32)> = Vec::with_capacity(entries.len());
    for (name, data) in entries {
        let off = alf.len() as u32;
        alf.extend_from_slice(data);
        placed.push((name.clone(), off, data.len() as u32));
    }

    // Index blob: arc_count(1) + 0x100 archive-name block + entry_count + 0x50 entries.
    let mut blob = Vec::new();
    blob.extend_from_slice(&1u32.to_le_bytes());
    let mut arc_block = vec![0u8; 0x100];
    let arc_sjis = encoding_rs::SHIFT_JIS.encode(arc_filename).0;
    let an = arc_sjis.len().min(0xff);
    arc_block[..an].copy_from_slice(&arc_sjis[..an]);
    blob.extend_from_slice(&arc_block);
    blob.extend_from_slice(&(placed.len() as u32).to_le_bytes());
    for (name, off, len) in &placed {
        let mut e = vec![0u8; 0x50];
        let nm = encoding_rs::SHIFT_JIS.encode(name).0;
        let n = nm.len().min(0x3f);
        e[..n].copy_from_slice(&nm[..n]);
        e[0x40..0x44].copy_from_slice(&0u32.to_le_bytes()); // archive_index (single volume)
        e[0x44..0x48].copy_from_slice(&0u32.to_le_bytes()); // file_index
        e[0x48..0x4c].copy_from_slice(&off.to_le_bytes());
        e[0x4c..0x50].copy_from_slice(&len.to_le_bytes());
        blob.extend_from_slice(&e);
    }

    // .aai header: "S4AC<ver> " + title (to 0x100) + [0,0,1] + (uncomp,uncomp2,comp) + LZSS stream.
    let comp = lzss_store(&blob);
    let mut aai = Vec::with_capacity(0x118 + comp.len());
    aai.extend_from_slice(b"S4AC");
    aai.extend_from_slice(&version);
    aai.push(b' ');
    let mut title_region = vec![0u8; 0x100 - 8];
    let t = title.len().min(title_region.len() - 1);
    title_region[..t].copy_from_slice(&title[..t]);
    aai.extend_from_slice(&title_region); // now at 0x100
    aai.extend_from_slice(&0u32.to_le_bytes()); // 0x100
    aai.extend_from_slice(&0u32.to_le_bytes()); // 0x104
    aai.extend_from_slice(&1u32.to_le_bytes()); // 0x108
    aai.extend_from_slice(&(blob.len() as u32).to_le_bytes()); // 0x10c uncomp
    aai.extend_from_slice(&(blob.len() as u32).to_le_bytes()); // 0x110 uncomp2
    aai.extend_from_slice(&(comp.len() as u32).to_le_bytes()); // 0x114 comp
    aai.extend_from_slice(&comp); // 0x118 LZSS data
    (alf, aai)
}

/// Extract one entry's raw bytes from its data volume in `game_dir`.
pub fn extract_entry(
    game_dir: &std::path::Path,
    index: &AlfIndex,
    entry: &AlfEntry,
) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let arc = index
        .archives
        .get(entry.archive_index as usize)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "archive index out of range"))?;
    let mut f = std::fs::File::open(game_dir.join(arc))?;
    f.seek(SeekFrom::Start(entry.offset))?;
    let mut buf = vec![0u8; entry.length as usize];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The store-mode encoder must round-trip through our decoder (and thus the
    /// engine's), at full and partial flag-group boundaries.
    #[test]
    fn lzss_store_roundtrips() {
        for len in [0usize, 1, 7, 8, 9, 16, 17, 100, 4096, 5000] {
            let data: Vec<u8> = (0..len).map(|i| ((i * 13 + 7) % 251) as u8).collect();
            let dec = lzss_decompress(&lzss_store(&data), data.len());
            assert_eq!(dec, data, "len {len}");
        }
    }

    /// A built append must round-trip through our own reader: same archive name,
    /// entry names, and each entry's bytes recoverable at (offset, length).
    #[test]
    fn append_roundtrips_through_reader() {
        let bin_a: Vec<u8> = (0..1000u32).map(|i| (i % 256) as u8).collect();
        let bin_b: Vec<u8> = (0..50u32).map(|i| 255 - (i % 256) as u8).collect();
        let entries = vec![
            ("$2$SC0000.BIN".to_string(), bin_a.clone()),
            ("$2$SC0010.BIN".to_string(), bin_b.clone()),
        ];
        let (alf, aai) = build_sys4_append("APPEND02.ALF", &entries, *b"443", "テスト".as_bytes());

        let idx = read_sys4_master(&aai).expect("reader parses our .aai");
        assert_eq!(idx.archives, vec!["APPEND02.ALF".to_string()]);
        assert_eq!(idx.entries.len(), 2);
        assert_eq!(idx.entries[0].name, "$2$SC0000.BIN");
        assert_eq!(idx.entries[1].name, "$2$SC0010.BIN");
        for (entry, original) in idx.entries.iter().zip([&bin_a, &bin_b]) {
            let slice = &alf[entry.offset as usize..(entry.offset + entry.length) as usize];
            assert_eq!(slice, &original[..], "bytes for {}", entry.name);
        }
    }
}
