//! Unity-era Eushully archive stack (Yamiyo no Parente Remastered and siblings).
//!
//! ```text
//! .arc (@ARCH000)  -> archive     (index + lazy entry reads)
//!   entry          -> bundle      (UnityFS, LZ4 blocks)
//!     blob         -> serialized  (SerializedFile -> TextAsset payload)
//! ```
//!
//! Consumers:
//!   * `eushully bin editor/eushully-bin-editor-gui/src-tauri` — re-exported as
//!     `crate::parente::{archive, bundle, serialized}` (read + repack scripts).
//!   * `Eushully Sys Decompiler/age-extractor` — read-only browsing/extraction.
//!
//! Format notes live in `eushully-bin-editor-gui/parente/PARENTE_ENGINE.md`.

pub mod archive;
pub mod bundle;
pub mod serialized;
pub mod typetree;
