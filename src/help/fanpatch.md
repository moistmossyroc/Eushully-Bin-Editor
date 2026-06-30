# Fanpatch export & the in‑game loader

Translations ship as a **`.fanpatch`** served by a small **`version.dll`** loader that sits next to the game EXE. The loader intercepts the game's file reads and serves your translated BINs in place of the originals — the base game files are never touched.

## Exporting

From the **Export** menu:

- **Single BIN → .fanpatch** — package the current file.
- **From a folder** — package a whole folder of translated BINs at once.

The export writes:

- `01_english.fanpatch` — your translated files (a renamed ZIP; flat, uppercase names).
- `version.dll` — the loader (bundled with the editor).
- `README` + `fanpatch_title.txt` — install notes and the optional title config.

## Installing (what the player does)

1. Copy **`version.dll`** and the **`.fanpatch`** into the game's install folder (next to the game `.exe`).
2. Launch the game. The translation is live.

To uninstall, delete `version.dll` (and the `.fanpatch`). The original game is untouched.

## Title‑bar config (`fanpatch_title.txt`)

Optional. Put it next to the EXE to label the window:

```
title = Madou Koukaku
label = Fanpatch v1.0
```

→ the title bar reads `Madou Koukaku [Fanpatch v1.0 - English]`. The `label` is great for showing a **version number**, so players can tell which patch build they're on from a screenshot. Omit the file to keep the game's own title.

## Why a loader (not `.alf` appends)

`.alf` appends can only *add* files; they can't reliably *override* a base scene. The loader serves your files with top priority, so it cleanly replaces dialogue, UI, anything — which is also what makes the **language swap** possible (see *Bilingual language swap*).
