import type { DialogueEntry } from "./types";
import { furiganaDisplayLength, stripQuoteBrackets } from "./types";
import { isAutoSplitContinuation } from "./insertions";

/** Labels eligible for overflow detection / auto-split. Limited to in-game
 *  rendered dialogue only — comments are translator notes (not shown), and
 *  set-string / draw-string populate game-state slots that don't follow the
 *  per-line character cap. Auto-splitting those would be meaningless. */
export const EDITABLE_OVERFLOW_LABELS = new Set([
  "show-text",
  "display-furigana",
]);

/** One overflowing row with metadata for the review dialog. */
export interface OverflowRow {
  /** Index into tab.entries */
  index: number;
  /** Stable entry id (instruction_index) */
  id: number;
  /** Line-group total visible character count (matches the visual badge) */
  length: number;
  /** This single entry's own visible length (NOT including line-group siblings).
   *  Used to decide if auto-split (which wraps a single row's text) can help. */
  individualLength: number;
  /** Active per-line cap */
  cap: number;
  /** length - cap; always > 0 */
  overflow: number;
  /** The entry itself, for preview/text access */
  entry: DialogueEntry;
  /** Whether this row is an auto-split continuation (already managed) */
  isContinuation: boolean;
  /** True if this entry is part of a multi-entry line group (furigana, etc.) */
  partOfGroup: boolean;
  /** True if auto-split can meaningfully fix THIS row (its own text exceeds the cap).
   *  False for rows that only overflow because of line-group siblings (furigana etc.). */
  splittable: boolean;
}

/** Compute display length for an entry (furigana-aware, strips 「」). */
export function entryDisplayLength(e: DialogueEntry): number {
  const baseLen =
    e.label === "display-furigana"
      ? furiganaDisplayLength(e.translated_text)
      : e.translated_text.length;
  return stripQuoteBrackets(baseLen, e.translated_text);
}

/**
 * Compute the same per-row line-group lengths that EditorTable shows in the
 * UI. A "line group" is the run of show-text + display-furigana entries
 * between two `end-text-line` opcodes — they all render onto one line in-game
 * and share a combined character count.
 *
 * Returns a Map: entry index → { length, groupSize } where groupSize is the
 * number of dialogue entries in the group (used to flag multi-entry lines).
 */
function computeLineGroupLengths(
  entries: DialogueEntry[],
): Map<number, { length: number; groupSize: number }> {
  const out = new Map<number, { length: number; groupSize: number }>();
  let groupIndices: number[] = [];
  let translatedAcc = 0;
  let translatedLine = "";

  const flush = () => {
    const length = stripQuoteBrackets(translatedAcc, translatedLine);
    const groupSize = groupIndices.length;
    for (const idx of groupIndices) {
      out.set(idx, { length, groupSize });
    }
    groupIndices = [];
    translatedAcc = 0;
    translatedLine = "";
  };

  for (let i = 0; i < entries.length; i++) {
    const e = entries[i];
    if (e.label === "end-text-line") {
      flush();
    } else if ((e.label === "show-text" || e.label === "display-furigana") && !e.deleted) {
      groupIndices.push(i);
      if (e.label === "display-furigana") {
        const transDisplay = e.translated_text.match(/^(.+?)\s*\[.+?\]$/);
        translatedLine += transDisplay ? transDisplay[1] : e.translated_text;
        translatedAcc += furiganaDisplayLength(e.translated_text);
      } else {
        translatedLine += e.translated_text;
        translatedAcc += e.translated_text.length;
      }
    }
  }
  if (groupIndices.length > 0) flush();

  return out;
}

/**
 * Scan entries for per-line overflows. Returns rows sorted by severity
 * (biggest overflow first). Matches the visual over-line-cap indicator in
 * EditorRow: a row counts as overflowing when its *line-group total* exceeds
 * the cap (so furigana + show-text combinations count as one line, just like
 * the in-game renderer treats them).
 *
 * Skips empty translations, deleted rows, context rows, and non-translatable
 * labels. For entries outside a line group (e.g. set-string, comment), falls
 * back to single-entry display length.
 */
