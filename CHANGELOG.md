# Changelog

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
