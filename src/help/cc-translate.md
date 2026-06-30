# CC Translate (offline workflow)

CC Translate is the recommended way to translate in bulk. It exports a self‑contained folder that you translate with **Claude Code** running locally — no API key in the editor, full file context, and it's resumable.

## Export the folder

From the **CC Translate** menu, export the current BIN (or a batch of BINs). Each folder contains:

- **`translation.json`** — the rows to translate (editable entries only; numbers/symbols and structural rows are skipped).
- **`PROMPT.md`** — the translation rules + your glossary + cross‑BIN context.
- **`translate.py`** — the helper CLI: `status`, `blocks-pending`, `block N`, `apply mapping.json`, `list-pending`.
- **`translate.bat` / `.sh`** — one‑click launcher (pre‑authorised allowlist).

A multi‑BIN export also writes, at the parent folder, a `BATCH‑PROMPT.md` and `translate‑all` / `translate‑parallel` launchers for doing every subfolder in one session (sequential) or N at once (headless parallel, resumable).

## The workflow (one block per turn)

1. Run the launcher (or `claude` in the folder).
2. Get a block: `python translate.py block N`.
3. Translate it, write `tl-out.json`, then `python translate.py apply tl-out.json`.
4. Repeat until `status` reports `N/N translated`.

**Why one block per turn:** batching many blocks blows past the 32K output limit. The PROMPT.md enforces this; trust `untranslated_blocks()` rather than dumping the whole file.

## Bring translations back

**Import translations** in the editor reads the completed folder back into your project, distributing duplicates and applying glossary terms. Optionally auto‑splits overflowing rows on import.

## Why it's nice

- No per‑entry API round‑trips — translate at your own pace, offline‑resumable.
- The whole file is in context, so the model keeps tone and terminology consistent.
- Duplicate lines are translated once and reused.
