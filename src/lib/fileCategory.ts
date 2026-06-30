/**
 * File-category helpers for the SYS BIN picker + translation gating.
 *
 * Name/item/init BINs hold character-name and item-name string tables where the
 * `set-string` (0x192) writes ARE the translatable content. In scene/UI BINs the
 * same opcode is the structural speaker slot (the 【 name 】 triplet) and must
 * stay protected — editing it corrupts the speaker rendering (e.g. produces
 * "【】 Liselle" instead of "【Liselle】"). These helpers tell the two apart by
 * filename so the editor + export logic only unlock `set-string` where it's safe.
 */

/** Filename prefixes (case-insensitive, append-prefix stripped) that mark a BIN
 *  as a character/item NAME table where `set-string` is editable content. */
export const NAME_ITEM_PREFIXES = [
  "CVINIT",
  "CIINIT",
  "CNINIT",
  "CHINIT",
  "AFINIT",
  "ITINIT",
  "ITMES",
] as const;

/** Reduce a path/filename to its bare logical basename: drop any directory, then
 *  strip a leading Eushully update-layer prefix (`$1$NAME.BIN` → `NAME.BIN`) so
 *  patched/append copies classify the same as their base game BIN. */
function logicalBaseName(filename: string): string {
  const base = filename.replace(/\\/g, "/").split("/").pop() ?? filename;
  const m = base.match(/^\$\d+\$(.*)$/);
  return m ? m[1] : base;
}

/** True when the BIN is a character/item NAME table (CVINIT/CIINIT/CNINIT/CHINIT/
 *  AFINIT/ITINIT/ITMES). In these files `set-string` is editable, translatable
 *  content; everywhere else it's the protected structural speaker slot. */
export function isNameItemFile(filename: string): boolean {
  if (!filename) return false;
  const upper = logicalBaseName(filename).toUpperCase();
  return NAME_ITEM_PREFIXES.some((p) => upper.startsWith(p));
}

/** Filename prefixes (case-insensitive, append-prefix stripped) that mark a BIN
 *  as menu/system UI text. Mirrors the `UI_PREFIXES` list in commands.rs
 *  `classify_bin` so the frontend's UI classification matches the Rust picker. */
const UI_PREFIXES = [
  "MENU",
  "INFO",
  "BTLMENU",
  "CGMODE",
  "CHMODE",
  "SAVE",
  "FIELD",
  "GAMESTART",
  "CHANGECH",
  "HOSOKU",
] as const;

/** True when the BIN is a menu/system UI file (any `SYS*`, or one of the
 *  MENU/INFO/BTLMENU/… prefixes). Mirrors the `ui` bucket in commands.rs
 *  `classify_bin`. Stat-screen abbreviations (ATK/DEF/HP…) are CORRECT here, so
 *  these files are exempt from the dialogue abbreviation check. */
export function isUiFile(filename: string): boolean {
  if (!filename) return false;
  const upper = logicalBaseName(filename).toUpperCase();
  return upper.startsWith("SYS") || UI_PREFIXES.some((p) => upper.startsWith(p));
}

/** A "dialogue bin" is any open tab that is NEITHER a name/item table NOR a UI
 *  file. Prose is wanted here, so UI abbreviations leaking in are suspect — this
 *  is the gate for both the abbreviation check and the glossary pre-fill skip. */
export function isDialogueBin(filename: string): boolean {
  return !isNameItemFile(filename) && !isUiFile(filename);
}

/** Common UI/stat-screen abbreviations (uppercase). Glossary entries mapping a
 *  JP stat term to one of these (攻撃力 → "ATK") are meant for menu screens; when
 *  the same entry pre-fills SCENE dialogue it leaks an abbreviation into prose. */
export const UI_ABBREVIATIONS: ReadonlySet<string> = new Set([
  "ATK", "DEF", "HP", "MP", "SP", "TP", "EXP", "LV", "LVL", "AGI", "INT",
  "STR", "VIT", "DEX", "LUK", "MAG", "MDF", "RES", "CRT", "HIT", "EVA",
  "SPD", "ATB", "AP", "CP", "EX",
]);

/**
 * Heuristic: does an English glossary rendering LOOK like a UI abbreviation
 * (no manual tagging required)? True when the trimmed value is either:
 *   - in {@link UI_ABBREVIATIONS}, OR
 *   - an all-caps token ≤4 chars (`/^[A-Z][A-Z0-9]{0,3}$/`, e.g. "MDEF").
 * All-caps is the reliable signal — lowercase short words ("yen", "Mr", "Dr")
 * are prose, not UI abbreviations, so they're deliberately NOT matched (avoids
 * flagging every "yen" in dialogue). If you want a mixed-case abbreviation
 * caught, add it to {@link UI_ABBREVIATIONS}.
 * Used to (1) seed the dialogue abbreviation check's term set from the glossary,
 * and (2) skip pre-filling such entries into scene translations.
 */
export function isUiAbbreviation(english: string): boolean {
  const v = english.trim();
  if (!v) return false;
  if (UI_ABBREVIATIONS.has(v.toUpperCase())) return true;
  if (/^[A-Z][A-Z0-9]{0,3}$/.test(v)) return true;
  return false;
}
