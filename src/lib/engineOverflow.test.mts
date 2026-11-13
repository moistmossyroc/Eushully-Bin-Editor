// Dependency-free unit test for engineOverflow.ts.
// Run: npm run test:engineoverflow   (Node ≥22.6: --experimental-strip-types)
import assert from "node:assert/strict";
import type { DialogueEntry } from "./types.ts";
import { engineBoxes, engineBoxRows, findEngineBoxOverflows, type EngineBoxRules } from "./engineOverflow.ts";
import { engineRows, textCells } from "./gameLayout.ts";

// Atelial (loader report): 12 / 22 px advance, check 11 / 22, row 78..788 → 59 per row.
const metrics = { advanceHalf: 12, advanceFull: 22, checkHalf: 11, checkFull: 22, rowLeft: 78, rowRight: 788 };
const rowsOf = (line: string) => engineRows(line, metrics, true);
const mode: EngineBoxRules = { rowsOf, cellsOf: (r: string) => textCells(r.trimEnd()), rowsPerBox: 3, bottomCap: 35 };

let nextId = 100;
function row(label: string, text = "", opcode = 0): DialogueEntry {
  return { label, translated_text: text, original_text: "", instruction_index: nextId++, opcode, deleted: false, is_context_row: false } as unknown as DialogueEntry;
}

const wind = "The wind carried a faint scent of something familiar, though I couldn't quite place where I had smelled it before. Strange.";
const long = `${wind} ${wind}`;
const entries: DialogueEntry[] = [
  row("show-text", wind), // box 1: 3 rows, bottom row "before. Strange." (16 cells) fits
  row("wait-for-input"),
  row("show-text", long), // box 2: one text, 5 rows → split directly
  row("wait-for-input"),
  row("show-text", wind), // box 3: two lines → 3 + 1 rows, needs re-flow
  row("end-text-line"),
  row("show-text", "Short."),
  row("wait-for-input"),
  row("show-text", "「何故こうもクリエイターが集まって……異変に呼び寄せられたとでもいうの……！？」"), // 2 rows
  row("wait-for-input"),
  row("show-text", "x".repeat(59 * 2) + " " + "y".repeat(40)), // 3rd row 40 > cap 35
];

const boxes = engineBoxes(entries);
assert.equal(boxes.length, 5);
assert.deepEqual(boxes[2].lines, [wind, "Short."]);
assert.equal(engineBoxRows(boxes[0].lines, rowsOf).length, 3);
assert.equal(engineBoxRows(boxes[2].lines, rowsOf).length, 4);

const overflows = findEngineBoxOverflows(entries, mode);
assert.deepEqual(
  overflows.map((o) => [o.index, o.rows, o.singleText, o.bottomRowTooLong]),
  [
    [2, 5, true, false],
    [4, 4, false, false],
    [10, 3, true, true],
  ],
);
// No bottom cap: the 40-cell third row is fine.
assert.equal(findEngineBoxOverflows(entries, { ...mode, bottomCap: 0 }).length, 2);
// Deleted and context rows don't count.
const deleted = entries.map((e, i) => (i === 2 ? { ...e, deleted: true } : e));
assert.equal(findEngineBoxOverflows(deleted, mode).length, 2);

console.log("engineOverflow: all tests passed");
