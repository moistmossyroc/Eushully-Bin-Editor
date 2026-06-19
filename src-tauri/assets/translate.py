"""
Canonical helper for the Eushully BIN translation workflow.

Provides a stable, deterministic API that Claude Code uses to read, iterate,
and update translation.json. Shipped in every CC Translate export so the
workflow is consistent across runs.

USAGE FROM CLAUDE CODE
----------------------
    import translate as t

    data = t.load()
    for entry in t.untranslated(data):
        # entry is a dict with keys: id, label, speaker, original, translated, notes
        english = your_translation_logic(entry["original"])
        t.set_translation(data, entry["id"], english)
        # Optional: save periodically so a crash doesn't lose progress
        t.save(data)

    t.save(data)
    t.print_status(data)

CLI
---
    python translate.py status       # print progress (translated / total)
    python translate.py list-pending # print ids and originals of untranslated entries
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any, Dict, Iterator, List

# translation.json lives in the same folder as this script.
_HERE = Path(__file__).resolve().parent
JSON_PATH = _HERE / "translation.json"
GLOSSARY_PATH = _HERE / "glossary.json"


def load_glossary() -> List[Dict[str, Any]]:
    """Load glossary.json's `entries` list. Returns [] if the file is missing
    or malformed — glossary is optional."""
    if not GLOSSARY_PATH.is_file():
        return []
    try:
        raw = json.loads(GLOSSARY_PATH.read_text(encoding="utf-8"))
    except Exception:
        return []
    entries = raw.get("entries", []) if isinstance(raw, dict) else []
    return [e for e in entries if isinstance(e, dict) and e.get("japanese")]


def glossary_for_block(block: List[Dict[str, Any]],
                       glossary: List[Dict[str, Any]] | None = None
                       ) -> List[Dict[str, Any]]:
    """Return only the glossary entries whose Japanese term appears as a
    substring in any of the block's originals. Keeps per-block token cost low
    by skipping the ~hundreds of irrelevant terms.

    Output is the compact form `{ja, en, notes?}` (skips `category` and empty
    notes to save tokens). Ordered by first appearance in the block."""
    if glossary is None:
        glossary = load_glossary()
    if not glossary:
        return []
    haystack = "\n".join(str(e.get("original", "")) for e in block)
    out: List[Dict[str, Any]] = []
    for g in glossary:
        ja = str(g.get("japanese", ""))
        if ja and ja in haystack:
            compact: Dict[str, Any] = {"ja": ja, "en": str(g.get("english", ""))}
            notes = str(g.get("notes", "")).strip()
            if notes:
                compact["notes"] = notes
            out.append(compact)
    return out


def load() -> Dict[str, Any]:
    """Load translation.json and return the parsed object."""
    if not JSON_PATH.is_file():
        raise FileNotFoundError(f"translation.json not found at {JSON_PATH}")
    return json.loads(JSON_PATH.read_text(encoding="utf-8"))


def save(data: Dict[str, Any]) -> None:
    """Write `data` back to translation.json (pretty-printed, UTF-8, BOM-free)."""
    JSON_PATH.write_text(
        json.dumps(data, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )


def entries(data: Dict[str, Any]) -> List[Dict[str, Any]]:
    """Return the entries list. Mutate items in place to update; call save() to persist."""
    return data.get("entries", [])


def _is_auto_resolved(entry: Dict[str, Any]) -> bool:
    """True if this entry doesn't need Claude. Covers:
      - In-folder duplicates (`duplicate_of`) → canonical handles it.
      - Cross-folder duplicates (`external_duplicate_of`) → the editor's
        import step fills it from the sibling folder's mapping.json.
    Both cases mean zero tokens for Claude on this row."""
    return (
        entry.get("duplicate_of") is not None
        or entry.get("external_duplicate_of") is not None
    )


def untranslated(data: Dict[str, Any]) -> Iterator[Dict[str, Any]]:
    """Yield every entry whose `translated` field is empty AND that needs to be
    translated directly. Skips entries that resolve automatically at import
    time (in-folder or cross-folder duplicates)."""
    for entry in entries(data):
        if str(entry.get("translated", "")).strip():
            continue
        if _is_auto_resolved(entry):
            continue
        yield entry


def untranslated_blocks(data: Dict[str, Any]) -> Iterator[List[Dict[str, Any]]]:
    """Yield untranslated entries grouped by `block_id` (one list per dialogue
    block). The whole list shares conversational context — translate it as a
    single batch to preserve character voice and tone across the block.

    Blocks are yielded in source order; within a block, entries are in source
    order. Skips fully-translated blocks AND blocks whose only remaining
    entries are auto-resolved (in-folder + cross-folder duplicates) — those
    get filled at import time without Claude work."""
    by_block: Dict[int, List[Dict[str, Any]]] = {}
    block_order: List[int] = []
    for entry in entries(data):
        if str(entry.get("translated", "")).strip():
            continue
        if _is_auto_resolved(entry):
            continue
        bid = int(entry.get("block_id", 0))
        if bid not in by_block:
            by_block[bid] = []
            block_order.append(bid)
        by_block[bid].append(entry)
    for bid in block_order:
        yield by_block[bid]


def set_translations(data: Dict[str, Any], mapping: Dict[int, str]) -> int:
    """Bulk-set translations from {entry_id: text}. Returns the number of
    entries updated. Use this with `untranslated_blocks` to translate a whole
    block at once and save once at the end."""
    by_id: Dict[int, Dict[str, Any]] = {e["id"]: e for e in entries(data)}
    updated = 0
    for entry_id, text in mapping.items():
        e = by_id.get(int(entry_id))
        if e is not None:
            e["translated"] = text
            updated += 1
    return updated


def find_entry(data: Dict[str, Any], entry_id: int) -> Dict[str, Any] | None:
    """Return the entry with matching `id`, or None."""
    for entry in entries(data):
        if entry.get("id") == entry_id:
            return entry
    return None


def set_translation(
    data: Dict[str, Any],
    entry_id: int,
    text: str,
    notes: str = "",
) -> bool:
    """Set the `translated` (and optionally `notes`) field on the entry with
    matching `id`. Returns True if the entry was found and updated."""
    entry = find_entry(data, entry_id)
    if entry is None:
        return False
    entry["translated"] = text
    if notes:
        entry["notes"] = notes
    return True


def stats(data: Dict[str, Any]) -> Dict[str, int]:
    """Return progress stats for the whole file.
    - total: every row in entries[]
    - translated: rows with non-empty `translated` field
    - remaining: rows you still need to translate yourself (excludes duplicates
      which fill in automatically)
    - dup_pending: rows whose canonical hasn't been translated yet (counted
      separately so progress math is honest)
    """
    items = entries(data)
    total = len(items)
    translated = sum(1 for e in items if str(e.get("translated", "")).strip())
    needs_work = sum(
        1 for e in items
        if not str(e.get("translated", "")).strip() and e.get("duplicate_of") is None
    )
    dup_pending = sum(
        1 for e in items
        if not str(e.get("translated", "")).strip() and e.get("duplicate_of") is not None
    )
    return {
        "total": total,
        "translated": translated,
        "remaining": needs_work,
        "dup_pending": dup_pending,
    }


def print_status(data: Dict[str, Any] | None = None) -> None:
    """Print a one-line progress summary to stdout."""
    if data is None:
        data = load()
    s = stats(data)
    pct = (s["translated"] * 100 // s["total"]) if s["total"] else 0
    tail = f", {s['dup_pending']} pending dup-fill" if s["dup_pending"] else ""
    print(f"{s['translated']}/{s['total']} translated ({pct}%), {s['remaining']} to translate{tail}")


def print_pending(data: Dict[str, Any] | None = None, limit: int = 20) -> None:
    """Print the first `limit` untranslated entries (id + original snippet)."""
    if data is None:
        data = load()
    shown = 0
    for entry in untranslated(data):
        original = str(entry.get("original", ""))
        snippet = original if len(original) <= 60 else original[:57] + "..."
        speaker = entry.get("speaker")
        prefix = f"[{speaker}] " if speaker else ""
        print(f"  #{entry['id']:>6}  {prefix}{snippet}")
        shown += 1
        if shown >= limit:
            break
    if shown == 0:
        print("  (no pending entries — everything translated)")


def _print_json(obj: Any) -> None:
    """Print JSON to stdout with UTF-8 + no ASCII escapes. Use sys.stdout.buffer
    on Windows to bypass the cp932 console encoding wall that crashes on
    Japanese characters when printed via `print()`."""
    text = json.dumps(obj, ensure_ascii=False, indent=2)
    try:
        sys.stdout.buffer.write(text.encode("utf-8"))
        sys.stdout.buffer.write(b"\n")
    except Exception:
        # Fallback: plain print (may garble on Windows console but won't crash
        # if the data is ASCII-safe).
        print(text)


def _cli(args: List[str]) -> int:
    cmd = args[0] if args else "status"
    if cmd == "status":
        print_status()
    elif cmd == "list-pending":
        limit = int(args[1]) if len(args) > 1 else 20
        print_pending(limit=limit)
    elif cmd == "blocks-pending":
        # Print "block_id: count" for each block with untranslated entries.
        data = load()
        blocks: Dict[int, int] = {}
        for entry in untranslated(data):
            bid = int(entry.get("block_id", 0))
            blocks[bid] = blocks.get(bid, 0) + 1
        if not blocks:
            print("(no pending blocks — everything translated)")
        else:
            for bid in sorted(blocks):
                print(f"  block {bid}: {blocks[bid]} pending")
            print(f"\n{len(blocks)} block(s) with pending work.")
    elif cmd == "block":
        # Dump the Nth untranslated block as JSON to stdout (UTF-8 safe).
        # Output schema: {"glossary": [...], "entries": [...]} — glossary
        # lists only the terms whose Japanese appears in this block (auto-
        # scoped so context stays small).
        # Usage: python translate.py block 0
        if len(args) < 2:
            print("usage: python translate.py block <N>", file=sys.stderr)
            return 1
        try:
            n = int(args[1])
        except ValueError:
            print(f"block index must be an integer, got: {args[1]!r}", file=sys.stderr)
            return 1
        data = load()
        all_blocks = list(untranslated_blocks(data))
        if n < 0 or n >= len(all_blocks):
            print(f"block {n} out of range (0..{len(all_blocks) - 1})", file=sys.stderr)
            return 1
        block = all_blocks[n]
        payload: Dict[str, Any] = {"entries": block}
        scoped = glossary_for_block(block)
        if scoped:
            payload = {"glossary": scoped, "entries": block}
        _print_json(payload)
    elif cmd == "glossary":
        # Dump the full glossary (or filter by Japanese substring match).
        # Usage: python translate.py glossary           — all entries
        #        python translate.py glossary リル      — entries containing リル
        glossary = load_glossary()
        if not glossary:
            print("(no glossary.json in this folder, or it has no entries)")
            return 0
        if len(args) > 1:
            needle = args[1]
            glossary = [g for g in glossary if needle in str(g.get("japanese", ""))]
        _print_json(glossary)
    elif cmd == "apply":
        # Read a {id: translated_text} JSON mapping from a file and apply via
        # set_translations + save. UTF-8 file IO sidesteps Windows console
        # encoding issues that crash `python -c` with Japanese inline strings.
        # Usage: python translate.py apply path/to/mapping.json
        if len(args) < 2:
            print("usage: python translate.py apply <mapping.json>", file=sys.stderr)
            return 1
        mapping_path = Path(args[1])
        if not mapping_path.is_file():
            print(f"file not found: {mapping_path}", file=sys.stderr)
            return 1
        raw = json.loads(mapping_path.read_text(encoding="utf-8"))
        if not isinstance(raw, dict):
            print(f"expected an object {{id: text}} in {mapping_path}", file=sys.stderr)
            return 1
        mapping = {int(k): str(v) for k, v in raw.items()}
        data = load()
        applied = set_translations(data, mapping)
        save(data)
        print(f"applied {applied} translation(s); ", end="")
        print_status(data)
    elif cmd in ("-h", "--help", "help"):
        print(__doc__)
    else:
        print(f"unknown command: {cmd}", file=sys.stderr)
        print(
            "usage: python translate.py [status | list-pending [N] | "
            "blocks-pending | block <N> | apply <mapping.json> | "
            "glossary [search] | help]",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(_cli(sys.argv[1:]))
