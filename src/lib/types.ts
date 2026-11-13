/** Mirrors Rust DialogueEntry */
export interface DialogueEntry {
  /** Index in original instruction list. Negative = visually-inserted entry. */
  instruction_index: number;
  opcode: number;
  label: string;
  original_text: string;
  translated_text: string;
  notes: string;
  is_context_row: boolean;
  string_arg_indices: number[];
  deleted: boolean;
  /** When true, export writes empty strings to this instruction's string args
   *  (instead of preserving the original). Used for auto-omitted group-translation
   *  siblings and for user-toggled blank display-furigana rows. */
  omit_export?: boolean;
}

/** Mirrors Rust CharacterEntry */
export interface CharacterEntry {
  pointer: number;
  name: string;
}

/** Mirrors Rust GlossaryEntry */
export interface GlossaryEntry {
  japanese: string;
  english: string;
  category: string;
  notes: string;
}

/** Mirrors Rust InsertedInstruction */
export interface InsertedInstruction {
  after_index: number;
  opcode: number;
  label: string;
  arguments: SerializedArg[];
  /** Frontend-only: links this to the visual entry's instruction_index */
  _syntheticIndex?: number;
  /** Persisted copy of `_syntheticIndex` (written on save, read on load) so
   *  project reload relinks exactly. Absent in legacy projects. */
  synthetic_index?: number;
}

/** Mirrors Rust SerializedArg */
export interface SerializedArg {
  arg_type: number;
  raw_value: number;
  string_value?: string;
}

/** Result from load_bin command */
export interface LoadBinResult {
  entries: DialogueEntry[];
  sys_version: number;
  /** AGE engine build version from the signature (e.g. 443); 0 if unparsed. */
  engine_version: number;
  /** UI label, e.g. "Madou Koukaku · SYS4 (v443)". */
  engine_label: string;
}

/** Mirrors Rust ProjectFile */
export interface ProjectFile {
  version: number;
  source_bin_filename: string;
  source_bin_sha256: string;
  is_ver5: boolean;
  sys_version?: number;
  entries: DialogueEntry[];
  glossary: GlossaryEntry[];
  characters: CharacterEntry[];
  inserted_instructions: InsertedInstruction[];
  settings: ProjectSettings;
}

/** Mirrors Rust ProjectSettings */
export interface ProjectSettings {
  claude_model: string;
  api_key_configured: boolean;
}

/** Mirrors Rust TranslationResult */
export interface TranslationResult {
  translation: string;
  notes: string;
  glossary_updates: GlossaryEntry[];
}

/** Mirrors Rust TranslationProgress */
export interface TranslationProgress {
  status: "Idle" | "Running" | "Paused" | "Stopped";
  current_index: number;
  total_entries: number;
  total_input_tokens: number;
  total_output_tokens: number;
  last_error: string | null;
  block_index?: number;
  total_blocks?: number;
}

/** A pending glossary update from Claude for user review */
export interface GlossaryReviewItem {
  tabId: string;
  existing: GlossaryEntry;
  proposed: GlossaryEntry;
  sourceEntryIndex: number;
}

/** Format for glossary import/export files */
export interface GlossaryExportFile {
  version: number;
  type: "eushully-glossary";
  entries: GlossaryEntry[];
}

/**
 * Returns the visible line length for a display-furigana entry.
 * Format is "kanji [furigana]" — only the kanji part occupies line space in-game.
 * For non-furigana text, returns the full length.
 */
export function furiganaDisplayLength(text: string): number {
  const match = text.match(/^(.+?)\s*\[.+?\]$/);
  return match ? match[1].length : text.length;
}

/**
 * Strips leading 「 and trailing 」 from a character count.
 * These quotation marks don't occupy display space in-game.
 */
export function stripQuoteBrackets(length: number, text: string): number {
  if (text.startsWith("「")) length--;
  if (text.endsWith("」")) length--;
  return Math.max(0, length);
}

/**
 * Substitute Unicode codepoints that have no glyph in any standard font with
 * visually-meaningful placeholders. Display-only — underlying stored text is
 * unchanged so exports keep the original game characters.
 *
 * Now that the app's body font is Meiryo-first (full CJK + standard punctuation
 * coverage), this only handles the Private Use Area (U+E000..U+F8FF). Those
 * codepoints are game-specific glyphs no font renders, and they leak into the
 * UI when loading project JSON saved before the SJIS PUA-remap fix landed.
 */
export function displaySafe(text: string): string {
  if (!text) return text;
  let needsRewrite = false;
  for (let i = 0; i < text.length; i++) {
    const cp = text.charCodeAt(i);
    if (cp >= 0xE000 && cp <= 0xF8FF) { needsRewrite = true; break; }
  }
  if (!needsRewrite) return text;
  let out = "";
  for (const ch of text) {
    const cp = ch.codePointAt(0)!;
    if (cp === 0xE000) out += "…";                                 // Eushully PUA ellipsis
    else if (cp === 0xE001 || cp === 0xE003) out += "—";           // Eushully PUA em dashes
    else if (cp >= 0xE000 && cp <= 0xF8FF) out += "〓";             // unknown PUA -> geta mark
    else out += ch;
  }
  return out;
}

/** Mirrors Rust SerializedInstruction from bridge.rs */
export interface SerializedInstruction {
  opcode: number;
  label: string;
  arguments: { arg_type: number; raw_data: number; decoded_string?: string }[];
}

/** Mirrors Rust SplitPlan from bridge.rs */
export interface SplitPlan {
  source_entry_index: number;
  /** Outer = boxes, inner = lines within each box. Lines are separate
   *  show-text instructions joined by end-text-line opcodes. */
  box_lines: string[][];
  bridge_instructions_per_split: SerializedInstruction[][];
  show_text_template_args: { arg_type: number; raw_data: number; decoded_string?: string }[];
  /** end-text-line template (opcode 0x6F) cloned from the source BIN.
   *  Null if no ETL was present (multi-line splits will be skipped). */
  end_text_line_template: SerializedInstruction | null;
}

export interface ClaudeModel {
  id: string;
  label: string;
}

/** Pre-export summary shown by ExportDiffDialog. Built from a tab's entries
 *  and inserted instructions in actions.ts; rendered as stat cards. */
export interface ExportDiff {
  translatedCount: number;
  insertedCount: number;
  omittedCount: number;
  deletedCount: number;
  untranslatedCount: number;
  /** Every translated row whose text differs from the source — the full set,
   *  rendered as a scrollable list for a final pre-export review. */
  changes: { id: number; label: string; original: string; translated: string }[];
}

/** Hardcoded fallback models used when the API is unreachable */
export const DEFAULT_CLAUDE_MODELS: ClaudeModel[] = [
  { id: "claude-sonnet-4-6-20250627", label: "Claude Sonnet 4.6" },
  { id: "claude-opus-4-6-20250627", label: "Claude Opus 4.6" },
  { id: "claude-haiku-4-5-20251001", label: "Claude Haiku 4.5" },
];
