# Bilingual language swap

The loader can flip the whole game between **English and the original Japanese live**, in‑game, with a single key — great for comparing the translation against the source or playing in either language.

## How to ship it

Use **Export ▸ Bilingual (EN↔JP switch) → .fanpatch**. You'll pick, in order:

1. **English BIN folder** (your translation),
2. **Japanese BIN folder** (the originals), and
3. *(optional)* a **translated‑asset (AGF) folder** — cancel this dialog to skip.

The editor builds **bilingual scene BINs** (each dialogue beat carries both languages behind a flag the loader flips) and bundles them with the loader.

## In‑game

- Press the **backtick** key (`` ` ``) to toggle language. Dialogue swaps **instantly, mid‑scene**.
- The title bar shows the current language: `… [Fanpatch - English]` / `… [Fanpatch - 日本語]`.
- **UI / CG images** (bundled AGFs) swap on their next load (re‑open the menu / next screen) — images can't repaint until redrawn.
- The **title screen and menus** flip on the spot (the loader refreshes non‑scene screens on toggle).

## Repair‑only (speaker boxes)

**Export ▸ Repair speaker boxes → .fanpatch** runs the same alignment to fix continuation boxes that lost their speaker name/portrait/colour (common when long translations were split into extra boxes), without the language conditional. Confirmed across hundreds of boxes on Sankai.

## Notes & limits

- Needs both EN and JP versions of the scenes to align them.
- Some names are drawn by the engine from boot data and stay in one language even when dialogue swaps — that's expected (they're identity labels).
- The bilingual build is round‑trip‑guarded; files that can't align cleanly are shipped as plain translations instead of failing the whole export.