export function findOverflows(
  entries: DialogueEntry[],
  lineCharCap: number,
): OverflowRow[] {
  if (lineCharCap <= 0) return [];

  const groupLengths = computeLineGroupLengths(entries);

  const out: OverflowRow[] = [];
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i];
    if (e.is_context_row || e.deleted) continue;
    if (!EDITABLE_OVERFLOW_LABELS.has(e.label)) continue;
    if (e.translated_text.trim() === "") continue;

    const group = groupLengths.get(i);
    const individualLength = entryDisplayLength(e);
    const effectiveLength = group ? group.length : individualLength;
    const partOfGroup = !!group && group.groupSize > 1;

    if (effectiveLength <= lineCharCap) continue;

    // Splittable cases:
    //   - individualLength > cap → wrap entry's own text into multiple lines
    //   - partOfGroup → can insert end-text-line opcodes to break the group
    out.push({
      index: i,
      id: e.instruction_index,
      length: effectiveLength,
      individualLength,
      cap: lineCharCap,
      overflow: effectiveLength - lineCharCap,
      entry: e,
      isContinuation: isAutoSplitContinuation(e),
      partOfGroup,
      splittable: individualLength > lineCharCap || partOfGroup,
    });
  }

  out.sort((a, b) => b.overflow - a.overflow);
  return out;
}

/** Opcode that opens a fresh text box (bridge terminator). */
const OP_NEW_BOX_OPEN = 0x1f5;

/** One text box whose contents exceed the per-box limits. */
export interface BoxOverflow {
  /** Array index of the box's first dialogue entry (for jump-to-row). */
  index: number;
  /** instruction_index of that first entry. */
  id: number;
  /** Number of rendered display lines in the box. */
  lines: number;
  /** Configured max lines per box. */
  maxLines: number;
  /** Total visible chars across the box (furigana-aware, 「」 stripped). */
  totalChars: number;
  /** Per-box total char cap (0 = disabled). */
  totalCap: number;
  /** True when the box has more display lines than fit (the 4th-line bug). */
  tooManyLines: boolean;
  /** True when the box's total chars exceed the per-box cap. */
  overTotalCap: boolean;
  /** First-line text, for a readable preview in the review dialog. */
  preview: string;
}

/**
 * Scan entries box-by-box and flag any text box that won't render correctly
 * because it exceeds the user-defined per-box limits:
 *   - more display lines than `maxLinesPerBox` (extra lines get clipped in-game)
 *   - more total characters than `translatedCharCap`
 *
 * A "box" is the run of dialogue between box boundaries (wait-for-input or a
 * `new-box-open` bridge opcode). Lines within a box are separated by
 * `end-text-line`; each non-empty run of show-text/display-furigana is one
 * display line. This is the post-auto-split safety net: auto-split can over-fill
 * a box (e.g. produce a 4th line under a 3-line cap), and this surfaces it so
 * the user can fix it rather than ship a box that drops text.
 */
export function findBoxOverflows(
  entries: DialogueEntry[],
  totalCap: number,
  maxLinesPerBox: number,
): BoxOverflow[] {
  const checkLines = maxLinesPerBox >= 1;
  const checkChars = totalCap > 0;
  if (!checkLines && !checkChars) return [];

  const out: BoxOverflow[] = [];
  let box:
    | { index: number; id: number; lines: number; chars: number; preview: string; inLine: boolean }
    | null = null;

  const finalize = () => {
    if (!box || box.lines === 0) {
      box = null;
      return;
    }
    const tooManyLines = checkLines && box.lines > maxLinesPerBox;
    const overTotalCap = checkChars && box.chars > totalCap;
    if (tooManyLines || overTotalCap) {
      out.push({
        index: box.index,
        id: box.id,
        lines: box.lines,
        maxLines: maxLinesPerBox,
        totalChars: box.chars,
        totalCap,
        tooManyLines,
        overTotalCap,
        preview: box.preview,
      });
    }
    box = null;
  };

  for (let i = 0; i < entries.length; i++) {
    const e = entries[i];
    if (e.deleted) continue;

    // Box boundary — close the current box and await the next one.
    if (e.label === "wait-for-input" || e.opcode === OP_NEW_BOX_OPEN) {
      finalize();
      continue;
    }

    if (e.label === "end-text-line") {
      if (box) box.inLine = false; // current line ended; next content = new line
      continue;
    }

    if (e.label === "show-text" || e.label === "display-furigana") {
      if (e.translated_text.trim() === "") continue;
      if (!box) {
        box = { index: i, id: e.instruction_index, lines: 0, chars: 0, preview: "", inLine: false };
      }
      if (!box.inLine) {
        box.lines++;
        box.inLine = true;
      }
      box.chars += entryDisplayLength(e);
      if (!box.preview) box.preview = e.translated_text;
    }
  }
  finalize();

  return out;
}
