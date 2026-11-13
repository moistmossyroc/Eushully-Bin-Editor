use crate::editor::extract::DialogueEntry;
use serde::{Deserialize, Serialize};

/// Serializable project file format for saving/loading translation state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    /// Format version for future compatibility
    pub version: u32,
    /// Original BIN filename (not full path, for portability)
    pub source_bin_filename: String,
    /// SHA-256 hash of the original BIN file to verify integrity
    pub source_bin_sha256: String,
    /// Whether the source BIN is SYS5 format (kept for backward compat)
    pub is_ver5: bool,
    /// Engine version number (3, 4, or 5). Defaults to 0 in old project files.
    #[serde(default)]
    pub sys_version: u8,
    /// Dialogue entries with translations and notes
    pub entries: Vec<DialogueEntry>,
    /// Translation glossary
    pub glossary: Vec<GlossaryEntry>,
    /// Character/speaker name mappings
    #[serde(default)]
    pub characters: Vec<CharacterEntry>,
    /// Instructions to be inserted during export
    pub inserted_instructions: Vec<InsertedInstruction>,
    /// Project settings
    pub settings: ProjectSettings,
}

/// A character/speaker entry mapping a numeric pointer to a name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterEntry {
    /// Numeric speaker pointer from show-text arg[0] (displayed as hex)
    pub pointer: u32,
    /// Resolved character name (empty if not yet resolved)
    pub name: String,
}

/// A glossary entry for consistent translation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlossaryEntry {
    /// Original Japanese term
    pub japanese: String,
    /// English translation
    pub english: String,
    /// Category: "character", "place", "term", "nuance", "joke"
    pub category: String,
    /// Additional notes about this entry
    pub notes: String,
}

/// An instruction to be inserted into the BIN during export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsertedInstruction {
    /// Insert after this instruction index (in the original instruction list)
    pub after_index: usize,
    /// Opcode of the new instruction
    pub opcode: u32,
    /// Label of the new instruction
    pub label: String,
    /// Arguments for the new instruction
    pub arguments: Vec<SerializedArg>,
    /// Frontend link to the visual entry (its negative `instruction_index`).
    /// Persisted so project reload can relink EXACTLY — the old label/opcode
    /// greedy relink crossed same-opcode bridge rows and permuted their args
    /// (LaDEA "box opens, text invisible until backlog"). Ignored by export;
    /// absent in legacy projects (serde default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synthetic_index: Option<i64>,
}

/// A serializable argument for inserted instructions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedArg {
    /// Argument type (matches the BIN format type field)
    pub arg_type: u32,
    /// Raw numeric value
    pub raw_value: u32,
    /// String value (for string arguments, type == 2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string_value: Option<String>,
}

/// Project-level settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// Selected Claude model for translation
    pub claude_model: String,
    /// Whether an API key has been configured (key stored in OS keyring)
    pub api_key_configured: bool,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            claude_model: "claude-sonnet-4-20250514".to_string(),
            api_key_configured: false,
        }
    }
}

impl ProjectFile {
    /// Create a new project from a parsed BIN file.
    pub fn new(
        source_bin_filename: String,
        source_bin_sha256: String,
        is_ver5: bool,
        sys_version: u8,
        entries: Vec<DialogueEntry>,
    ) -> Self {
        Self {
            version: 1,
            source_bin_filename,
            source_bin_sha256,
            is_ver5,
            sys_version,
            entries,
            glossary: Vec::new(),
            characters: Vec::new(),
            inserted_instructions: Vec::new(),
            settings: ProjectSettings::default(),
        }
    }
}
