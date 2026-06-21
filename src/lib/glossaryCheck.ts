import type { DialogueEntry, GlossaryEntry } from "./types";

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
}

const CHECKABLE = new Set(["show-text", "display-furigana", "comment"]);

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
  const terms = glossary
    .map((g) => ({ jp: g.japanese, en: g.english.trim() }))
    .filter((g) => g.jp && g.en);
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
      if (!original.includes(jp)) continue;
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
