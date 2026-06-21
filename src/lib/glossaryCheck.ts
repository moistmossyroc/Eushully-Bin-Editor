import type { DialogueEntry, GlossaryEntry } from "./types";
import { hasBoundaryMatch } from "./jpMatch";

/** One row where a glossary term's Japanese appears in the original but its
 *  English rendering is missing from the (non-empty) translation. */
export interface GlossaryFlag {
  /** Array index in tab.entries (for jump-to-row). */
  rowIndex: number;
  /** instruction_index (stable id). */
  rowId: number;
  /** Glossary Japanese term found in the original. */
  term: string;
  /** Glossary English rendering that's missing from the translation. */
  glossaryEn: string;
  original: string;
  translated: string;
  /** Owning tab — set only by the all-tabs variant. */
  tabId?: string;
  tabName?: string;
  tabIndex?: number;
}

/** Minimal per-tab input for the all-tabs scan (decouples lib from the store). */
export interface TabGlossaryInput {
  id: string;
  name: string;
  index: number;
  entries: DialogueEntry[];
  /** Tab-local glossary (merged with the global one, global winning ties). */
  glossary: GlossaryEntry[];
}

const CHECKABLE = new Set(["show-text", "display-furigana", "comment"]);

/** Global + tab-local glossary, deduped by `japanese` (global wins). Mirrors
 *  the store's `mergedGlossary` getter so per-tab scans match what the user sees. */
export function mergeGlossary(global: GlossaryEntry[], local: GlossaryEntry[]): GlossaryEntry[] {
  const seen = new Set(global.map((g) => g.japanese));
  const out = [...global];
  for (const g of local) {
    if (!seen.has(g.japanese)) {
      out.push(g);
      seen.add(g.japanese);
    }
  }
  return out;
}

/**
 * Glossary consistency check: for every glossary entry with a non-empty English
 * rendering, find translated rows whose ORIGINAL contains the term's Japanese
 * but whose TRANSLATION does NOT contain the English (case-insensitive). These
 * are surfaced as review flags — not errors, since a term is sometimes
 * legitimately rephrased — so the translator can confirm or fix drift.
 *
 * One flag per (row, term): a row can flag multiple terms.
 */
export function findGlossaryInconsistencies(
  entries: DialogueEntry[],
  glossary: GlossaryEntry[],
): GlossaryFlag[] {
  // Dedup terms by Japanese (the merged glossary can contain the same JP twice)
  // so a row never produces duplicate flags for one term.
  const seenJp = new Set<string>();
  const terms: { jp: string; en: string }[] = [];
  for (const g of glossary) {
    const jp = g.japanese;
    const en = g.english.trim();
    if (!jp || !en || seenJp.has(jp)) continue;
    seenJp.add(jp);
    terms.push({ jp, en });
  }
  if (terms.length === 0) return [];

  const flags: GlossaryFlag[] = [];
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i];
    if (e.deleted || e.is_context_row) continue;
    if (!CHECKABLE.has(e.label)) continue;
    const original = e.original_text;
    const translated = e.translated_text;
    if (!original || translated.trim() === "") continue; // only translated rows

    const transLc = translated.toLowerCase();
    for (const { jp, en } of terms) {
      // Boundary match, not raw substring — avoids flagging e.g. ベル ("Bell")
      // when it's only an inner fragment of レベル ("level").
      if (!hasBoundaryMatch(original, jp)) continue;
      if (transLc.includes(en.toLowerCase())) continue; // term rendered — OK
      flags.push({
        rowIndex: i,
        rowId: e.instruction_index,
        term: jp,
        glossaryEn: en,
        original,
        translated,
      });
    }
  }
  return flags;
}

/**
 * Run the consistency check across EVERY open tab. Each tab is checked against
 * its own merged glossary (global + that tab's local), and every flag is tagged
 * with its owning tab so the UI can group results and jump across tabs.
 */
export function findGlossaryInconsistenciesAllTabs(
  tabs: TabGlossaryInput[],
  globalGlossary: GlossaryEntry[],
): GlossaryFlag[] {
  const out: GlossaryFlag[] = [];
  for (const t of tabs) {
    const glossary = mergeGlossary(globalGlossary, t.glossary);
    for (const f of findGlossaryInconsistencies(t.entries, glossary)) {
      out.push({ ...f, tabId: t.id, tabName: t.name, tabIndex: t.index });
    }
  }
  return out;
}
