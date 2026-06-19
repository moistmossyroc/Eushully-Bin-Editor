//! Minimal SerializedFile (v22) surgery: read and replace a TextAsset's
//! `m_Script` without touching the type tree.
//!
//! We rely on two facts confirmed for these bundles:
//!   * header (big-endian i64s): metadataSize@16, fileSize@24, dataOffset@32
//!   * the object info table (little-endian) is `i32 count` then
//!     `[i64 pathID, i64 byteStart, u32 byteSize, i32 typeID]` records, located
//!     by scanning the metadata region (it's the only place whose first record
//!     has byteStart 0 and whose records exactly tile the data section).
//!
//! The TextAsset is the first object (byteStart 0); its data is
//! `[i32 nameLen][name][pad4][i32 scriptLen][script]`. Replacing the script
//! shifts later objects, so we patch every later object's `byteStart`, the
//! TextAsset's `byteSize`, and the header `fileSize`.

use encoding_rs::{SHIFT_JIS, UTF_16LE};

/// Encoding of a TextAsset's `m_Script`. Parente mixes all three: most scenes
/// are UTF-8 (no BOM), ~17% are UTF-16LE (with a `FF FE` BOM), and a handful of
/// system/init files are Shift-JIS. We must preserve whichever the file used on
/// write or the engine reads garbage.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ScriptEncoding {
    Utf8,
    Utf16Le,
    ShiftJis,
}

fn decode_script(bytes: &[u8]) -> (String, ScriptEncoding) {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let (cow, _) = UTF_16LE.decode_without_bom_handling(&bytes[2..]);
        (cow.into_owned(), ScriptEncoding::Utf16Le)
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        (s.to_owned(), ScriptEncoding::Utf8)
    } else {
        // Not valid UTF-8 and no UTF-16 BOM → legacy Shift-JIS.
        let (cow, _) = SHIFT_JIS.decode_without_bom_handling(bytes);
        (cow.into_owned(), ScriptEncoding::ShiftJis)
    }
}

fn encode_script(text: &str, enc: ScriptEncoding) -> Vec<u8> {
    match enc {
        ScriptEncoding::Utf8 => text.as_bytes().to_vec(),
        ScriptEncoding::Utf16Le => {
            let mut out = vec![0xFF, 0xFE];
            for u in text.encode_utf16() {
                out.extend_from_slice(&u.to_le_bytes());
            }
            out
        }
        ScriptEncoding::ShiftJis => SHIFT_JIS.encode(text).0.into_owned(),
    }
}

fn align4(x: usize) -> usize {
    (x + 3) & !3
}
fn align16(x: usize) -> usize {
    (x + 15) & !15
}

fn rd_i32_le(d: &[u8], p: usize) -> Result<i32, String> {
    d.get(p..p + 4)
        .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| "serialized: truncated i32".to_string())
}
fn rd_i64_be(d: &[u8], p: usize) -> Result<i64, String> {
    d.get(p..p + 8)
        .map(|b| i64::from_be_bytes(b.try_into().unwrap()))
        .ok_or_else(|| "serialized: truncated i64".to_string())
}

#[derive(Clone)]
struct ObjInfo {
    byte_start: i64,
    byte_size: u32,
    /// Byte position of this record inside the blob (for in-place patching).
    rec_pos: usize,
}

fn data_offset(blob: &[u8]) -> Result<usize, String> {
    Ok(rd_i64_be(blob, 32)? as usize)
}

/// Locate the object info table by validation scan.
fn find_object_table(blob: &[u8], data_offset: usize) -> Result<Vec<ObjInfo>, String> {
    let data_total = blob.len().checked_sub(data_offset).ok_or("bad dataOffset")?;
    let limit = data_offset.saturating_sub(4);
    for p in 64..limit {
        let cnt = rd_i32_le(blob, p)?;
        if cnt < 1 || cnt > 8 {
            continue;
        }
        let mut objs = Vec::with_capacity(cnt as usize);
        let mut q = p + 4;
        let mut ok = true;
        for _ in 0..cnt {
            if q + 24 > blob.len() {
                ok = false;
                break;
            }
            let byte_start = i64::from_le_bytes(blob[q + 8..q + 16].try_into().unwrap());
            let byte_size = u32::from_le_bytes(blob[q + 16..q + 20].try_into().unwrap());
            let type_id = i32::from_le_bytes(blob[q + 20..q + 24].try_into().unwrap());
            if byte_start < 0
                || (byte_start as usize + byte_size as usize) > data_total
                || type_id < 0
                || type_id > 7
            {
                ok = false;
                break;
            }
            objs.push(ObjInfo { byte_start, byte_size, rec_pos: q });
            q += 24;
        }
        if !ok || objs.is_empty() || objs[0].byte_start != 0 {
            continue;
        }
        let max_end = objs
            .iter()
            .map(|o| o.byte_start as usize + o.byte_size as usize)
            .max()
            .unwrap_or(0);
        if max_end + 16 < data_total {
            continue;
        }
        return Ok(objs);
    }
    Err("serialized: object table not found".to_string())
}

/// A located TextAsset object within the data section.
struct TextAssetLoc {
    /// Index into the object table.
    obj_index: usize,
    /// Absolute offset of the `m_Script` length prefix.
    script_pos: usize,
    /// Length of the script bytes.
    script_len: usize,
}

