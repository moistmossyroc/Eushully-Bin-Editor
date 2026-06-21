import type { DialogueEntry, GlossaryEntry } from "./types";

export type AutoApplyKind = "fill" | "replace";

/** One proposed glossary substitution — deterministic and reviewable. */
export interface GlossaryAutoChange {
  /** Array index in tab.entries (for jump-to-row). */
  rowIndex: number;
  /** instruction_index (stable id, re-found before apply). */
  rowId: number;
  /** Glossary Japanese term(s) involved (comma-joined when more than one). */
  term: string;
  /** Glossary English rendering(s) (comma-joined to match `term`). */
  glossaryEn: string;
  kind: AutoApplyKind;
  /** Current translated_text. */
  before: string;
  /** Proposed translated_text. */
  after: string;
}

const CHECKABLE = new Set(["show-text", "display-furigana", "comment"]);

/**
 * Find SAFE, deterministic glossary substitutions that can be auto-applied:
 *
 *  - "fill":    an *untranslated* row whose ORIGINAL is exactly a glossary term
 *               → fill the translation with the canonical English.
 *  - "replace": a translated row where the Japanese term leaked verbatim into
 *               the English → replace that exact substring with the English
 *               (chains every leaked term in the row into one change).
 *
 * Both are exact-match operations — there is no guessing where a term belongs in
 * free text (that stays in the consistency CHECK, which only flags). Every change
 * is returned for review; nothing is mutated here.
 */
export function findGlossaryAutoApply(
  entries: DialogueEntry[],
  glossary: GlossaryEntry[],
): GlossaryAutoChange[] {
  const terms = glossary
    .map((g) => ({ jp: g.japanese, en: g.english.trim() }))
    .filter((g) => g.jp && g.en && g.jp !== g.en);
  if (terms.length === 0) return [];

  const changes: GlossaryAutoChange[] = [];
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i];
    if (e.deleted || e.is_context_row) continue;
    if (!CHECKABLE.has(e.label)) continue;
    const original = (e.original_text ?? "").trim();
    const translated = e.translated_text ?? "";

    if (translated.trim() === "") {
      // fill: the whole original is exactly one glossary term.
      const hit = terms.find((t) => original === t.jp);
      if (hit) {
        changes.push({
          rowIndex: i,
          rowId: e.instruction_index,
          term: hit.jp,
          glossaryEn: hit.en,
          kind: "fill",
          before: translated,
          after: hit.en,
        });
      }
      continue;
    }

    // replace: any glossary term whose Japanese leaked verbatim into the English.
    let after = translated;
    const usedJp: string[] = [];
    const usedEn: string[] = [];
    for (const { jp, en } of terms) {
      if (after.includes(jp)) {
        after = after.split(jp).join(en);
        usedJp.push(jp);
        usedEn.push(en);
      }
    }
    if (usedJp.length > 0 && after !== translated) {
      changes.push({
        rowIndex: i,
        rowId: e.instruction_index,
        term: usedJp.join(", "),
        glossaryEn: usedEn.join(", "),
        kind: "replace",
        before: translated,
        after,
      });
    }
  }
  return changes;
}
