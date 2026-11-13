// Pure in-game text layout for the game-accurate dialog preview (no store / Svelte
// imports; unit-tested in gameLayout.test.mts). Engine model (docs/engine/age-sys.md §7b,
// decompiled + verified in-game on Atelial 2026-11-12): each glyph advances
// `advanceHalf` / `advanceFull` px; a row breaks before a glyph when cursor + check
// width passes `rowRight` (per character, no indent); rows step font + spacing; the
// surface clips rows at its bottom edge. With the loader's English word wrap, an ASCII
// word that would not fit starts the next row and a space never starts one. Metrics come
// from the loader's fanpatch_layout.txt when present (mirrors wrap.rs engine_rows).

/** The box-geometry fields the layout needs (subset of ipc.ts BoxGeometry). */
export interface LayoutGeometry {
  /** Surface position on screen (canvas pixels); 0 when unknown. */
  x?: number;
  y?: number;
  width: number;
  height: number;
  textX: number;
  textY: number;
  fontPx: number;
  lineSpacing: number;
}

export interface PlacedChar {
  ch: string;
  x: number;
  /** Half-width cells (1) or full-width (2). */
  cells: number;
  /** Past the surface's right edge (clipped in-game). */
  clipped: boolean;
}

export interface PlacedLine {
  /** Top of the glyph cell row. */
  y: number;
  chars: PlacedChar[];
  /** "visible" | "partial" (cut by the bottom edge) | "hidden" (entirely below it). */
  vertical: "visible" | "partial" | "hidden";
  /** Some characters run past the right edge (only a single glyph wider than the line). */
  clippedRight: boolean;
  /** Width in half-width cells. */
  cellWidth: number;
  /** Index of the script line this row belongs to. */
  sourceLine: number;
  /** Continuation row produced by the engine's automatic wrap. */
  wrapped: boolean;
}

/** Leading letters of a BIN name ("SC0010.BIN" → "SC", "CIMES03.BIN" → "CIMES"). */
export function binPrefix(fileName: string): string {
  return (fileName.toUpperCase().match(/^[A-Z]+/) ?? [""])[0];
}

/**
 * Geometry for previewing text in `slot`. Slot 1 is the ADV body box with the ADV
 * font. Other slots (UI, battle, info) use their own surface + origin and the font the
 * script itself sets, falling back to the ADV font.
 */
export function slotGeometry(
  layout: { body: LayoutGeometry; slots: { slot: number; x: number; y: number; width: number; height: number; textX: number; textY: number }[]; scriptFontPx: number | null; scriptLineSpacing: number | null },
  slot: number,
): LayoutGeometry {
  const found = layout.slots.find((s) => s.slot === slot);
  if (slot === 1 || !found) return layout.body;
  return {
    x: found.x,
    y: found.y,
    width: found.width,
    height: found.height,
    textX: found.textX,
    textY: found.textY,
    fontPx: layout.scriptFontPx ?? layout.body.fontPx,
    lineSpacing: layout.scriptLineSpacing ?? layout.body.lineSpacing,
  };
}

/** Re-flow caps measured from the game: half-width characters per row (engine wrap)
 *  and rows per box, for the dialogue box with the game's enabled text box profile. */
export interface GameBoxCaps {
  charsPerLine: number;
  linesPerBox: number;
}

export function dialogueBoxCaps(
  layout: { body: LayoutGeometry; baseBody: LayoutGeometry | null; metrics?: EngineMetrics | null },
  profile: { enabled: boolean; profile: ProfileShift } | undefined,
): GameBoxCaps {
  const geometry = profile?.enabled && layout.baseBody ? applyBoxProfile(layout.baseBody, profile.profile) : layout.body;
  return { charsPerLine: lineCapacityCells(geometry, layout.metrics ?? defaultEngineMetrics(geometry)), linesPerBox: visibleLineCount(geometry) };
}

/** Half-width = ASCII printable + half-width katakana/forms; everything else is full width. */
export function charCells(ch: string): number {
  const code = ch.codePointAt(0) ?? 0;
  if (code >= 0x20 && code <= 0x7e) return 1;
  if (code >= 0xff61 && code <= 0xff9f) return 1;
  return 2;
}

export function textCells(text: string): number {
  let cells = 0;
  for (const ch of text) cells += charCells(ch);
  return cells;
}

/** The engine's text metrics (ipc.ts EngineTextMetrics subset; loader-reported). */
export interface EngineMetrics {
  advanceHalf: number;
  advanceFull: number;
  checkHalf: number;
  checkFull: number;
  /** Row extent in the surface's coordinates (rowLeft = text origin x). */
  rowLeft: number;
  rowRight: number;
}

