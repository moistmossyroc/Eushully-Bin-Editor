// Box overflow under the engine's own row breaking (loader word-wrap mode; no runtime
// imports — the row model is injected (engineMode.ts builds it from gameLayout); unit-tested
// in engineOverflow.test.mts). A box is the dialogue between
// box boundaries (wait-for-input / new-box-open); each `end-text-line` forces a new row,
// and the engine wraps every line into rows itself (gameLayout.engineRows). A box
// overflows when its rows exceed the box, or when a full box's bottom row passes the
// bottom-row cap (UI art shares that row, e.g. Atelial's buttons).
import type { DialogueEntry } from "./types";

/** Opcode that opens a fresh text box (bridge terminator). */
const OP_NEW_BOX_OPEN = 0x1f5;

/** The engine box rules of a tab's game (word wrap on, metrics reported). */
export interface EngineBoxRules {
  /** Rows the engine breaks one display line into (gameLayout.engineRows). */
  rowsOf: (line: string) => string[];
  /** Visible width of a row in half-width cells (trailing spaces ignored). */
  cellsOf: (row: string) => number;
  rowsPerBox: number;
  /** Max half-width cells of a full box's bottom row (0 = no cap). */
  bottomCap: number;
}

/** One box of dialogue as the engine receives it. */
export interface EngineBox {
  /** Array index of the box's first text entry. */
  index: number;
  /** instruction_index of that entry. */
  id: number;
  /** Display lines (end-text-line separated), each the concatenated visible text. */
  lines: string[];
  /** instruction_index of every text entry in the box. */
  entryIds: number[];
}

/** A box the engine can't show in one go. */
export interface EngineBoxOverflow {
  index: number;
  id: number;
  rows: number;
  rowsPerBox: number;
  bottomRowTooLong: boolean;
  /** The whole box is one show-text: it can be split into engine boxes directly. */
  singleText: boolean;
  preview: string;
}

/** Visible text of a dialogue entry: furigana rows show only their base text. */
function visibleText(entry: DialogueEntry): string {
  if (entry.label !== "display-furigana") return entry.translated_text;
  const match = entry.translated_text.match(/^(.+?)\s*\[.+?\]$/);
  return match ? match[1] : entry.translated_text;
}

/** Group entries into boxes and display lines (deleted rows and empty texts skipped). */
export function engineBoxes(entries: DialogueEntry[]): EngineBox[] {
  const boxes: EngineBox[] = [];
  let box: (EngineBox & { inLine: boolean }) | null = null;
  const close = () => {
    if (box && box.lines.length > 0) boxes.push({ index: box.index, id: box.id, lines: box.lines, entryIds: box.entryIds });
    box = null;
  };
  entries.forEach((entry, i) => {
    if (entry.deleted || entry.is_context_row) return;
    if (entry.label === "wait-for-input" || entry.opcode === OP_NEW_BOX_OPEN) {
      close();
      return;
    }
    if (entry.label === "end-text-line") {
      if (box) box.inLine = false;
      return;
    }
    if (entry.label !== "show-text" && entry.label !== "display-furigana") return;
    if (entry.translated_text.trim() === "") return;
    if (!box) box = { index: i, id: entry.instruction_index, lines: [], entryIds: [], inLine: false };
    if (!box.inLine) {
      box.lines.push("");
      box.inLine = true;
    }
    box.lines[box.lines.length - 1] += visibleText(entry);
    box.entryIds.push(entry.instruction_index);
  });
  close();
  return boxes;
}

/** Rows the engine lays a box out in (each line starts a new row). */
export function engineBoxRows(lines: string[], rowsOf: EngineBoxRules["rowsOf"]): string[] {
  return lines.flatMap(rowsOf);
}

/** Boxes the engine can't show: too many rows, or a full box's bottom row over the cap. */
export function findEngineBoxOverflows(entries: DialogueEntry[], rules: EngineBoxRules): EngineBoxOverflow[] {
  if (rules.rowsPerBox < 1) return [];
  const out: EngineBoxOverflow[] = [];
  for (const box of engineBoxes(entries)) {
    const rows = engineBoxRows(box.lines, rules.rowsOf);
    const tooManyRows = rows.length > rules.rowsPerBox;
    const bottomRowTooLong =
      !tooManyRows && rules.bottomCap > 0 && rows.length === rules.rowsPerBox && rules.cellsOf(rows[rules.rowsPerBox - 1]) > rules.bottomCap;
    if (!tooManyRows && !bottomRowTooLong) continue;
    out.push({
      index: box.index,
      id: box.id,
      rows: rows.length,
      rowsPerBox: rules.rowsPerBox,
      bottomRowTooLong,
      singleText: box.entryIds.length === 1,
      preview: box.lines.join(" / "),
    });
  }
  return out;
}
