# Auto‑split & textbox pagination

English translations are usually longer than the Japanese, so a line can overflow the in‑game text box. Auto‑split fixes this by **word‑wrapping** the text and inserting the engine's own line/box breaks — no manual byte‑fiddling.

## Why it's needed

The engine ignores `\n` in strings. Lines break only via the `end‑text‑line` opcode, and new boxes only via a **bridge** (the box‑transition sequence). Auto‑split inserts those for you.

## Character caps (Settings)

- **Per‑line cap** — wrap width; lines longer than this are wrapped.
- **Per‑box cap / max lines per box** — when to start a new box.

Set these to match the game's box size. Overflowing rows are flagged in the editor.

## Splitting

- **Single row**: split an individual overflowing entry on demand.
- **Split all**: process every overflow in the file in one pass. This is batched — one round‑trip and a single table repaint — so it stays fast even on large scenes.

Output is identical either way: the same boxes, just faster in bulk.

## Bridges are per‑game

The box‑transition (“bridge”) the editor clones differs by engine build. The editor detects the right one automatically:

- **Clean** — a simple branch‑free transition (e.g. Madou).
- **Degraded** — a richer transition with branches/extra opcodes that the editor safely adapts (e.g. Megami Zero's double box‑open + subroutine call). Works, just flagged so you know it was adapted.
- **Unavailable** — no usable transition found; auto‑split into new boxes isn't supported for that BIN (you can still wrap within the existing box).

## Round‑trip safety

Every split is validated: the export re‑parses its own output and aborts before writing if it wouldn't round‑trip, so a split can't corrupt your BIN. If something looks off in‑game (an extra empty box, a stray break), report the scene — bridge handling can be tuned per build.
