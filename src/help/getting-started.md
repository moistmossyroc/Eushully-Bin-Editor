# Getting Started

**Eushully BIN Editor** translates Eushully / AGE‑engine visual novels by editing their `.BIN` script files and packaging the result as a `.fanpatch` the game loads at runtime — no modification of the original game files required.

## The core loop

1. **Load** a `.BIN`, or a whole folder of them — **Open** menu. The badge in the top‑right confirms the detected game and engine build (e.g. *Madou Koukaku · SYS4 (v443)*).
2. **Translate** the Japanese dialogue — type directly into the table, use the live AI translator, or the offline **CC Translate** workflow.
3. **Auto‑split** lines that overflow the in‑game text box (optional — the editor word‑wraps and paginates for you).
4. **Export** a `.fanpatch` (**Export** menu), then drop the produced `.fanpatch` **and** `version.dll` into the game folder.

That's it — launch the game and your translation is live.

## Projects keep your work safe

Your edits live in a per‑BIN **project file**, not the BIN itself. The original `.BIN` is only read; the translated result is written out at export. So you can always re‑export, and you never risk corrupting the source.

- **Save**: writes the current tab's project. Autosave runs in the background.
- **Backups**: each save keeps timestamped backups you can diff/restore.

## Tips for newcomers

- The **?** button (top‑right) or **F1** opens this Help Center any time.
- Many features have a small **?** next to them — click it to jump straight to the relevant help.
- Work one BIN at a time, or open a folder and tab between them.
