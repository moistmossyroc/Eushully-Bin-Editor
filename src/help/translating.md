# Translating

There are three ways to translate, and you can mix them freely on the same file.

## 1. Manual editing

Type your translation into the **English** column of any dialogue row. The original Japanese stays in view for reference. Rows that aren't dialogue (control codes, structural entries) are marked non‑editable so you don't break the script.

- `display‑furigana` rows are folded into their line as a `[reading: …]` annotation.
- Speaker name slots are protected — never blank them (it corrupts the name box).

## 2. Live AI translation (online)

With an API key set (**Settings**), the editor can translate via Claude:

- **Translate one entry**, or **Start** a full run that batches by dialogue block and feeds prior‑block context for consistency.
- Pause / resume / stop at any time.
- Glossary terms are pre‑applied so names and key terms stay consistent.

This path needs network + an API key and bills to your account.

## 3. Offline CC Translate (recommended for bulk)

Exports a self‑contained folder you translate with Claude Code locally — no API key in the editor, full project context, resumable. See the **CC Translate** help section.

## Settings that affect translation

- **Per‑line / per‑box character caps** — drive overflow detection and auto‑split.
- **Max lines per box** — pagination target.
- **Model / provider** — for the live AI path.

## Saving & exporting

Translations are saved into the project (autosave + manual Save). When you're ready to play‑test, **Export** a `.fanpatch`. The export re‑parses its own output and aborts if it wouldn't round‑trip, so a bad export can never overwrite a good BIN.