/**
 * Metrics when the loader hasn't reported any: the rule verified on Atelial (bold
 * MS Gothic at lfWidth = font/2 advances font/2 + 1, checks font/2; full-width font; the
 * row ends one half-width advance before the surface edge: 800 → 788). INFERRED for
 * other games until their loader report exists.
 */
export function defaultEngineMetrics(geometry: LayoutGeometry): EngineMetrics {
  const half = Math.floor(geometry.fontPx / 2);
  return {
    advanceHalf: half + 1,
    advanceFull: geometry.fontPx,
    checkHalf: half,
    checkFull: geometry.fontPx,
    rowLeft: geometry.textX,
    rowRight: geometry.width - (half + 1),
  };
}

function isWordChar(ch: string): boolean {
  return ch > " " && ch <= "~";
}

/** Rows the engine lays `text` out in (wrap.rs engine_rows, same rule and tests). */
export function engineRows(text: string, metrics: EngineMetrics, wordWrap: boolean): string[] {
  const chars = [...text];
  const advance = (ch: string) => (charCells(ch) === 1 ? metrics.advanceHalf : metrics.advanceFull);
  const check = (ch: string) => (charCells(ch) === 1 ? metrics.checkHalf : metrics.checkFull);
  const slack = metrics.advanceHalf - metrics.checkHalf;
  const rows: string[] = [""];
  let x = metrics.rowLeft;
  chars.forEach((ch, i) => {
    const rowHasText = rows[rows.length - 1] !== "";
    let newRow: boolean;
    if (wordWrap && ch === " ") {
      newRow = false;
    } else if (wordWrap && isWordChar(ch) && (i === 0 || chars[i - 1] === " ") && x > metrics.rowLeft) {
      let word = 0;
      for (let j = i; j < chars.length && isWordChar(chars[j]); j++) word += advance(chars[j]);
      newRow = x + word - slack > metrics.rowRight;
    } else {
      newRow = rowHasText && x + check(ch) > metrics.rowRight;
    }
    if (newRow) {
      rows.push("");
      x = metrics.rowLeft;
    }
    rows[rows.length - 1] += ch;
    x += advance(ch);
  });
  return rows;
}

/** Whether `text` fits one engine-wrapped box (wrap.rs engine_box_fits): at most
 *  `rowsPerBox` rows, and a full box's bottom row within `bottomCap` cells (0 = no cap). */
export function engineBoxFits(text: string, metrics: EngineMetrics, wordWrap: boolean, rowsPerBox: number, bottomCap: number): boolean {
  const rows = engineRows(text, metrics, wordWrap);
  if (rows.length > rowsPerBox) return false;
  return bottomCap <= 0 || rows.length < rowsPerBox || textCells(rows[rowsPerBox - 1].trimEnd()) <= bottomCap;
}

/** How the preview wraps: engine metrics (reported or default) and the word-wrap hook. */
export interface WrapMode {
  metrics: EngineMetrics;
  wordWrap: boolean;
}

export function layoutLines(lines: string[], geometry: LayoutGeometry, mode: WrapMode = { metrics: defaultEngineMetrics(geometry), wordWrap: false }): PlacedLine[] {
  const step = geometry.fontPx + geometry.lineSpacing;
  const { metrics } = mode;
  const placed: PlacedLine[] = [];
  lines.forEach((text, sourceLine) => {
    engineRows(text, metrics, mode.wordWrap).forEach((rowText, part) => {
      const y = geometry.textY + placed.length * step;
      const vertical: PlacedLine["vertical"] =
        y >= geometry.height ? "hidden" : y + geometry.fontPx > geometry.height ? "partial" : "visible";
      let x = geometry.textX;
      let cellWidth = 0;
      const chars: PlacedChar[] = [];
      for (const ch of rowText) {
        const cells = charCells(ch);
        const advance = cells === 1 ? metrics.advanceHalf : metrics.advanceFull;
        chars.push({ ch, x, cells, clipped: x + advance > geometry.width });
        x += advance;
        cellWidth += cells;
      }
      // Trailing spaces of a word-wrapped row are invisible: not part of its width.
      cellWidth -= rowText.length - rowText.trimEnd().length;
      placed.push({ y, chars, vertical, clippedRight: chars.some((c) => c.clipped), cellWidth, sourceLine, wrapped: part > 0 });
    });
  });
  return placed;
}

/** Rows (after the engine's wrap) that collide with UI art in the bottom slot: the row in
 *  slot `maxLines - 1` wider than the bottom-line cap. */
export function bottomRowOverCap(placed: PlacedLine[], maxLines: number, bottomLineCap: number): boolean {
  if (bottomLineCap <= 0 || maxLines < 1) return false;
  const row = placed[maxLines - 1];
  return row !== undefined && row.cellWidth > bottomLineCap;
}

