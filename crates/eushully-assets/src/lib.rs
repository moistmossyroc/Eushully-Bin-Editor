//! eushully-assets — the UI-agnostic engine behind the AGE Extractor, the
//! `eushully` CLI and agf-translator, kept separate so the future Eushully
//! suite (Tauri, in the BIN editor) can use the exact same code. No GUI types
//! appear anywhere in this crate.
//!
//! - [`age_index`]: pure SYS*INI.BIN / AAI table parsers (bytes in, table out).
//! - [`archive`]: open a game (AGE SYS3/4/5 + ALF/AAI, loose files, or
//!   Parente via [`parente`]) into an `AgeArchive` of `FileEntry`s;
//!   `ContainerHandles` keeps container files open for bulk reads.
//! - [`utils::game_discovery`]: find game roots below a picked folder.
//! - [`entry_index`]: per-archive lookups (kinds, folder tree, tier copies).
//! - [`preview`]: decode an entry for display (image / audio / text / info).
//! - [`agf`]: the AGF (ACGF) codec — decode, `AgfDocument`, re-encode edited
//!   images (8bpp via [`utils::palette_quantize`], 24bpp, ACIF alpha).
//! - [`lzss`]: the AGE LZSS variant.
//! - [`fanpatch`]: the patch loader's `.fanpatch` mod archives (read, list, write).
//! - [`version_compare`]: original vs mod — similarity, changed-pixel maps,
//!   "made from an older original" / "identical" / "new file" verdicts.
//! - [`extract`]: background extraction (parallel decode/encode/write,
//!   deterministic naming) with conversion and progress.
//! - [`utils`]: pure helpers (magic sniffing, diffs, paths, external tools,
//!   `parallel_map_with`).
//!
//! Regression tools: `examples/index_fingerprint.rs` (index parsers),
//! `examples/extract_bench.rs` (extraction speed + output checksum),
//! `tests/robustness.rs` (mutation testing), `eushully audit --roundtrip`
//! (every retail AGF decodes and re-encodes identically).
pub mod age_index;
pub mod agf;
pub mod archive;
pub mod entry_index;
pub mod extract;
pub mod fanpatch;
pub mod lzss;
pub mod parente;
pub mod preview;
pub mod utils;
pub mod version_compare;

/// Re-exported so front-ends name the same `RgbaImage` type without pinning their own version.
pub use image;
