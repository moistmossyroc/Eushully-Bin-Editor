# Glossary

The glossary keeps names and key terms **consistent** across a whole translation — define a term once and every pipeline reuses it.

## What it does

- **Pre‑fills exact matches**: any source line that exactly equals a glossary term gets its translation auto‑filled.
- **Feeds the AI**: glossary terms are injected into both the live‑AI prompt and CC Translate's `PROMPT.md`, so the model renders names/terms your way.
- **Travels with the project**: stored per‑project; export/import to move it between files.

## Managing terms

In the **Glossary** panel: add `Japanese → English` pairs, edit, or remove them. Use it for character names, place names, recurring phrases, honorific handling — anything you want rendered identically everywhere.

## Folder glossary

Drop a `glossary.json` next to a folder of BINs and the editor merges it in on load (without stomping in‑editor changes). Handy for sharing one glossary across an entire game's scenes.

## CLI export/import

You can export the glossary for command‑line tooling and read results back, so a glossary built during CC Translate can feed future runs.
