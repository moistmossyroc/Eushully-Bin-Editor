//! UnityFS bundle reader/writer, scoped to the single-SerializedFile bundles
//! shipped by Yamiyo no Parente (fmt v8, Unity 6000.3.x, LZ4HC blocks).
//!
//! Header (all multi-byte ints BIG-ENDIAN):
//! ```text
//!   "UnityFS\0"
//!   i32 format (=8)
//!   cstr minVersion ("5.x.x")
//!   cstr unityVersion ("6000.3.9f1")
//!   i64 size                (total bundle file size)
//!   i32 compressedBlocksInfoSize
//!   i32 uncompressedBlocksInfoSize
//!   i32 flags
//!   [pad to 16 if flags & 0x200]
//!   blocksInfo (compressed per flags & 0x3f)
//!   [pad to 16 if flags & 0x200]
//!   data blocks (each compressed per its own flags)
//! ```
//! blocksInfo: 16-byte hash, i32 blockCount, [u32 uSize, u32 cSize, u16 flags]*,
//! i32 nodeCount, [i64 offset, i64 size, u32 flags, cstr path]*.
//!
//! On write we store the (single) data block and the blocksInfo UNCOMPRESSED
//! (compression type 0) — Unity reads those fine and it removes any need for an
//! LZ4 *compressor*. We only ever LZ4-*decompress* on read.

const COMP_MASK: i32 = 0x3f;
const FLAG_PADDING: i32 = 0x200;

fn align16(x: usize) -> usize {
    (x + 15) & !15
}

struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self
            .d
            .get(self.pos..self.pos + n)
            .ok_or("bundle truncated")?;
        self.pos += n;
        Ok(s)
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn cstr(&mut self) -> Result<String, String> {
        let start = self.pos;
        while *self.d.get(self.pos).ok_or("bundle truncated (cstr)")? != 0 {
            self.pos += 1;
        }
        let s = String::from_utf8_lossy(&self.d[start..self.pos]).into_owned();
        self.pos += 1; // skip NUL
        Ok(s)
    }
}

/// A bundle's directory node. A bundle may have several (e.g. a prefab's
/// SerializedFile + its `.resS` texture stream), all of which must be preserved.
#[derive(Clone)]
pub struct BundleNode {
    pub offset: i64,
    pub size: i64,
    pub flags: u32,
    pub path: String,
}

/// Parsed bundle: header info to round-trip + the decompressed data (`blob`,
/// the concatenation of all blocks; node offsets index into it).
pub struct Bundle {
    pub format: i32,
    pub min_version: String,
    pub unity_version: String,
    pub flags: i32,
    pub nodes: Vec<BundleNode>,
    pub blob: Vec<u8>,
}

fn lz4_decompress(input: &[u8], out_size: usize) -> Result<Vec<u8>, String> {
    lz4_flex::block::decompress(input, out_size)
        .map_err(|e| format!("LZ4 decompress failed: {}", e))
}

/// Parse a UnityFS bundle and return its decompressed SerializedFile blob.
pub fn unpack_bundle(d: &[u8]) -> Result<Bundle, String> {
    let mut r = Reader::new(d);
    if r.take(8)? != b"UnityFS\0" {
        return Err("not a UnityFS bundle".to_string());
    }
    let format = r.i32()?;
    let min_version = r.cstr()?;
    let unity_version = r.cstr()?;
    let _size = r.i64()?;
    let ci = r.i32()? as usize;
    let ui = r.i32()? as usize;
    let flags = r.i32()?;

    if flags & FLAG_PADDING != 0 {
        r.pos = align16(r.pos);
    }
    let bi_raw = r.take(ci)?;
    let bi = match flags & COMP_MASK {
        0 => bi_raw.to_vec(),
        2 | 3 => lz4_decompress(bi_raw, ui)?,
        other => return Err(format!("unsupported blocksInfo compression {}", other)),
    };

    // Parse blocksInfo.
    let mut br = Reader::new(&bi);
    br.pos = 16; // skip hash
    let block_count = br.i32()?;
    let mut blocks: Vec<(usize, usize, u16)> = Vec::new();
    for _ in 0..block_count {
        let u = br.i32()? as usize;
        let c = br.i32()? as usize;
        let bflags = u16::from_be_bytes(br.take(2)?.try_into().unwrap());
        blocks.push((u, c, bflags));
    }
    let node_count = br.i32()?;
    if node_count < 1 {
        return Err("bundle has no directory nodes".to_string());
    }
    // Preserve ALL nodes (SerializedFile, .resS streams, etc.).
    let mut nodes: Vec<BundleNode> = Vec::with_capacity(node_count as usize);
    for _ in 0..node_count {
        let offset = br.i64()?;
        let size = br.i64()?;
        let nflags = u32::from_be_bytes(br.take(4)?.try_into().unwrap());
        let path = br.cstr()?;
        nodes.push(BundleNode { offset, size, flags: nflags, path });
    }

    // Data blocks follow (padded to 16 if the flag is set).
    if flags & FLAG_PADDING != 0 {
        r.pos = align16(r.pos);
    }
    let mut blob: Vec<u8> = Vec::new();
    for (u, c, bflags) in &blocks {
        let chunk = r.take(*c)?;
        match (*bflags as i32) & COMP_MASK {
            0 => blob.extend_from_slice(chunk),
            2 | 3 => blob.extend_from_slice(&lz4_decompress(chunk, *u)?),
            other => return Err(format!("unsupported block compression {}", other)),
        }
    }

    Ok(Bundle {
        format,
        min_version,
        unity_version,
        flags,
        nodes,
        blob,
    })
}

