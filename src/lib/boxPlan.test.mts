// Dependency-free unit test for boxPlan.ts (no TS test runner in repo).
// Run: npm run test:boxplan   (Node ≥22.6: --experimental-strip-types)
import assert from "node:assert/strict";
import { boxFits, bottomSlotOverflows, planBoxSizes } from "./boxPlan.ts";

// Atelial 3-line profile: full lines ~60 chars, bottom line ~34 (button row).
const MAX = 3;
const BOTTOM = 34;

// No bottom cap: unchanged balanced behaviour.
assert.deepEqual(planBoxSizes([50, 50, 50, 50], MAX, 0), [2, 2]);
assert.deepEqual(planBoxSizes([50, 50, 50, 50, 50, 50, 50], MAX, 0), [3, 2, 2]);

// Balanced split already keeps long lines out of the bottom slot.
assert.deepEqual(planBoxSizes([60, 60, 60, 60], MAX, BOTTOM), [2, 2]);

// A full 3-line box whose third line is too long: ends after 2, line 3 opens box 2.
assert.deepEqual(planBoxSizes([60, 60, 60], MAX, BOTTOM), [2, 1]);

// Short bottom line is fine: one box.
assert.deepEqual(planBoxSizes([60, 60, 20], MAX, BOTTOM), [3]);

// Balanced [3,3] would put a long line in box 1's bottom slot → greedy fill.
const mixed = [60, 60, 60, 60, 60, 20];
const sizes = planBoxSizes(mixed, MAX, BOTTOM);
assert.equal(sizes.reduce((a, b) => a + b, 0), mixed.length);
let start = 0;
for (const size of sizes) {
  assert.ok(boxFits(mixed.slice(start, start + size), MAX, BOTTOM), `box ${sizes} fits`);
  start += size;
}

// Detector helpers.
assert.equal(bottomSlotOverflows([60, 60, 60], MAX, BOTTOM), true);
assert.equal(bottomSlotOverflows([60, 60], MAX, BOTTOM), false); // bottom slot unused
assert.equal(bottomSlotOverflows([60, 60, 60], MAX, 0), false); // cap disabled
assert.equal(boxFits([10, 10, 10, 10], MAX, 0), false); // too many lines

// Balanced re-flow rows (wrap_rows_balanced, Atelial 63 × 3, bottom 35): 59/44 | 59/39 → 2+2.
assert.deepEqual(planBoxSizes([59, 44, 59, 39], MAX, 35), [2, 2]);
// Orphan guard: greedy 3+1 becomes 2+2 (no single-line last box).
assert.deepEqual(planBoxSizes([63, 63, 35, 44], MAX, 35), [2, 2]);
assert.deepEqual(planBoxSizes([60, 20, 20, 60, 20, 20, 30], MAX, 34), [3, 2, 2]);

console.log("boxPlan: all tests passed");
