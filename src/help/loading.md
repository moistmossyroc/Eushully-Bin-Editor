# Loading BINs & folders

## Open a single BIN

**Open ▸ BIN file** loads one `.BIN`. The editor parses the script, extracts the dialogue rows, and detects the speakers.

## Open a folder

**Open ▸ folder** scans a directory and lists every BIN with its type (scene / names / system) and dialogue count, so you can pick the ones worth translating. Scene files (`SC*.BIN`) hold the dialogue; `*INIT.BIN` files hold character names, the system tables, etc.

## The game / engine badge

After loading, the top‑right badge shows the **detected game and engine build**, e.g. *Madou Koukaku · SYS4 (v443)*. It's read from the BIN signature:

- `SYS3` / `SYS4` / `SYS5` is the engine generation (text encoding differs).
- The 3‑digit number (e.g. **443**) is the exact AGE build — it's how the editor tells Madou (443) from Megami Zero (415) from Sankai (448), and it's what selects the correct per‑game handling for speaker detection and box‑splitting.

If a build isn't recognised yet, the badge falls back to `SYS4 (v###)` and the version‑agnostic logic is used.

## Loose files vs `.alf` archives

Games ship dialogue either as **loose `.BIN` files** in the game folder, or packed inside **`.alf` archives**. The editor reads both. You can also open BINs the developer dropped as update layers — names prefixed `$1$`, `$2$`, etc. (`$N$` = a higher‑priority override layer the engine loads before the base file).

## Re‑opening a project

Open a BIN you've already translated and the editor restores its project automatically (translations, glossary, splits), re‑detecting speakers in case detection has improved since you last saved.
