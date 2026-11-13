// Engine box mode: when the fanpatch loader's English word wrap is on and the tab's game
// has reported its text metrics (fanpatch_layout.txt), the ENGINE breaks rows itself — the
// editor checks whole boxes against the engine's rows (engineOverflow.ts) and splits only
// into boxes, never into 0x6F lines. Otherwise the classic per-line caps apply.
import { store, type TabState } from "./stores.svelte";
import { engineTextLayout, type EngineTextMetrics } from "./ipc";
import { dialogueBoxCaps, engineRows, textCells } from "./gameLayout";
import { findEngineBoxOverflows, type EngineBoxRules } from "./engineOverflow";
import { findBoxOverflows, findOverflows } from "./overflow";

const layoutLookups = new Map<string, Promise<void>>();

/** Look up the game-accurate text layout of a tab once per session (only after its BIN
 *  has loaded — a restored session shows the tab before the backend has the BIN).
 *  Resolves when the lookup is done (await it before deciding engine vs classic mode). */
export function ensureTextLayout(tab: TabState | null | undefined): Promise<void> {
  if (!tab || tab.format === "parente" || !tab.binPath || tab.entries.length === 0) return Promise.resolve();
  const pending = layoutLookups.get(tab.id);
  if (pending) return pending;
  if (tab.id in store.textLayoutByTab) return Promise.resolve();
  const tabId = tab.id;
  store.textLayoutByTab[tabId] = null; // in flight / unavailable until resolved
  const lookup = engineTextLayout(tabId, store.lastSysGameDir)
    .then((layout) => {
      store.textLayoutByTab[tabId] = layout;
    })
    .catch(() => {
      /* no game folder known: simple preview, classic caps */
    })
    .finally(() => layoutLookups.delete(tabId));
  layoutLookups.set(tabId, lookup);
  return lookup;
}

/** The reported metrics when engine box mode applies to `tab`, else null. */
export function engineMetricsFor(tab: TabState | null | undefined): EngineTextMetrics | null {
  if (!tab || !store.fanpatchWordWrap) return null;
  return store.textLayoutByTab[tab.id]?.metrics ?? null;
}

/** Engine box rules for `tab` (rows per box from the game box + enabled profile). */
export function engineBoxRulesFor(tab: TabState | null | undefined): EngineBoxRules | null {
  const metrics = engineMetricsFor(tab);
  const layout = tab ? store.textLayoutByTab[tab.id] : null;
  if (!metrics || !layout) return null;
  const caps = dialogueBoxCaps(layout, layout.gameDir ? store.boxProfiles[layout.gameDir] : undefined);
  return {
    rowsOf: (line) => engineRows(line, metrics, true),
    cellsOf: (row) => textCells(row.trimEnd()),
    rowsPerBox: caps.linesPerBox,
    bottomCap: store.bottomLineCharCap,
  };
}

/** Overflow counts shown for a tab (tab badge, stats, translate panel). Engine mode: boxes
 *  the engine can't show (`box`), no per-line overflows; classic: line + box caps. */
export function tabOverflowCounts(tab: TabState): { line: number; box: number; engine: boolean } {
  const rules = engineBoxRulesFor(tab);
  if (rules) return { line: 0, box: findEngineBoxOverflows(tab.entries, rules).length, engine: true };
  return {
    line: store.translatedLineCharCap > 0 ? findOverflows(tab.entries, store.translatedLineCharCap).length : 0,
    box: findBoxOverflows(tab.entries, store.translatedCharCap, store.maxLinesPerBox, store.bottomLineCharCap).length,
    engine: false,
  };
}
