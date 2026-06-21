// Japanese-aware substring matching. Plain `String.includes` produces false
// positives because Japanese has no word boundaries: the name ベル ("Bell")
// matches inside レベル ("level"), 重要 matches inside 重要性, etc. These helpers
// only count a hit when the term sits at a SCRIPT boundary — i.e. it isn't an
// inner fragment of a longer run of the same script (katakana/hiragana/kanji/
// latin). Heuristic, not a tokenizer, but it kills the common false matches.
//
// Note: indexing is UTF-16 code-unit based, which is correct for the BMP range
// that covers ~all kana + common kanji in VN dialogue. Rare CJK-Ext-B surrogate
// pairs aren't special-cased (not worth it for this domain).

type ScriptClass = "katakana" | "hiragana" | "kanji" | "latin" | "other";

function scriptClass(ch: string): ScriptClass {
  const c = ch.codePointAt(0) ?? 0;
  if ((c >= 0x30a0 && c <= 0x30ff) || (c >= 0xff66 && c <= 0xff9d)) return "katakana"; // full + half width
  if (c >= 0x3040 && c <= 0x309f) return "hiragana";
  if ((c >= 0x4e00 && c <= 0x9fff) || (c >= 0x3400 && c <= 0x4dbf)) return "kanji";
  if (
    (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a) || (c >= 0x30 && c <= 0x39) ||
    (c >= 0xff21 && c <= 0xff3a) || (c >= 0xff41 && c <= 0xff5a) || (c >= 0xff10 && c <= 0xff19)
  ) return "latin";
  return "other";
}

/** A match at [i, i+term.length) is a boundary if neither neighbouring char
 *  continues the same script run as the term's edge characters. */
function isBoundaryAt(text: string, i: number, term: string): boolean {
  const startCls = scriptClass(term[0]);
  const endCls = scriptClass(term[term.length - 1]);
  const before = i > 0 ? text[i - 1] : "";
  const after = i + term.length < text.length ? text[i + term.length] : "";
  const leftOk = before === "" || scriptClass(before) !== startCls;
  const rightOk = after === "" || scriptClass(after) !== endCls;
  return leftOk && rightOk;
}

/** True if `term` is a katakana word — at least one katakana char and no kanji
 *  or hiragana (long-vowel ー, ・, latin, digits are allowed). In these games
 *  that almost always means a proper noun / name. */
export function isKatakanaTerm(term: string): boolean {
  let hasKatakana = false;
  for (const ch of term) {
    const cls = scriptClass(ch);
    if (cls === "kanji" || cls === "hiragana") return false;
    if (cls === "katakana") hasKatakana = true;
  }
  return hasKatakana;
}

/** True if `term` occurs in `text` at a script boundary (not as an inner
 *  fragment of a longer same-script word). */
export function hasBoundaryMatch(text: string, term: string): boolean {
  if (!term) return false;
  let from = 0;
  for (;;) {
    const i = text.indexOf(term, from);
    if (i < 0) return false;
    if (isBoundaryAt(text, i, term)) return true;
    from = i + 1;
  }
}

/** Replace only boundary occurrences of `term` with `repl`, leaving inner
 *  fragments (e.g. ベル inside レベル) untouched. */
export function replaceBoundaryMatches(text: string, term: string, repl: string): string {
  if (!term) return text;
  let out = "";
  let from = 0;
  for (;;) {
    const i = text.indexOf(term, from);
    if (i < 0) return out + text.slice(from);
    if (isBoundaryAt(text, i, term)) {
      out += text.slice(from, i) + repl;
      from = i + term.length;
    } else {
      out += text.slice(from, i + 1);
      from = i + 1;
    }
  }
}
