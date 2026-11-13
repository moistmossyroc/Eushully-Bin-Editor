# Changelog

## 1.5.0 — 2026-11-13

### New — box splits that keep the speaker (in-game verified on 16 games)
- **Keep-speaker continuation boxes.** When a translated line is split into more
  boxes, every continuation box keeps the speaker's name plate, text colour and
  backlog entry, and does not replay the voice line. Narration continuations stay
  nameless. Verified in-game on Arterial, VERITA, Megami Zero, Tenmei, Hyakusen,
  Madou, LaDEA, Sankai, Kamidori, Rhapsody, Castle Meister, Grasesta, Labyrinth
  Meister, Meishoku and Minefuka, including save/load on a continuation box.
- Classic auto-split, the row menu, overflow review, live auto-split and bulk split
  all use this box break in Semantic mode.
- **SYS3 support for box splits** (Kuutei, Meishoku, Minefuka): the old engine has
  no statement-block opcodes and needs the line closed before the click and the
  window redrawn after the clear — the export now does both, and repeats the
  in-text name line on speech continuations.
- **Export lint**: warn-only checks against the original (text drawn while the
  window is hidden, unbalanced statement blocks, clicks that don't clear the box).

### New — engine word-wrap mode
- With the fan-patch loader's English word wrap on, the game breaks rows at word
  boundaries itself; the editor then measures overflow in whole boxes and splits
  into balanced boxes instead of hard line breaks.

### New — Browse & Extract workspace
- Browse, preview (images as PNG) and extract assets of AGE (SYS3–5 .alf) and
  Parente (.arc) games from the editor (`--browse <folder>` at launch too).

### Parente
- **Message-window model** measured in-game: 3 rows × 52 half-width / 26
  full-width characters; English is pre-broken at word boundaries.
- **Fixed: ASCII parentheses froze the game.** Parente turns `( )` into its
  inline-command brackets, so a line like "BP (Bonus Points)" hung the message.
  Export now writes `[ ]` instead (and full-width `＜ ＞` for stray `< >`); ruby
  markup is kept. Re-export existing patches to apply.
- Config language switch (CONFIG.DAT), runtime font fix, timestamp-safe patch export.

### Fixed
- Splits made in the first box of a scene could lose the speaker (the redraw call
  was matched against the scene's voiced start-up routine).
- Tests that need sample game files now skip cleanly when the files are absent.

### Build
- The shared `parente-arc` and `eushully-assets` crates are vendored under
  `crates/`, so a fresh clone builds on its own.

## 1.4.1 — 2026-07-04

### Fixed — in-game render correctness
- **Furigana lines no longer render as ruby.** A dialogue line containing an
  inline `display-furigana` word used to put the whole English translation on the
  furigana (ruby) slot, so it rendered as tiny annotation with the Japanese word
  still showing. The translation now lands on a normal `show-text`, and auto-split
  continuations are proper show-text. Already-translated furigana scenes are
  corrected simply by re-exporting (a prose-carrying furigana entry converts to
  show-text on export).
- **Full Shift-JIS character normalization on export.** Characters that Claude
  emits but the game's font can't show were becoming wide or `?` mojibake in-game.
  Now normalized on export: smart/curly quotes → ASCII (fixes `Celica's`
  rendering as `Celica' s`); tilde `~` → centered full-width wave dash; and
  accented Latin (é/ñ/ü), romaji macrons (ā/ō/ū), em/en dashes, non-breaking
  spaces, ligatures (æ/œ/ß), fractions, `™©®`, and hearts (→ `<3`) are
  transliterated to safe ASCII. The in-editor text keeps its original typography;
  only the exported bytes change. Re-export existing scenes to apply.
- **`.fanpatch` loading for Japanese-named game folders.** The loader's zip reader
  assumed UTF-8 paths and failed to open the patch when the game folder had a
  Japanese name (so nothing was served).
- Loader-package export no longer drops a redundant loose `fanpatch_title.txt`
  (the title/label are bundled inside the `.fanpatch`).

### Added
- **CC Translate support for Parente** (Yamiyo no Parente) scenes, with a
  format-aware export menu — native `.arc`/Lua export, and the AGE-only
  `.fanpatch` options are hidden when a Parente tab is active.
- **Inline editing** in the Glossary Check "Abbreviations in dialogue" list, so
  flagged lines can be fixed without leaving the dialog.

## 1.4.0 — 2026-06-29

### Added — archived-game support & staged translation
- **Load games packed in `.alf` archives** (SYS3/4/5 folder loading + `.alf`
  reader/writer). Read-only installs (e.g. `Program Files`) extract just the
  selected scenes + character-name tables into a writable **workspace** you
  choose — **remembered per game**, and later loads skip re-extracting files
  already present.
- **Staged folder picker, five categories** — *Character & item names · Story
  scenes · Side scenes · UI & system · Other* — names-first, with files that have
  no translatable text hidden behind a toggle.
- **Character/item name-table translation** — `set-string` name tables
  (CVINIT/CIINIT/CNINIT/…) are now editable and exported to CC Translate, which
  orders name files first so scenes inherit the finalized names.
- **`.fanpatch` loader configs bundled inside the zip** + in-editor title/label
  fields (a shipped patch is just `version.dll` + one `.fanpatch`).
- **UI-abbreviation leak check** under Glossary Check — flags stat abbreviations
  (ATK/DEF/…) that landed in dialogue prose; glossary pre-fill no longer seeds
  them into scene translations.
- **Re-flow to box caps** — rebuild line/box layout at corrected geometry without
  re-translating; call-free bridge pagination adds text boxes to bridge-less
  event scenes.
- **Export popover** (grouped, with descriptions) replacing the export dropdown,
  and a **special-scene filename-prefix** setting.

### Fixed
- **Speaker names for SYS5-style ID-register SYS4 games** (e.g. 戦女神VERITA) now
  resolve (the engine resolves character IDs at runtime; detection now handles
  the `mov g-int`-before-`lookup-array` ordering).
- Archived-game loading hardened end-to-end: `.alf` extraction is per-entry
  best-effort, the workspace source path is captured correctly through dialog
  close, and load failures surface a clear status instead of failing silently.

### Changed
- Native UI translation extended (Megami Zero menu / title bar); broader
  load-flow error surfacing throughout the SYS folder pipeline.

## 1.3.2 — 2026-06-21

### Added — Export Review (was "Omit empty cells")
- The button now opens a full **Export Review** dialog instead of a one-shot bulk
  toggle. Across the active tab or **all open tabs** (grouped by tab):
  - Per-row **red/blue Export toggle** (same control as the editor) — blue =
    will export, red = omitted — for at-a-glance scanning.
  - Filter chips: **Needs review · Omitted · Included · All editable**, with counts.
  - A dedicated **notes column** (notes often say whether a row should be omitted).
  - Click the original to **expand surrounding context**; double-click it, or click
    the translation, to **jump to the row**.
  - **Omit all / Include all** for the current view (one bulk-undo).
  - Remembers scope/filter/scroll so reopening **resumes where you left off**.

### Fixed
- **Glossary panel:** editing an entry while a filter was active made it vanish
  mid-edit (it stopped matching). The edited row now stays put until you finish
  (click away / Update / Enter / Escape).
- **Find & Replace:** the confirm popup didn't block, so the replace could fire
  before you answered. Now uses the awaited themed dialog, and its message
  correctly notes the action is bulk-undoable.

### Changed — polish
- **All confirmation popups are now themed** (no more OS-native dialogs):
  Replace/Replace-all, Restore backup, Clear glossary, Optimize via API,
  Enable auto-split, BIN-hash/translation mismatch warnings, and "apply character
  names to all tabs." Consistent dark styling, proper line breaks, danger
  styling on destructive actions.

## 1.3.1 — 2026-06-21

### Added — glossary consistency review
- **Apply all edits** — edit the translation on as many flagged rows as you want
  (across tabs), then commit them in one click, captured as a single bulk-undo.
  Per-row Apply / Open-row still available.
- **Script filter** — All · Katakana (names) · Other, with live counts. Katakana
  terms are almost always proper nouns, so this lets you triage names separately.
  Filtering only narrows the list; pending edits and Apply-all stay
  filter-independent so nothing is lost when you switch.

## 1.3.0 — 2026-06-21

### Changed — glossary review
- **Both glossary tools moved into the Glossary panel** (Check + Auto-apply),
  next to Suggest/Import/Export.
- **Now span every open tab**, grouped by tab. The consistency **Check** lets you
  fix each flagged row two ways — edit the translation **inline**, or **jump to
  the row** in its tab. **Auto-apply** reviews safe substitutions across all tabs
  and applies them under a single bulk-undo.

### Fixed
- **Glossary check false positives.** Matching is now Japanese script-boundary
  aware, so a term is no longer flagged when it's merely an inner fragment of a
  longer same-script word (e.g. ベル "Bell" inside レベル "level", or 重要 inside
  重要性). Auto-apply's replace step is boundary-aware too. Duplicate glossary
  terms also no longer produce duplicate flags.

## 1.2.0 — 2026-06-21

### Added — glossary
- **Glossary auto-apply (reviewable).** Proposes only *safe, deterministic*
  substitutions — filling an untranslated row whose original is exactly a
  glossary term, and replacing a Japanese term that leaked verbatim into the
  English — and lets you review/deselect each one before applying. Nothing is
  ever applied silently; the whole batch is one-click undoable. After a CC
  import, a toast points you to any available fixes.

### Changed — tests / quality
- **Engine-specific round-trip tests** that need no game data: synthetic
  SYS3 / SYS4 / SYS5 BINs validate the parse↔write codec + the insertion-ordering
  contract on every machine (CI, clean clones), not just where a real BIN sits.
- Game-data-dependent tests now **skip cleanly** when their BIN is absent instead
  of failing, so `cargo test` is green on a fresh checkout.

### Removed
- Dead speaker-setup carry-over code path (unused after the Sankai continuation
  work settled on narrator-colored continuation boxes).

## 1.1.0 — 2026-06-20

### Fixed
- **Export ordering corruption.** Inserted line breaks / continuation lines could
  be written out of order, making dialogue render scrambled in-game. Fixed the
  insertion-ordering contract end to end.
- **Export safety net.** Every export now re-parses its own output and aborts
  *before touching disk* if the result is corrupt — the original BIN and its
  backup are never overwritten by a bad write. Backed by round-trip regression
  tests.

### Added — line splitting & pagination (SYS4)
- **Split lines** (word-boundary line breaks) and **adding a text box** are now
  separate, user-controlled actions.
- **Balanced pagination** — long dialogue splits into evenly-filled boxes
  (4 lines → 2+2, never an orphan single-line box).
- **Global actions** — "Split lines — all tabs" and "Paginate — all tabs" in
  the Overflow Review window, for the whole project in one click after a batch
  import.

### Added — glossary
- **Translate glossary terms via the Claude CLI** — select terms, send them to an
  interactive `claude` session (your subscription, no API billing), with an
  Eushully-world brief; results auto-import (with a manual fallback).
- **Glossary consistency check** — flags translated rows where a glossary term's
  Japanese appears but its English rendering is missing; click to jump.

### Added — workflow / QoL
- **Find next untranslated** navigator in the Stats panel — cycles across all
  open tabs; plus a per-tab "N left" count.
- **Replace all** one-click button in Global Search.
- **Dialog preview** now shows the per-box line limit, marking lines that would
  fall off-screen in-game until paginated.
- **One-click undo** for bulk operations (replace-all, split-all, paginate-all).

### Added — Parente (Unity / IL2CPP titles)
- Adult-scene classification + Normal/Adult filtering in the scene picker.
- Characters tab population, themed dialogs, export-folder defaults, and
  load-progress fixes.

### Notes
- No game data is included or distributed — you supply your own legally-obtained
  files. The editor only modifies what you load.
