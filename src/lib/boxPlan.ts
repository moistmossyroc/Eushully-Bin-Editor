// Pure box-pagination planning (no store / Svelte imports, unit-tested in boxPlan.test.mts).
//
// A text box shows up to `maxLines` lines. Some games share the bottom line with UI art
// (Atelial's 3-line profile: Q.SAVE…✕ buttons sit on line 3's row), so the BOTTOM slot
// only fits `bottomLineCap` characters. A line too long for that slot must not land
// there: the box ends one line early and the line opens the next box instead.

/** True when a box of these line lengths renders cleanly. */
export function boxFits(lineLengths: number[], maxLines: number, bottomLineCap: number): boolean {
  if (lineLengths.length > maxLines) return false;
  return !bottomSlotOverflows(lineLengths, maxLines, bottomLineCap);
}

/** True when the box fills its bottom slot with a line wider than the bottom-line cap. */
export function bottomSlotOverflows(lineLengths: number[], maxLines: number, bottomLineCap: number): boolean {
  return bottomLineCap > 0 && lineLengths.length === maxLines && lineLengths[maxLines - 1] > bottomLineCap;
}

/**
 * Lines per box for one over-long or bottom-overflowing box (sum = lineLengths.length).
 * Without a bottom-line cap: the balanced split (fewest boxes, lines spread evenly, no
 * orphan line: 4 lines / max 3 → 2+2). With one: a greedy fill — the wrap already laid
 * the rows out as balanced boxes (wrap.rs wrap_rows_balanced), and a box ends early where
 * the next row is too long for its bottom slot; then an orphan guard moves a line down so
 * the last box never holds a single line when the previous box can spare one.
 */
export function planBoxSizes(lineLengths: number[], maxLines: number, bottomLineCap: number): number[] {
  const total = lineLengths.length;
  if (maxLines < 1 || total === 0) return total === 0 ? [] : [total];

  if (bottomLineCap <= 0) return balancedSizes(total, Math.ceil(total / maxLines));

  const sizes: number[] = [];
  let start = 0;
  while (start < total) {
    let take = Math.min(maxLines, total - start);
    if (take === maxLines && bottomLineCap > 0 && lineLengths[start + take - 1] > bottomLineCap) take = Math.max(1, take - 1);
    sizes.push(take);
    start += take;
  }
  const last = sizes.length - 1;
  if (last > 0 && sizes[last] === 1 && sizes[last - 1] > 2) {
    const moved = [...sizes.slice(0, last - 1), sizes[last - 1] - 1, 2];
    const boxStart = total - 2 - (sizes[last - 1] - 1);
    if (boxFits(lineLengths.slice(boxStart, total - 2), maxLines, bottomLineCap) && boxFits(lineLengths.slice(total - 2), maxLines, bottomLineCap)) {
      return moved;
    }
  }
  return sizes;
}

function balancedSizes(total: number, boxes: number): number[] {
  const base = Math.floor(total / boxes);
  const extra = total % boxes; // the first `extra` boxes get one more line
  return Array.from({ length: boxes }, (_, b) => base + (b < extra ? 1 : 0));
}