/// Try to interpret an object's data as a TextAsset (`[i32 nameLen][name]
/// [pad4][i32 scriptLen][script]`). Returns the script position/length if it
/// validates. Used to find the TextAsset regardless of object ordering — the
/// AssetBundle object can come first or second.
fn try_parse_textasset(blob: &[u8], base: usize, byte_size: usize) -> Option<(usize, usize)> {
    let nl = rd_i32_le(blob, base).ok()?;
    if nl <= 0 || nl > 128 {
        return None;
    }
    let name = blob.get(base + 4..base + 4 + nl as usize)?;
    // TextAsset names are ASCII (e.g. "SC0001", "$1$SC0601").
    if !name.iter().all(|&c| (0x20..0x7f).contains(&c)) {
        return None;
    }
    let script_pos = align4(base + 4 + nl as usize);
    if script_pos + 4 > base + byte_size {
        return None;
    }
    let sl = rd_i32_le(blob, script_pos).ok()?;
    if sl < 0 || script_pos + 4 + sl as usize > base + byte_size {
        return None;
    }
    Some((script_pos, sl as usize))
}

/// Find the TextAsset object — the one whose data parses as a TextAsset with
/// the largest script (the AssetBundle object's fields can superficially parse
/// but its "script" is tiny).
fn locate_textasset(blob: &[u8], dofs: usize, objs: &[ObjInfo]) -> Result<TextAssetLoc, String> {
    let mut best: Option<TextAssetLoc> = None;
    for (i, o) in objs.iter().enumerate() {
        let base = dofs + o.byte_start as usize;
        if let Some((script_pos, script_len)) = try_parse_textasset(blob, base, o.byte_size as usize) {
            if best.as_ref().map_or(true, |b| script_len > b.script_len) {
                best = Some(TextAssetLoc { obj_index: i, script_pos, script_len });
            }
        }
    }
    best.ok_or_else(|| "serialized: no TextAsset object found".to_string())
}

/// Read the TextAsset's script text and its original encoding.
pub fn read_textasset(blob: &[u8]) -> Result<(String, ScriptEncoding), String> {
    let dofs = data_offset(blob)?;
    let objs = find_object_table(blob, dofs)?;
    let ta = locate_textasset(blob, dofs, &objs)?;
    let bytes = blob
        .get(ta.script_pos + 4..ta.script_pos + 4 + ta.script_len)
        .ok_or("serialized: script out of range")?;
    Ok(decode_script(bytes))
}

/// Read the TextAsset's script text (encoding auto-detected).
pub fn read_textasset_text(blob: &[u8]) -> Result<String, String> {
    Ok(read_textasset(blob)?.0)
}

/// Produce a new SerializedFile blob with the TextAsset script replaced,
/// re-encoded in the file's original script encoding. Works regardless of
/// object order (the TextAsset may be object 0 or a later object).
pub fn patch_textasset(blob: &[u8], new_text: &str, enc: ScriptEncoding) -> Result<Vec<u8>, String> {
    let new_bytes = encode_script(new_text, enc);
    let dofs = data_offset(blob)?;
    let objs = find_object_table(blob, dofs)?;
    let ta = locate_textasset(blob, dofs, &objs)?;

    // Rebuild the TextAsset object: m_Name (aligned) + m_Script (aligned). Name
    // bytes precede the script-length prefix we located.
    let ta_obj = &objs[ta.obj_index];
    let base = dofs + ta_obj.byte_start as usize;
    let name = blob
        .get(base..ta.script_pos)
        .ok_or("serialized: name out of range")?
        .to_vec();
    let mut new_obj: Vec<u8> = name; // already includes nameLen + name + pad4
    new_obj.extend_from_slice(&(new_bytes.len() as i32).to_le_bytes());
    new_obj.extend_from_slice(&new_bytes);
    while new_obj.len() % 4 != 0 {
        new_obj.push(0);
    }

    // Reassemble the data section: every object in byteStart order, each placed
    // at a 16-byte boundary (matches Unity's layout and reproduces the original
    // offsets when nothing changed). The TextAsset gets its rebuilt bytes; all
    // others are copied verbatim.
    let mut data: Vec<u8> = Vec::new();
    let mut new_starts: Vec<(usize, usize)> = Vec::new(); // (rec_pos, new_byte_start)
    let mut new_ta_size = 0usize;
    for (i, o) in objs.iter().enumerate() {
        let start = align16(data.len());
        while data.len() < start {
            data.push(0);
        }
        if i == ta.obj_index {
            new_ta_size = new_obj.len();
            data.extend_from_slice(&new_obj);
        } else {
            let src = blob
                .get(dofs + o.byte_start as usize..dofs + o.byte_start as usize + o.byte_size as usize)
                .ok_or("serialized: object out of range")?;
            data.extend_from_slice(src);
        }
        new_starts.push((o.rec_pos, start));
    }

    // Metadata (unchanged) + new data section.
    let mut out: Vec<u8> = blob[..dofs].to_vec();
    out.extend_from_slice(&data);

    // Patch object table: every object's byteStart + the TextAsset's byteSize.
    for (rec_pos, new_start) in new_starts {
        out[rec_pos + 8..rec_pos + 16].copy_from_slice(&(new_start as i64).to_le_bytes());
    }
    out[ta_obj.rec_pos + 16..ta_obj.rec_pos + 20].copy_from_slice(&(new_ta_size as u32).to_le_bytes());
    // Patch header fileSize (i64 BE @24).
    let total = out.len() as i64;
    out[24..32].copy_from_slice(&total.to_be_bytes());

    Ok(out)
}
