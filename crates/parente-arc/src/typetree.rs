//! Type-tree-driven SerializedFile (v22, little-endian metadata) reader.
//!
//! The Parente prefab bundles ship full type trees, so every object can be
//! walked field-by-field with exact byte spans — no heuristics. Used to edit
//! non-script assets (e.g. swap a `Font`'s embedded `m_FontData`).
//!
//! Layout references: Unity SerializedFile format as implemented by UnityPy /
//! AssetStudio (header 48 bytes for v22; type tree node = 32 bytes for v>=19;
//! node strings with the high bit set index Unity's built-in common-string
//! table, the subset of which we need is embedded below).

/// Unity's built-in common type-tree strings (offset → name). Only offsets that
/// appear in practice for the types we walk need to be present; an unknown
/// offset yields "<common:N>" and the walker falls back to byte sizes.
const COMMON_STRINGS: &[(u32, &str)] = &[
    (0, "AABB"), (5, "AnimationClip"), (19, "AnimationCurve"), (34, "AnimationState"), (49, "Array"),
    (55, "Base"), (60, "BitField"), (69, "bitset"), (76, "bool"), (81, "char"), (86, "ColorRGBA"),
    (96, "Component"), (106, "data"), (111, "deque"), (117, "double"), (124, "dynamic_array"),
    (138, "FastPropertyName"), (155, "first"), (161, "float"), (167, "Font"), (172, "GameObject"),
    (183, "Generic Mono"), (196, "GradientNEW"), (208, "GUID"), (213, "GUIStyle"), (222, "int"),
    (226, "list"), (231, "long long"), (241, "map"), (245, "Matrix4x4f"), (256, "MdFour"),
    (263, "MonoBehaviour"), (277, "MonoScript"), (288, "m_ByteSize"), (299, "m_Curve"),
    (307, "m_EditorClassIdentifier"), (331, "m_EditorHideFlags"), (349, "m_Enabled"),
    (359, "m_ExtensionPtr"), (374, "m_GameObject"), (387, "m_Index"), (395, "m_IsArray"),
    (405, "m_IsStatic"), (416, "m_MetaFlag"), (427, "m_Name"), (434, "m_ObjectHideFlags"),
    (452, "m_PrefabInternal"), (469, "m_PrefabParentObject"), (490, "m_Script"),
    (499, "m_StaticEditorFlags"), (519, "m_Type"), (526, "m_Version"), (536, "Object"), (543, "pair"),
    (548, "PPtr<Component>"), (564, "PPtr<GameObject>"), (581, "PPtr<Material>"),
    (598, "PPtr<MonoBehaviour>"), (618, "PPtr<MonoScript>"), (635, "PPtr<Object>"),
    (648, "PPtr<Prefab>"), (661, "PPtr<Sprite>"), (674, "PPtr<TextAsset>"), (690, "PPtr<Texture>"),
    (705, "PPtr<Texture2D>"), (721, "PPtr<Transform>"), (737, "Prefab"), (744, "Quaternionf"),
    (756, "Rectf"), (762, "RectInt"), (770, "RectOffset"), (781, "second"), (788, "set"),
    (792, "short"), (798, "size"), (803, "SInt16"), (810, "SInt32"), (817, "SInt64"), (824, "SInt8"),
    (830, "staticvector"), (843, "string"), (850, "TextAsset"), (860, "TextMesh"), (869, "Texture"),
    (877, "Texture2D"), (887, "Transform"), (897, "TypelessData"), (910, "UInt16"), (917, "UInt32"),
    (924, "UInt64"), (931, "UInt8"), (937, "unsigned int"), (950, "unsigned long long"),
    (969, "unsigned short"), (984, "vector"), (991, "Vector2f"), (1000, "Vector3f"),
    (1009, "Vector4f"), (1018, "m_ScriptingClassIdentifier"), (1045, "Gradient"), (1054, "Type*"),
    (1060, "int2_storage"), (1073, "int3_storage"), (1086, "BoundsInt"),
    (1096, "m_CorrespondingSourceObject"), (1124, "m_PrefabInstance"), (1141, "m_PrefabAsset"),
    (1155, "FileSize"), (1164, "Hash128"),
];

