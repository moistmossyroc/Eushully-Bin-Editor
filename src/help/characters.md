# Characters & speaker detection

The **Characters** panel lists every speaker the editor found in the loaded scene, and each dialogue row shows a speaker badge so you know who's talking — essential context when translating.

## How names are detected

AGE games identify the speaker differently per engine build, so the editor uses several strategies and picks what fits:

- **Triplet / register games** (e.g. Madou): the name is a `【…】` reference filled from `CVINIT`.
- **Display‑slot games** (e.g. Megami Zero, `v415`): the speaker is set via the `0x1D2` set‑display‑slot opcode; the editor reads that slot and resolves the character ID.
- **SYS5 / SYS3** patterns are handled with their own detectors.

## How names are resolved

Detected speaker IDs are looked up against the game's init BINs — `CVINIT`, `CIINIT`, `CNINIT`, `CHINIT` — which hold the `id → name` tables. The editor scans whichever of these the game uses, so names appear even when one table is empty (e.g. Megami Zero stores them in `CNINIT`, not `CHINIT`).

Keep the init BINs **next to the scene BIN** (same folder, including any `$N$` versions) so resolution can find them.

## Editing names

You can type a **translated name** for any character in the Characters panel. The badge on every line by that speaker updates to your translation, giving you consistent, readable attribution as you work.

## If no names appear

- Make sure the init BINs are in the same folder as the scene.
- A brand‑new engine build may need its detector added — the badge's build number (e.g. `v###`) tells us which.
