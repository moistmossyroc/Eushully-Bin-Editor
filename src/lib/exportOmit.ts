import type { TabState } from "./stores.svelte";
import type { DialogueEntry } from "./types";
import { isNameItemFile } from "./fileCategory";

/** Labels eligible for auto-omit / bulk-omit. EXCLUDES `set-string` and
 *  `draw-string` — those populate game-state slots (speaker names, UI labels,
 *  etc.) where emptying them produces visible breakage. Their per-row Export
 *  checkbox still works for explicit user overrides; just not for bulk/auto.
 *
 *  EXCEPTION: in character/item NAME files (CVINIT/CIINIT/… — see
 *  `isNameItemFile`) the `set-string` writes ARE the translatable name table, so
 *  they're treated as editable there. `isRowLabelEditable` folds that in. */
const EDITABLE_LABELS = new Set([
  "show-text",
  "display-furigana",
  "comment",
]);

/** Whether a row's label is editable/omit-eligible. `set-string` is unlocked
 *  ONLY in name/item files; in scene/UI files it stays the protected speaker
 *  slot. `sourceBinFilename` is the owning tab's BIN filename. */
function isRowLabelEditable(label: string, sourceBinFilename: string): boolean {
  if (EDITABLE_LABELS.has(label)) return true;
  if (label === "set-string" && isNameItemFile(sourceBinFilename)) return true;
  return false;
}

/** Returns true when a string is empty or contains only structural markup
 *  (brackets and whitespace, no real translatable content). Used to skip
 *  the 3-opcode speaker-name pattern `show-text "【"` / `show-text [ref]` /
 *  `show-text "】"` from export, grouping, and auto-omit. */
export function isStructuralOnly(text: string): boolean {
  if (!text) return true;
  // Brackets and quote/parenthesis variants the engine uses for structural
  // markup. Whitespace allowed. Any other character → not structural.
  return /^[【】「」『』〔〕（）()｛｝\[\]{}\s]*$/.test(text);
}

/** Returns true when an original has no real translatable content:
 *   - empty
 *   - structural-only (just brackets / whitespace)
 *   - no letter characters at all (pure digits / symbols / "????" placeholders)
 *
 *  Used by anything that counts "untranslated rows" — tab progress dot,
 *  status bar entry count, etc. Mirrors what CC Translate export already
 *  filters via the same `isStructuralOnly` + `isNumberOrSymbolOnly` pair so
 *  the dot's "remaining" count matches what Claude would actually see. */
export function hasTranslatableContent(text: string): boolean {
  if (!text) return false;
  if (isStructuralOnly(text)) return false;
  // \p{L} = any Unicode letter. Strings with no letters at all (e.g. "????",
  // "...", "★★", numeric IDs) don't need translation — they're system
  // placeholders the engine renders as-is.
  if (!/\p{L}/u.test(text)) return false;
  return true;
}

/** Whether an entry is eligible for the bulk omit toggle. Matches exactly the
 *  set of rows that display a per-row Export checkbox in EditorRow — so
 *  clicking the bulk toggle visibly flips every checkbox you can see.
 *
 *  Includes: editable rows whose translated_text is empty OR whose omit_export
 *  is already true (e.g. group-translated siblings).
 *  Excludes: deleted, context, non-editable, and translated rows (no checkbox). */
/** An editable text row (show-text / display-furigana / comment) carrying real
 *  content — the universe the Export Review can show under "All editable". */
export function isEditableRow(e: DialogueEntry, sourceBinFilename: string): boolean {
  if (e.deleted || e.is_context_row) return false;
  if (!isRowLabelEditable(e.label, sourceBinFilename)) return false;
  if (isStructuralOnly(e.original_text)) return false;
  return true;
}

export function isOmitEligible(e: DialogueEntry, sourceBinFilename: string): boolean {
  if (e.deleted || e.is_context_row) return false;
  if (!isRowLabelEditable(e.label, sourceBinFilename)) return false;
  // Structural-only rows (brackets, empty, etc.) are not translatable content
  // — omitting them would wipe speaker-name brackets from the BIN.
  if (isStructuralOnly(e.original_text)) return false;
  const hasTranslation = e.translated_text.trim() !== "";
  const alreadyOmitted = e.omit_export === true;
  if (hasTranslation && !alreadyOmitted) return false;
  return true;
}

/** Stats used to drive the toggle UI in both TranslationControls and the
 *  EditorTable column header. */
export interface EmptyStats {
  eligible: number;
  omitted: number;
}

export function computeEmptyStats(tab: TabState | null | undefined): EmptyStats {
  if (!tab) return { eligible: 0, omitted: 0 };
  let eligible = 0;
  let omitted = 0;
  for (const e of tab.entries) {
    if (!isOmitEligible(e, tab.sourceBinFilename)) continue;
    eligible++;
    if (e.omit_export === true) omitted++;
  }
  return { eligible, omitted };
}

/** Bulk-set omit_export on every eligible entry. Builds a fresh array so
 *  `{#each}` children re-render — pure property mutation isn't always tracked
 *  by Svelte 5 when the change originates outside the row component itself.
 *  Returns the number of entries that actually changed. */
export function bulkSetOmitExport(tab: TabState | null | undefined, value: boolean): number {
  if (!tab) return 0;
  let changed = 0;
  const next = tab.entries.map((e) => {
    if (!isOmitEligible(e, tab.sourceBinFilename)) return e;
    if ((e.omit_export === true) === value) return e;
    changed++;
    return { ...e, omit_export: value };
  });
  if (changed > 0) {
    tab.entries = next;
    tab.isDirty = true;
  }
  return changed;
}
