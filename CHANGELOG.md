# Changelog

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
