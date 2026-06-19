# Eushully BIN Editor

A desktop tool for **translating Japanese visual-novel scripts from Eushully games**. It loads the game's script files, shows the dialogue in an editable table, helps you (or an LLM) translate it, fits the text to the in-game text boxes, and exports patched files that play in-game.

It supports two script formats:

* **Classic AGE engine `.BIN`** (SYS3 / SYS4 / SYS5) — most Eushully titles.
* **Unity-era Lua scenes** packed in `.arc` archives (e.g. *Yamiyo no Parente*).

> ⚠️ **This tool ships no game data.** It contains no scripts, assets, or other content from any game. It only reads and writes files **you already own**. You must obtain the original game and any official patches yourself, legally. This is an unofficial, fan-made tool and is **not affiliated with or endorsed by Eushully**.

---

## Table of contents

- [Features at a glance](#features-at-a-glance)
- [Installing / building](#installing--building)
- [Quick start](#quick-start)
- [The editor](#the-editor)
- [Translating](#translating)
  - [Offline: CC Translate (recommended)](#offline-cc-translate-recommended)
  - [Live API translation](#live-api-translation)
- [Glossary](#glossary)
- [Character / speaker names](#character--speaker-names)
- [Fitting text to the box: character limits & auto-split](#fitting-text-to-the-box-character-limits--auto-split)
- [Proofreading & length optimization](#proofreading--length-optimization)
- [Exporting](#exporting)
- [Unity-era games (Parente)](#unity-era-games-parente)
- [Settings reference](#settings-reference)
- [Keyboard shortcuts & gestures](#keyboard-shortcuts--gestures)
- [Tips & troubleshooting](#tips--troubleshooting)
- [Credits](#credits)
- [License](#license)

---

## Features at a glance

**Editing**
* Multi-tab editor — work on many scenes at once; drag to reorder, scroll-wheel to cycle.
* Editable table of every translatable line, with speaker labels and in-game block ordering.
* Raw **BIN viewer** for inspecting the underlying instruction stream.
* **Global search** across all open tabs; cross-tab **progress dashboard**.
* Per-tab autosave with **backup history** (restore the last few versions), and optional **session restore** on launch.
* Project files (`.json`) preserve all progress, glossary, and character names.

**Translation**
* **CC Translate** — a fully offline pipeline that hands the script to Claude Code (or any agent) via a generated `PROMPT.md` + `translate.py`, including batch and parallel launchers for whole games.
* **Live API** translation through Anthropic (Claude) or any OpenAI-compatible endpoint, with block-mode batching, context windows, retries, rate-limit handling, and Anthropic prompt caching.
* Per-project glossary + a shared **global glossary**, with katakana-name suggestion and category filtering.
* **Speaker-name resolution** from init BINs (CVINIT/CIINIT/CNINIT/CHINIT) and per-scene tables, with manual override and a bulk resolver.

**In-game fit**
* **Auto-split**: word-aware wrapping to your per-line width, with line breaks inserted as real engine opcodes, and continuation **text boxes** added automatically where the script supports it.
* **Overflow Review** dialog to find and fix every over-long line/box; optional **auto-split on import**.
* Inline **proofreading** with accept/reject diffs and per-line length optimization.

**Unity-era (Parente)**
* Drag in the game folder; the tool resolves the latest version of every dialogue file across all archives.
* Non-destructive export into the engine's **secondary language slot** (Japanese stays intact, with JP fallback for untranslated lines).
* Enables the engine's own dormant **in-game Japanese/English toggle**, which the game saves across restarts.

---

## Installing / building

There is no bundled binary in the source tree — build it once, or grab a prebuilt installer from the project's GitHub **Releases** page if one is attached.

**Requirements**

* **Rust** (stable) — install via [rustup](https://rustup.rs/) (`rustup toolchain install stable`). Windows uses the MSVC target; WebView2 ships with Windows 10/11. Linux: see the [Tauri Linux prerequisites](https://tauri.app/start/prerequisites/#linux). macOS: the Xcode Command Line Tools.
* **Node.js 22+** — [fnm](https://github.com/Schniz/fnm) or [nvm](https://github.com/nvm-sh/nvm) recommended.
* The Tauri CLI is a local dev dependency (run via `npm run tauri`); no global install needed.

**Build**

```bash
npm install            # install JS deps
npm run tauri dev      # development (hot reload)
npm run tauri build    # release binary + installer
```

Output lands in `src-tauri/target/release/` and `src-tauri/target/release/bundle/` (MSI + NSIS on Windows).

---

## Quick start

1. **Open** a `.BIN` (toolbar → *Open Files…*, or drag it onto the window).
2. Translate in the table — type directly, or use **CC Translate** / **Translate** (live API).
3. Set your game's **character limits** (Settings → Editor) and run **auto-split** so text fits the boxes.
4. **Export BIN** (toolbar) to write the patched file. Your progress also saves to a project `.json`.

For a whole game, open many BINs at once (*Open Project Folders…*), translate with the CC Translate **batch** launchers, then **Import All**.

---

## The editor

Each row is one translatable element from the script (dialogue line, furigana, comment). Columns:

* **Function call** — the opcode/label, the in-source index, and the resolved **speaker name** badge.
* **Original text** — the Japanese (furigana kanji are shown as `kanji〔reading〕`).
* **Translated text** — your English; character counts and over-cap highlights show here.
* **Notes** — optional translator notes; per-row **R/E/F** toggles and an **Export** checkbox to control what gets written back.

Side panels (toolbar toggles): **Characters/Speakers**, **Glossary**, **Global Search**, **Stats** (cross-tab progress). The tab **status dot** shows progress (red → amber → green; an amber ring means some lines exceed the per-line cap). Click a red/amber dot to jump to untranslated rows.

---

## Translating

### Offline: CC Translate (recommended)

This path needs no API key and no network from the editor — it generates a folder you hand to **Claude Code** (or any capable coding agent), which fills in the translations; you then import them back.

**Single BIN**

1. Toolbar → **CC Translate ▸ Export for Translation…**, choose an output folder.
2. The folder contains:
   * `translation.json` — the rows to translate (with speaker, block grouping, and a per-entry length budget).
   * `PROMPT.md` — full instructions, translation rules, glossary reference, and the **length/line-break rules tailored to this BIN** (see below).
   * `glossary.json` — your terms, auto-injected per block by the helper.
   * `translate.py` — helper CLI (`status`, `blocks-pending`, `block N`, `apply mapping.json`, `list-pending`).
   * `translate.bat` / `translate.sh` — double-click launcher that starts Claude Code in that folder.
   * `.claude/settings.local.json` — a narrow pre-authorized allowlist so the run is fire-and-forget.
3. Run the launcher (or `cd` in and run Claude Code). It translates **one block per turn**, saving as it goes.
4. Back in the editor: **CC Translate ▸ Import Translations…** and pick the `translation.json`.

**Whole game (batch)**

* **CC Translate ▸ Export All Tabs for Translation…** writes one subfolder per BIN plus, at the parent:
  * `BATCH-PROMPT.md` + `translate-all.bat`/`.sh` — sequential single-session run over every subfolder.
  * `translate-parallel.bat`/`.sh` + `translate-parallel.py` — headless parallel run (N concurrent), with a per-folder log and resume support (completed folders are skipped).
* When done, **CC Translate ▸ Import All Translations…** and point at the parent folder — each subfolder is matched to its tab by content hash.
* **CC Translate ▸ Check Translation Status…** scans a parent folder and reports per-file progress.

**Budget-aware prompts.** The exporter inspects each BIN and writes the right length rule into its `PROMPT.md`:

* If the script can add extra text boxes → **no length cap** ("translate fully; the tool wraps and adds boxes").
* If it can't add boxes but can wrap → **one box** (`lineCap × maxLines`, e.g. 150).
* If it can do neither → **one line** (`lineCap`).

In every case the prompt tells the model **not to insert line breaks** — the editor owns wrapping (see [auto-split](#fitting-text-to-the-box-character-limits--auto-split)).

### Live API translation

Toolbar → **Translate**. Configure a provider and key in **Settings ▸ Provider** (Anthropic, or any OpenAI-compatible endpoint). The key is stored in your **OS keychain** (Windows Credential Manager / macOS Keychain / Linux libsecret) — never written to disk or compiled in.

Live mode batches by dialogue **block** (a run between input-waits), can include prior-block context, folds furigana into the preceding line, applies your glossary, retries on errors, and (Anthropic) uses prompt caching to cut cost. Tune all of this in **Settings ▸ Translation**.

---

## Glossary

Keep names and recurring terms consistent. Open the **Glossary** panel:

* **Per-project** entries live with the BIN's project; the **global glossary** is shared across everything and applies to all tabs.
* **Suggest** — scans for katakana names not yet in the glossary; one click to add.
* **Category pills** — filter to Character / Location / Item / Skill / Title / General.
* **Save to folder** — writes `glossary.json` next to the BIN; it auto-loads next time you open any BIN from that folder.
* Exact glossary matches are auto-filled into translations and injected into LLM prompts (per-block, to keep token cost low).

---

## Character / speaker names

Open the **Characters / Speakers** panel. It lists every speaker ID referenced by the scene and the resolved name (or *Unknown*).

* **Scan BIN for Names** — pull names from an init BIN.
* Names resolve automatically from the scene's folder via **CVINIT / CIINIT / CNINIT / CHINIT** (the tool reconciles differing layouts between games).
* **🔍** (shown on unresolved rows) — pick a name from any init BIN.
* You can **type a name directly** into any row, including unresolved IDs — useful for minor/runtime-only characters that no table defines. It persists with the project.
* **Bulk Resolve** (shown when ≥2 tabs are open) — resolve unknown names across all open tabs in one pass.

---

## Fitting text to the box: character limits & auto-split

Eushully boxes have a fixed width and a maximum number of lines, and the engine wraps Japanese **character-by-character** — which mangles English mid-word. So you set your game's limits and let the tool insert clean, word-aware breaks.

**1. Find your game's limits** (they differ per title — confirm in-game): the maximum **characters per line** and **lines per box**. (Example: one SYS4 title fits 50 chars/line × 3 lines; quote brackets `「」` don't count for character dialogue.)

**2. Set them** in **Settings ▸ Editor**: *per-line character cap* and *max lines per box*.

**3. Auto-split** wraps each translation to that width with **word-aware** breaks (never mid-word), inserting real `end-text-line` opcodes, and — where the script supports it — adds **continuation text boxes** for anything longer than one box.

* The **auto-split badge** shows capability per file: *ready* / *ready (degraded)* / *line-only*. "Line-only" means the file can wrap within a box but can't grow into extra boxes; long text there must be shortened.
* **Overflow Review** (opens automatically after import when something is over cap, or via the status dot's amber ring) lists every over-long line/box. **Split all** fixes them; per-row **Split** handles one. Rows that can't fit are flagged rather than silently clipped.
* **Auto-split on import** (Settings ▸ Editor, default off) runs the bulk split automatically on every BIN as you import — handy for whole-game imports so you don't split each file by hand.

---

## Proofreading & length optimization

* **Proofread** produces an inline **diff** you can accept or reject per slot.
* Per-row **length optimization** asks the model to tighten a specific over-cap line to fit, preserving meaning.

---

## Exporting

* **Export BIN** (toolbar) writes the patched `.BIN`. Replace the original in your game (keep a backup).
* Your work also saves to a **project `.json`** (translations, glossary, character names, omitted cells) — reopen it later to continue. The exporter preserves instruction offsets and control-flow targets so patched files load correctly.

---

## Unity-era games (Parente)

For Unity-era titles (e.g. *Yamiyo no Parente*) whose scripts live in `.arc` archives:

1. **Open Files ▸ Open Game Folder… (Parente)** and point at the game's `Start`/`Archives` folder. The tool resolves the latest version of every dialogue file across all archives into one list.
2. Pick scenes in the **categorized picker** (Scenes / System-UI / Sequence / Test), with quick-select by type.
3. Translate as usual (speaker labels and in-game block order are shown; progress auto-saves and restores).
4. **Export** produces a single drop-in **patch archive**. The status bar tells you where to put it and which official patch files to disable.

**How the patch works:** English is written into the engine's **secondary language slot**, so the original Japanese is untouched and untranslated lines fall back to Japanese automatically. The patch also enables the engine's own dormant **Japanese/English toggle** in the Config menu — the player's choice is saved by the game and **persists across restarts**.

---

## Settings reference

**Editor**
* **Per-line character cap** — max characters per rendered line (drives overflow detection and auto-split wrapping).
* **Per-block character cap** — optional total cap across a dialogue block.
* **Max lines per box** — how many lines fit one text box before auto-split starts a new box.
* **Auto-split on overflow** — split during live translation when a line exceeds the cap.
* **Auto-split on import** — run the bulk split automatically when importing translations.
* **BIN viewer mode** — scroll vs paged.

**Provider**
* **Provider** — Anthropic (Claude) or OpenAI-compatible; **custom endpoint** for the latter.
* **API key** — stored in your OS keychain.

**Translation** (affects the in-editor live **Translate** button only; CC Translate behavior is baked into the exported `PROMPT.md`)
* **Enforce character limits**, **block mode**, **context blocks** (depth), **cross-BIN context**, **fast mode**, **minimal notes**, **skip glossary**, **scoped glossary**.

---

## Keyboard shortcuts & gestures

| Action | How |
|---|---|
| Shortcuts dialog | `?` |
| Toggle Global Search | `Ctrl`+`Shift`+`F` |
| Close active dialog/overlay | `Esc` |
| Previous / next row | `↑` / `↓` |
| Step search matches while typing | `Alt`+`↑`/`↓` |
| Cycle tabs | scroll wheel over the tab bar |
| Reorder tabs | drag |
| Close a tab | hover → `×` |
| Restore a tab from autosave | right-click the tab (last few versions kept) |
| Jump to untranslated rows | click the tab's status dot when red/amber |
| Pick a character name | **🔍** on an unresolved Characters row |
| Block translation memory | **💡** on a block identical to one in another tab |

---

## Tips & troubleshooting

* **English getting crushed to one line?** That's the per-line cap doing its job per row. If your game adds boxes, set the cap as the line width and let auto-split add boxes; CC Translate's prompt already accounts for this.
* **"Auto-split: line-only"?** That file has no text-box bridge — lines still wrap within one box, but text longer than one box must be shortened. (This is rare and usually only affects files with little dialogue.)
* **Names show as a number / "Unknown"?** The ID isn't in any init table (often a minor or runtime-spawned character) — type the name directly in the Characters panel.
* **Use `--` or `...`, not em/en dashes.** The in-game renderer doesn't display `—` / `–` correctly.
* **Always keep a backup** of original game files before replacing them.

---

## Credits & special thanks

This tool stands entirely on the Eushully reverse-engineering community's work. Huge thanks to:

* **ArsArcanumb** — [EushlatorV2](https://github.com/ArsArcanumb/EushlatorV2). This tool's AGE `.BIN` translation handling is based on Eushlator; its `correct_en_box` English line-wrapping is ported directly into the editor's auto-split.
* **Kelebek1** — [Eushully-Decompiler](https://github.com/Kelebek1/Eushully-Decompiler). The SYS3 / SYS4 / SYS5 `.BIN` parser is built from this decompiler's AGE opcode table (`age-shared.cpp`).
* **Tweyen** and **Zoltanar** — reverse-engineering and tooling for the Unity-era Eushully archive/bundle format (`.arc` / UnityFS / Lua scenes), which made this tool's native *Yamiyo no Parente* support possible.

Built with [Tauri](https://tauri.app/), [Svelte](https://svelte.dev/), and Rust.

Thanks to Eushully for many years of great games — please support them by buying their titles.

## License

MIT — see [LICENSE](LICENSE). This license covers the **tool's source code only**. It grants no rights to any game data, which remains the property of its respective owners.