/** Half-width characters a row holds under the engine rule (Atelial 59). */
export function lineCapacityCells(geometry: LayoutGeometry, metrics: EngineMetrics = defaultEngineMetrics(geometry)): number {
  return [...engineRows("a".repeat(400), metrics, false)[0]].length;
}

/** Full-width characters a row holds (Atelial 32). */
export function lineCapacityFullWidth(geometry: LayoutGeometry, metrics: EngineMetrics = defaultEngineMetrics(geometry)): number {
  return [...engineRows("あ".repeat(200), metrics, false)[0]].length;
}

/** Lines that fit fully before the surface's bottom edge. */
export function visibleLineCount(geometry: LayoutGeometry): number {
  const room = geometry.height - geometry.textY;
  if (room < geometry.fontPx) return 0;
  return 1 + Math.floor((room - geometry.fontPx) / (geometry.fontPx + geometry.lineSpacing));
}

/** Rectangle of the bottom line that UI art occupies (from the bottom-line cap), or null. */
export function bottomLineUiZone(
  geometry: LayoutGeometry,
  maxLines: number,
  bottomLineCap: number,
): { x: number; y: number; width: number; height: number } | null {
  if (bottomLineCap <= 0 || maxLines < 1) return null;
  const x = geometry.textX + bottomLineCap * (geometry.fontPx / 2);
  const y = geometry.textY + (maxLines - 1) * (geometry.fontPx + geometry.lineSpacing);
  if (x >= geometry.width || y >= geometry.height) return null;
  return { x, y, width: geometry.width - x, height: Math.min(geometry.fontPx, geometry.height - y) };
}

/**
 * Where the speaker name goes: inside the name-plate frame (the art sprite sitting on the
 * box's top edge, narrower than the box), else just above the text surface.
 */
export function speakerNamePosition(
  geometry: LayoutGeometry,
  art: ArtRect[],
  fontPx: number,
): { x: number; y: number } {
  const surfaceX = geometry.x ?? 0;
  const surfaceY = geometry.y ?? 0;
  const plate = art.find(
    (s) => s.dstY <= surfaceY + fontPx && s.dstY + s.height >= surfaceY - fontPx && s.width < geometry.width * 0.7 && s.height <= fontPx * 3,
  );
  if (plate) {
    // Calibrated on Atelial in-game (2026-11-11): the name starts 108 px into the 403×43
    // plate, 3 px below its top; scaled with the plate for other games (unverified).
    return { x: plate.dstX + Math.round((plate.width * 108) / 403), y: plate.dstY + Math.round((plate.height * 3) / 43) };
  }
  return { x: surfaceX + geometry.textX, y: surfaceY - fontPx - 4 };
}

/** Box profile fields the layout needs (ipc.ts BoxProfile). */
export interface ProfileShift {
  liftPx: number;
  growPx: number;
  lineSpacing: number;
}

/**
 * The body box with a text box profile applied (mirrors engine/box_profile.rs
 * patch_system_script + patch_scene_script): text origin moves up by `liftPx` (not past
 * the surface top), the surface grows down by `growPx`, ADV spacing becomes `lineSpacing`.
 */
export function applyBoxProfile(base: LayoutGeometry, profile: ProfileShift): LayoutGeometry {
  return {
    ...base,
    height: base.height + Math.max(0, profile.growPx),
    textY: Math.max(0, base.textY - Math.max(0, profile.liftPx)),
    lineSpacing: Math.max(0, profile.lineSpacing),
  };
}

/** A screen rectangle of box art (subset of ipc.ts BoxArtSprite). */
export interface ArtRect {
  dstX: number;
  dstY: number;
  width: number;
  height: number;
}

/**
 * Bottom-line cap implied by the box art: when the bottom text line (slot maxLines-1)
 * shares its row with UI sprites to the right of the text start (Atelial's Q.SAVE…✕),
 * the half-width characters that fit before the first of them. Full-width art (the box
 * band itself) is not a blocker. Null = nothing in that row.
 */
export function bottomLineCapFromArt(geometry: LayoutGeometry, art: ArtRect[], maxLines: number): number | null {
  if (maxLines < 1) return null;
  const textLeft = (geometry.x ?? 0) + geometry.textX;
  const rowTop = (geometry.y ?? 0) + geometry.textY + (maxLines - 1) * (geometry.fontPx + geometry.lineSpacing);
  const rowBottom = rowTop + geometry.fontPx;
  const blockers = art.filter(
    (s) => s.dstY < rowBottom && s.dstY + s.height > rowTop && s.dstX > textLeft && s.width < geometry.width / 2,
  );
  if (blockers.length === 0) return null;
  const firstX = Math.min(...blockers.map((s) => s.dstX));
  return Math.floor((firstX - textLeft) / (geometry.fontPx / 2));
}
