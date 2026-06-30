# Troubleshooting & FAQ

## The badge says `SYS4 (v###)` with no game name
The engine build is recognised but not yet mapped to a name, or it's a new title. Everything still works via the version‑agnostic logic; the build number identifies it for adding a name later.

## No character names / empty Characters panel
Keep the init BINs (`CVINIT` / `CIINIT` / `CNINIT` / `CHINIT`, plus any `$N$` versions) in the **same folder** as the scene BIN so name resolution can find them. If they're present and still empty, the engine build may need its speaker detector added.

## Auto‑split says "Unavailable"
The editor couldn't find a usable box‑transition (bridge) for that BIN's engine build. You can still wrap within the existing box; creating new boxes isn't supported there yet. Report the game/build (the badge number) so its bridge can be added.

## A split produced an extra empty box or a stray line break
Some engine builds use richer box transitions (flagged **Degraded**). If a continuation looks off in‑game, note the scene — the per‑build bridge can be trimmed.

## Export refused / "would not round‑trip"
That's the safety guard working: the export re‑parses its own output and aborts before writing if the result wouldn't parse back identically, so a bad export can't overwrite a good BIN. Usually it means an edit broke the script structure (e.g. a blanked speaker slot). Undo the offending change and retry.

## The game shows a "データが壊れています" (data is corrupted) popup
A known, harmless Eushully quirk on some installs — clicking *Yes* continues. The loader auto‑dismisses it where it can. It doesn't indicate a problem with your patch.

## Language swap (backtick) does nothing
- Make sure you exported the **Bilingual** fanpatch (not a plain one) and that **both** `version.dll` and the `.fanpatch` are in the game folder.
- The swap is mid‑scene for dialogue; UI/CG images change on their next load.

## Names don't swap with the language
Expected for engine‑drawn name plates — they're rendered from boot data, not the per‑line script, so they stay in the launch language. Dialogue swaps; name labels don't.

## Reverting everything
Delete `version.dll` (and the `.fanpatch`) from the game folder. The original game files were never modified.
