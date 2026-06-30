import type { DialogueEntry, GlossaryEntry } from "./types";
import { hasBoundaryMatch } from "./jpMatch";
import { UI_ABBREVIATIONS, isUiAbbreviation, isDialogueBin } from "./fileCategory";

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

/**
 * The full translated text of a beat starting at entry `i`: the entry's own
 * translation plus any following continuation lines (inserted split lines have an
 * EMPTY original — no JP source). A single Japanese line is often translated into
 * a longer English run that auto-split spreads across several rows, so the term's
 * English may land in a continuation, not the primary. Matching the term against
 * this combined text avoids false "term missing" flags on split lines. Gathering
 * stops at the next row that has its own JP original (the next beat).
 */
function combinedTranslation(entries: DialogueEntry[], i: number): string {
  let combined = entries[i].translated_text ?? "";
  for (let j = i + 1; j < entries.length; j++) {
    const e = entries[j];
    if (e.deleted) continue;
    if (e.original_text && e.original_text.trim() !== "") break; // next beat
    if (CHECKABLE.has(e.label) && e.translated_text) {
      combined += " " + e.translated_text;
    }
  }
  return combined;
}

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
    if (!original) continue; // only rows that carry a JP source can hold a term
    // Match against the whole beat (primary + its split continuation lines), not
    // just this row, so a term rendered in a continuation isn't false-flagged.
    const translated = combinedTranslation(entries, i);
    if (translated.trim() === "") continue; // only translated rows

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

/* ─────────────────────────── UI-abbreviation leak check ──────────────────────
 * UI glossary entries (攻撃力 → "ATK", 防御力 → "DEF") are global, so the
 * abbreviation can leak into SCENE dialogue where prose is wanted. This pass
 * scans dialogue BINs for those abbreviations and flags them as suggestions —
 * advisory only, no auto-change. */

/** One dialogue line where a UI-style abbreviation appears in the translation. */
export interface AbbreviationHit {
  /** Source BIN filename (tab.sourceBinFilename) — the grouping key in the UI. */
  file: string;
  tabId: string;
  tabIndex: number;
  /** Array index in tab.entries (jump fallback). */
  rowIndex: number;
  /** instruction_index (stable id used for jump-to-row). */
  instruction_index: number;
  /** The matched abbreviation as it appears in the line (e.g. "ATK"). */
  term: string;
  /** The full translated line the term was found in. */
  snippet: string;
}

/** Labels whose `translated_text` is rendered dialogue prose to the player. */
const ABBREV_LABELS = new Set(["show-text", "display-furigana"]);

/**
 * The abbreviation term set: the built-in {@link UI_ABBREVIATIONS} plus any
 * glossary English renderings (global + each tab's local) that LOOK like a UI
 * abbreviation per {@link isUiAbbreviation}. Terms keep their original casing so
 * matching can stay case-sensitive (a leaked "ATK" is uppercase, "atk" in prose
 * — e.g. inside a word — should not match).
 */
export function buildAbbreviationTerms(
  global: GlossaryEntry[],
  locals: GlossaryEntry[][],
): string[] {
  const set = new Set<string>(UI_ABBREVIATIONS);
  for (const list of [global, ...locals]) {
    for (const g of list) {
      const en = g.english.trim();
      if (en && isUiAbbreviation(en)) set.add(en);
    }
  }
  return [...set];
}

/** One case-sensitive, whole-word alternation regex for all terms (longest-first
 *  so "EXP" wins over "EX"). Returns null when there are no terms. */
function buildAbbreviationRegex(terms: string[]): RegExp | null {
  const unique = [...new Set(terms.filter((t) => t.length > 0))];
  if (unique.length === 0) return null;
  unique.sort((a, b) => b.length - a.length);
  const escaped = unique.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  return new RegExp(`\\b(${escaped.join("|")})\\b`, "g");
}

/**
 * Scan every (open) dialogue BIN for UI abbreviations in translated dialogue.
 * Only tabs where {@link isDialogueBin} holds are scanned — UI and name files are
 * exempt (abbreviations are correct there). For each `show-text`/`display-furigana`
 * row with a non-empty translation, a whole-word, case-sensitive match against the
 * term set yields one hit per (row, term). Results are advisory.
 */
export function findUiAbbreviationsInDialogue(
  tabs: TabGlossaryInput[],
  globalGlossary: GlossaryEntry[],
): AbbreviationHit[] {
  const terms = buildAbbreviationTerms(globalGlossary, tabs.map((t) => t.glossary));
  const re = buildAbbreviationRegex(terms);
  if (!re) return [];

  const hits: AbbreviationHit[] = [];
  for (const t of tabs) {
    if (!isDialogueBin(t.name)) continue;
    for (let i = 0; i < t.entries.length; i++) {
      const e = t.entries[i];
      if (e.deleted || e.is_context_row) continue;
      if (!ABBREV_LABELS.has(e.label)) continue;
      const text = e.translated_text;
      if (!text || text.trim() === "") continue;

      re.lastIndex = 0;
      const seenTerms = new Set<string>();
      let m: RegExpExecArray | null;
      while ((m = re.exec(text)) !== null) {
        const term = m[1];
        if (seenTerms.has(term)) continue; // one hit per term per line
        seenTerms.add(term);
        hits.push({
          file: t.name,
          tabId: t.id,
          tabIndex: t.index,
          rowIndex: i,
          instruction_index: e.instruction_index,
          term,
          snippet: text,
        });
      }
    }
  }
  return hits;
}