/// Class id of `MonoBehaviour` (carries a script hash in its type record).
const CLASS_MONOBEHAVIOUR: i32 = 114;
/// `m_MetaFlag` bit: align the stream to 4 bytes after this field.
const META_ALIGN: i32 = 0x4000;
/// `m_TypeFlags` bit: node is an array.
const TYPE_FLAG_ARRAY: u8 = 0x1;
const HEADER_LEN_V22: usize = 48;
const NODE_LEN_V19: usize = 32;

#[derive(Debug, Clone)]
pub struct TypeNode {
    pub level: u8,
    pub type_flags: u8,
    pub type_name: String,
    pub name: String,
    pub byte_size: i32,
    pub meta_flag: i32,
}

#[derive(Debug, Clone)]
pub struct SerializedType {
    pub class_id: i32,
    pub nodes: Vec<TypeNode>,
}

#[derive(Debug, Clone)]
pub struct ObjectInfo {
    pub path_id: i64,
    pub byte_start: u64,
    pub byte_size: u32,
    pub type_index: usize,
    /// Byte position of this object's record in the metadata (for patching).
    pub record_pos: usize,
}

#[derive(Debug, Clone)]
pub struct SerializedFile {
    pub version: u32,
    pub data_offset: usize,
    pub types: Vec<SerializedType>,
    pub objects: Vec<ObjectInfo>,
}

impl SerializedFile {
    pub fn class_id(&self, object: &ObjectInfo) -> i32 {
        self.types.get(object.type_index).map_or(-1, |t| t.class_id)
    }
}