/// Re-serialize a bundle with a replacement data blob, storing it as a single
/// uncompressed block and preserving the directory nodes.
///
/// - Single-node bundles (scenes): the node is resized to the new blob length
///   (a TextAsset edit changes the SerializedFile size).
/// - Multi-node bundles (prefabs with `.resS`): nodes are preserved verbatim,
///   which requires the blob length to be UNCHANGED (only in-place edits).
pub fn pack_bundle(b: &Bundle, new_blob: &[u8]) -> Vec<u8> {
    // Build the directory node table.
    let mut node_bytes: Vec<u8> = Vec::new();
    if b.nodes.len() == 1 {
        let n = &b.nodes[0];
        node_bytes.extend_from_slice(&0i64.to_be_bytes()); // offset
        node_bytes.extend_from_slice(&(new_blob.len() as i64).to_be_bytes()); // resized
        node_bytes.extend_from_slice(&n.flags.to_be_bytes());
        node_bytes.extend_from_slice(n.path.as_bytes());
        node_bytes.push(0);
    } else {
        for n in &b.nodes {
            node_bytes.extend_from_slice(&n.offset.to_be_bytes());
            node_bytes.extend_from_slice(&n.size.to_be_bytes());
            node_bytes.extend_from_slice(&n.flags.to_be_bytes());
            node_bytes.extend_from_slice(n.path.as_bytes());
            node_bytes.push(0);
        }
    }

    // blocksInfo: zero hash, 1 uncompressed block, then the node table.
    let mut bi: Vec<u8> = Vec::new();
    bi.extend_from_slice(&[0u8; 16]);
    bi.extend_from_slice(&1i32.to_be_bytes()); // blockCount
    bi.extend_from_slice(&(new_blob.len() as u32).to_be_bytes()); // uSize
    bi.extend_from_slice(&(new_blob.len() as u32).to_be_bytes()); // cSize
    bi.extend_from_slice(&0u16.to_be_bytes()); // block flags = uncompressed
    bi.extend_from_slice(&(b.nodes.len() as i32).to_be_bytes()); // nodeCount
    bi.extend_from_slice(&node_bytes);

    let flags = b.flags & !COMP_MASK; // compression type 0 for blocksInfo

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"UnityFS\0");
    out.extend_from_slice(&b.format.to_be_bytes());
    out.extend_from_slice(b.min_version.as_bytes());
    out.push(0);
    out.extend_from_slice(b.unity_version.as_bytes());
    out.push(0);
    let size_pos = out.len();
    out.extend_from_slice(&0i64.to_be_bytes()); // size — patched below
    out.extend_from_slice(&(bi.len() as i32).to_be_bytes()); // ci
    out.extend_from_slice(&(bi.len() as i32).to_be_bytes()); // ui
    out.extend_from_slice(&flags.to_be_bytes());
    if flags & FLAG_PADDING != 0 {
        while out.len() % 16 != 0 {
            out.push(0);
        }
    }
    out.extend_from_slice(&bi);
    if flags & FLAG_PADDING != 0 {
        while out.len() % 16 != 0 {
            out.push(0);
        }
    }
    out.extend_from_slice(new_blob);

    let total = out.len() as i64;
    out[size_pos..size_pos + 8].copy_from_slice(&total.to_be_bytes());
    out
}
