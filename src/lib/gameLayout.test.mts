// Dependency-free unit test for gameLayout.ts (no TS test runner in repo).
// Run: npm run test:gamelayout   (Node ≥22.6: --experimental-strip-types)
import assert from "node:assert/strict";
import { applyBoxProfile, dialogueBoxCaps, bottomLineCapFromArt, bottomRowOverCap, lineCapacityFullWidth, engineRows, defaultEngineMetrics, bottomLineUiZone, charCells, layoutLines, lineCapacityCells, textCells, visibleLineCount } from "./gameLayout.ts";

// Atelial native: surface 800×120, origin 78,52, font 22, spacing 12.
const native = { width: 800, height: 120, textX: 78, textY: 52, fontPx: 22, lineSpacing: 12 };
// Atelial 3-line profile: lift 3, grow 5, spacing 5.
const profiled = { ...native, height: 125, textY: 49, lineSpacing: 5 };

assert.equal(charCells("a"), 1);
assert.equal(charCells("重"), 2);
assert.equal(charCells("ｱ"), 1); // half-width katakana
assert.equal(textCells("「abc」"), 7);

assert.equal(visibleLineCount(native), 2); // what the game shows
assert.equal(visibleLineCount(profiled), 3);
// Engine rule (in-game 2026-11-12, loader log): bold MS Gothic 22 → 12 px half / 22 px
// full advance, check 11 / 22, row 78..788 → 59 half-width / 32 full-width per row.
assert.deepEqual(defaultEngineMetrics(native), { advanceHalf: 12, advanceFull: 22, checkHalf: 11, checkFull: 22, rowLeft: 78, rowRight: 788 });
assert.equal(lineCapacityCells(native), 59);
assert.equal(lineCapacityFullWidth(native), 32);

assert.deepEqual(layoutLines(["one", "two", "three"], native).map((l) => l.vertical), ["visible", "visible", "hidden"]);
// First in-game test (spacing 6, origin unchanged): line 3 starts at 108, cut at 120 — as seen.
const firstTest = { ...native, lineSpacing: 6 };
assert.deepEqual(layoutLines(["one", "two", "three"], firstTest).map((l) => l.vertical), ["visible", "visible", "partial"]);
assert.deepEqual(layoutLines(["a", "b", "c", "d"], native).map((l) => l.vertical)[3], "hidden");
assert.deepEqual(layoutLines(["one", "two", "three"], profiled).map((l) => l.vertical), ["visible", "visible", "visible"]);

// Native per-character rows (in-game #54) and the word-wrap hook (#60), mirrors wrap.rs.
const metrics = defaultEngineMetrics(native);
const wind = "The wind carried a faint scent of something familiar, though I couldn't quite place where I had smelled it before. Strange.";
assert.equal(engineRows(wind, metrics, false)[0], "The wind carried a faint scent of something familiar, thoug");
assert.deepEqual(engineRows(wind, metrics, true), [
  "The wind carried a faint scent of something familiar, ",
  "though I couldn't quite place where I had smelled it ",
  "before. Strange.",
]);
// Japanese: 32 full-width per row, no indent on the continuation (#64).
const japanese = "「何故こうもクリエイターが集まって……異変に呼び寄せられたとでもいうの……！？」";
assert.deepEqual(engineRows(japanese, metrics, true).map((r) => [...r].length), [32, 8]);
// Word-wrapped rows report their visible width (trailing space dropped).
const placedWind = layoutLines([wind], native, { metrics, wordWrap: true });
assert.deepEqual(placedWind.map((r) => [r.cellWidth, r.wrapped]), [[53, false], [52, true], [16, true]]);
// With a third line the bottom row must respect the button cap.
const profiledRows = layoutLines(["t".repeat(100), "u".repeat(40)], profiled);
assert.deepEqual(profiledRows.map((r) => r.cellWidth), [59, 41, 40]);
assert.equal(bottomRowOverCap(profiledRows, 3, 35), true);
assert.equal(bottomRowOverCap(profiledRows, 3, 0), false);

const zone = bottomLineUiZone(profiled, 3, 34);
assert.ok(zone);
assert.equal(zone.x, 78 + 34 * 11);
assert.equal(zone.y, 49 + 2 * 27);
assert.equal(bottomLineUiZone(profiled, 3, 0), null);

// P3: Atelial SYSTEM4 art — band (full width) + Q.SAVE…✕ row at y 542.
const art = [
  { dstX: 67, dstY: 451, width: 890, height: 102 },
  { dstX: 579, dstY: 542, width: 44, height: 27 },
  { dstX: 628, dstY: 542, width: 44, height: 27 },
  { dstX: 893, dstY: 542, width: 24, height: 19 },
];
const onScreen = { ...profiled, x: 112, y: 423 };
assert.equal(bottomLineCapFromArt(onScreen, art, 3), 35); // (579 - 190) / 11
assert.equal(bottomLineCapFromArt({ ...native, x: 112, y: 423 }, art, 2), null); // native 2 lines clear the buttons

// Box profile: Atelial lift 3, grow 5, spacing 5 on the native box = the verified 3-line box.
const applied = applyBoxProfile(native, { liftPx: 3, growPx: 5, lineSpacing: 5 });
assert.deepEqual([applied.height, applied.textY, applied.lineSpacing], [125, 49, 5]);
assert.equal(visibleLineCount(applied), 3);
assert.equal(applyBoxProfile(native, { liftPx: 80, growPx: 0, lineSpacing: 12 }).textY, 0); // not past the top

console.log("gameLayout: all tests passed");

// Re-flow caps from the game box: native Atelial 59 × 2, with the 3-line profile 59 × 3.
assert.deepEqual(dialogueBoxCaps({ body: native, baseBody: native }, undefined), { charsPerLine: 59, linesPerBox: 2 });
assert.deepEqual(dialogueBoxCaps({ body: native, baseBody: native }, { enabled: true, profile: { liftPx: 3, growPx: 5, lineSpacing: 5 } }), { charsPerLine: 59, linesPerBox: 3 });
console.log("gameLayout: box caps tests passed");