/// A walked field: dotted path, type name, absolute byte span in the blob.
#[derive(Debug, Clone)]
pub struct FieldSpan {
    pub path: String,
    pub type_name: String,
    pub start: usize,
    pub end: usize,
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let slice = self.data.get(self.pos..self.pos + n).ok_or_else(|| format!("typetree: truncated at {}", self.pos))?;
        self.pos += n;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8, String> { Ok(self.take(1)?[0]) }
    fn i16(&mut self) -> Result<i16, String> { Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap())) }
    fn i32(&mut self) -> Result<i32, String> { Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn u32(&mut self) -> Result<u32, String> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn i64(&mut self) -> Result<i64, String> { Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn align4(&mut self, base: usize) { let rel = self.pos - base; self.pos = base + ((rel + 3) & !3); }
    fn cstr(&mut self) -> Result<String, String> {
        let end = self.data[self.pos..].iter().position(|&c| c == 0).ok_or("typetree: unterminated string")?;
        let s = String::from_utf8_lossy(&self.data[self.pos..self.pos + end]).into_owned();
        self.pos += end + 1;
        Ok(s)
    }
}

fn be_u32(d: &[u8], p: usize) -> Result<u32, String> {
    d.get(p..p + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).ok_or_else(|| "typetree: short header".into())
}
fn be_i64(d: &[u8], p: usize) -> Result<i64, String> {
    d.get(p..p + 8).map(|b| i64::from_be_bytes(b.try_into().unwrap())).ok_or_else(|| "typetree: short header".into())
}

fn node_string(buffer: &[u8], offset: u32) -> String {
    if offset & 0x8000_0000 != 0 {
        let common = offset & 0x7FFF_FFFF;
        return COMMON_STRINGS
            .iter()
            .find(|(o, _)| *o == common)
            .map_or_else(|| format!("<common:{common}>"), |(_, s)| (*s).to_string());
    }
    let start = offset as usize;
    let end = buffer[start.min(buffer.len())..].iter().position(|&c| c == 0).map_or(buffer.len(), |e| start + e);
    String::from_utf8_lossy(&buffer[start.min(buffer.len())..end]).into_owned()
}

fn read_type_tree(c: &mut Cursor) -> Result<Vec<TypeNode>, String> {
    let node_count = c.i32()? as usize;
    let string_len = c.i32()? as usize;
    let raw_nodes = c.take(node_count * NODE_LEN_V19)?;
    let strings = c.take(string_len)?;
    let mut nodes = Vec::with_capacity(node_count);
    for chunk in raw_nodes.chunks_exact(NODE_LEN_V19) {
        let rd_u32 = |p: usize| u32::from_le_bytes(chunk[p..p + 4].try_into().unwrap());
        let rd_i32 = |p: usize| i32::from_le_bytes(chunk[p..p + 4].try_into().unwrap());
        nodes.push(TypeNode {
            level: chunk[2],
            type_flags: chunk[3],
            type_name: node_string(strings, rd_u32(4)),
            name: node_string(strings, rd_u32(8)),
            byte_size: rd_i32(12),
            meta_flag: rd_i32(20),
        });
    }
    Ok(nodes)
}

/// Parse the metadata (types with type trees + object table) of a v22 file.
pub fn parse(blob: &[u8]) -> Result<SerializedFile, String> {
    let version = be_u32(blob, 8)?;
    if version < 22 {
        return Err(format!("typetree: SerializedFile v{version} unsupported (need >= 22)"));
    }
    if blob.get(16) != Some(&0) {
        return Err("typetree: big-endian metadata unsupported".into());
    }
    let data_offset = be_i64(blob, 32)? as usize;
    let mut c = Cursor { data: blob, pos: HEADER_LEN_V22 };
    let _unity_version = c.cstr()?;
    let _platform = c.i32()?;
    // Player builds strip type trees (e.g. START_Data/*.assets); bundles keep
    // them. Stripped files parse with empty `nodes` — pair them with
    // `borrow_types` from a same-version file that has trees.
    let has_type_tree = c.u8()? != 0;
    let type_count = c.i32()? as usize;
    let mut types = Vec::with_capacity(type_count);
    for _ in 0..type_count {
        let class_id = c.i32()?;
        let _stripped = c.u8()?;
        let _script_index = c.i16()?;
        if class_id == CLASS_MONOBEHAVIOUR {
            c.take(16)?; // script id
        }
        c.take(16)?; // old type hash
        let nodes = if has_type_tree {
            let nodes = read_type_tree(&mut c)?;
            let dep_count = c.i32()? as usize; // type dependencies (v21+, non-ref types)
            c.take(dep_count * 4)?;
            nodes
        } else {
            Vec::new()
        };
        types.push(SerializedType { class_id, nodes });
    }
    let object_count = c.i32()? as usize;
    let mut objects = Vec::with_capacity(object_count);
    for _ in 0..object_count {
        c.align4(0);
        let record_pos = c.pos;
        let path_id = c.i64()?;
        let byte_start = c.i64()? as u64;
        let byte_size = c.u32()?;
        let type_index = c.i32()? as usize;
        objects.push(ObjectInfo { path_id, byte_start, byte_size, type_index, record_pos });
    }
    Ok(SerializedFile { version, data_offset, types, objects })
}

/// Fill a stripped file's empty type trees from a same-Unity-version file that
/// has them (matched by class id; MonoBehaviours are script-specific and are
/// not borrowed). Returns how many types received a tree.
pub fn borrow_types(stripped: &mut SerializedFile, donor: &SerializedFile) -> usize {
    let mut filled = 0;
    for t in stripped.types.iter_mut().filter(|t| t.nodes.is_empty() && t.class_id != CLASS_MONOBEHAVIOUR) {
        if let Some(d) = donor.types.iter().find(|d| d.class_id == t.class_id && !d.nodes.is_empty()) {
            t.nodes = d.nodes.clone();
            filled += 1;
        }
    }
    filled
}

/// Walk one object by its type tree, returning every field's byte span.
/// Verifies the walk consumes exactly the object's byteSize.
pub fn walk_object(blob: &[u8], file: &SerializedFile, object: &ObjectInfo) -> Result<Vec<FieldSpan>, String> {
    let nodes = &file.types.get(object.type_index).ok_or("typetree: bad type index")?.nodes;
    let base = file.data_offset + object.byte_start as usize;
    let mut c = Cursor { data: blob, pos: base };
    let mut spans = Vec::new();
    if nodes.is_empty() {
        return Ok(spans);
    }
    walk_node(&mut c, nodes, 0, base, "", &mut spans)?;
    let consumed = c.pos - base;
    if consumed != object.byte_size as usize {
        return Err(format!(
            "typetree: walk consumed {consumed} bytes but object {} is {} bytes",
            object.path_id, object.byte_size
        ));
    }
    Ok(spans)
}

/// Index range of a node's direct children.
fn children(nodes: &[TypeNode], index: usize) -> Vec<usize> {
    let level = nodes[index].level;
    let mut out = Vec::new();
    let mut i = index + 1;
    while i < nodes.len() && nodes[i].level > level {
        if nodes[i].level == level + 1 {
            out.push(i);
        }
        i += 1;
    }
    out
}

fn walk_node(c: &mut Cursor, nodes: &[TypeNode], index: usize, base: usize, prefix: &str, spans: &mut Vec<FieldSpan>) -> Result<(), String> {
    let node = &nodes[index];
    let path = if prefix.is_empty() { node.name.clone() } else { format!("{prefix}.{}", node.name) };
    let start = c.pos;
    let kids = children(nodes, index);
    let mut align_after = node.meta_flag & META_ALIGN != 0;

    if node.type_flags & TYPE_FLAG_ARRAY != 0 || node.type_name == "Array" {
        // Array: kids[0] = size (int), kids[1] = element template.
        let count = c.i32()?;
        if count < 0 {
            return Err(format!("typetree: negative array length at {path}"));
        }
        let element = *kids.get(1).ok_or("typetree: array without element node")?;
        let elem = &nodes[element];
        let fixed = elem.byte_size > 0 && children(nodes, element).is_empty();
        if fixed {
            c.take(count as usize * elem.byte_size as usize)?;
            if elem.meta_flag & META_ALIGN != 0 {
                align_after = true;
            }
        } else {
            for _ in 0..count {
                walk_node(c, nodes, element, base, &path, spans)?;
            }
        }
    } else if node.type_name == "string" {
        let len = c.i32()? as usize;
        c.take(len)?;
        align_after = true; // strings always align (their inner Array carries the flag)
    } else if node.type_name == "TypelessData" {
        let len = c.i32()? as usize;
        c.take(len)?;
    } else if kids.is_empty() {
        if node.byte_size <= 0 {
            return Err(format!("typetree: leaf {path} ({}) has no size", node.type_name));
        }
        c.take(node.byte_size as usize)?;
    } else {
        for k in kids {
            walk_node(c, nodes, k, base, &path, spans)?;
        }
    }
    if align_after {
        c.align4(base);
    }
    spans.push(FieldSpan { path, type_name: node.type_name.clone(), start, end: c.pos });
    Ok(())
}

/// Replace one object's bytes in place. Everything before it is untouched;
/// later objects shift by the size delta rounded up to 16 bytes, so every
/// object keeps its original alignment (mod 16) and an unchanged replacement
/// reproduces the file byte-for-byte. Patches the object table + header size.
pub fn replace_object(blob: &[u8], file: &SerializedFile, target: &ObjectInfo, new_bytes: &[u8]) -> Result<Vec<u8>, String> {
    const ALIGNMENT: i64 = 16;
    let old_start = file.data_offset + target.byte_start as usize;
    let old_end = old_start + target.byte_size as usize;
    if old_end > blob.len() {
        return Err("typetree: target object out of range".into());
    }
    let delta = new_bytes.len() as i64 - target.byte_size as i64;
    let shift = (delta + ALIGNMENT - 1).div_euclid(ALIGNMENT) * ALIGNMENT; // ceil to 16, works for negatives
    let tail_start = file
        .objects
        .iter()
        .filter(|o| o.byte_start > target.byte_start)
        .map(|o| file.data_offset + o.byte_start as usize)
        .min()
        .unwrap_or(blob.len());
    let new_tail_start = (tail_start as i64 + shift) as usize;
    let mut out = Vec::with_capacity((blob.len() as i64 + shift).max(0) as usize);
    out.extend_from_slice(&blob[..old_start]);
    out.extend_from_slice(new_bytes);
    if out.len() > new_tail_start {
        return Err("typetree: internal layout error (replacement overlaps next object)".into());
    }
    // Preserve the original inter-object padding bytes, then zero-fill any growth.
    let gap_src = &blob[old_end.min(tail_start)..tail_start];
    for i in 0..(new_tail_start - out.len()) {
        out.push(*gap_src.get(i).unwrap_or(&0));
    }
    out.extend_from_slice(&blob[tail_start..]);
    for object in &file.objects {
        let p = object.record_pos;
        if object.path_id == target.path_id {
            out[p + 16..p + 20].copy_from_slice(&(new_bytes.len() as u32).to_le_bytes());
        } else if object.byte_start > target.byte_start {
            let moved = object.byte_start as i64 + shift;
            out[p + 8..p + 16].copy_from_slice(&moved.to_le_bytes());
        }
    }
    let total = out.len() as i64;
    out[24..32].copy_from_slice(&total.to_be_bytes());
    Ok(out)
}