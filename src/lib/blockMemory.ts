/**
 * Block-level translation memory.
 *
 * Splits each tab's entries into dialog "blocks" bounded by `wait-for-input`
 * opcodes, then fingerprints each block as `speaker|original` per editable
 * line joined by newlines. Cross-tab matching uses these fingerprints so a
 * suggestion only fires when an entire conversation chunk is identical
 * between two BINs — much safer than per-row matching, which can collapse
 * ambiguous short lines (`ええ`, `そうか`) across unrelated scenes.
 *
 * Cached per-tab by reference identity of `entries` and `rowSpeakers`. Edits
 * to individual `translated_text` fields don't invalidate the cache because
 * fingerprints only include `original_text` + speaker — the read for
 * "is this block fully translated?" stays fresh via direct iteration.
 */
import type { DialogueEntry } from "./types";
import type { TabState } from "./stores.svelte";

const EDITABLE_LABELS = new Set(["show-text", "display-furigana"]);

export interface BlockInfo {
  /** Stable hash of (speaker, original) over the block's editable entries in order. */
  fingerprint: string;
  /** Editable entries in source order. Indexed lookup is the apply path. */
  entries: DialogueEntry[];
}

export interface BlockMaps {
  /** entry.instruction_index → blockId (0-based). */
  blockIdOf: Map<number, number>;
  /** blockId → BlockInfo. Blocks with no editable entries are omitted. */
  blocks: Map<number, BlockInfo>;
  /** fingerprint → blockId for cross-tab lookup. */
  byFingerprint: Map<string, number>;
}

interface CacheEntry {
  entriesRef: DialogueEntry[];
  rowSpeakersRef: TabState["rowSpeakers"];
  maps: BlockMaps;
}
const cache = new WeakMap<TabState, CacheEntry>();

function buildMaps(entries: DialogueEntry[], rowSpeakers: TabState["rowSpeakers"]): BlockMaps {
  const blockIdOf = new Map<number, number>();
  const collected = new Map<number, DialogueEntry[]>();
  let curBlock = 0;
  for (const e of entries) {
    blockIdOf.set(e.instruction_index, curBlock);
    if (EDITABLE_LABELS.has(e.label) && !e.deleted && !e.is_context_row) {
      let bucket = collected.get(curBlock);
      if (!bucket) { bucket = []; collected.set(curBlock, bucket); }
      bucket.push(e);
    }
    // Block boundary: wait-for-input opcode terminates the current block.
    if (e.label === "wait-for-input") curBlock++;
  }

  const blocks = new Map<number, BlockInfo>();
  const byFingerprint = new Map<string, number>();
  for (const [blockId, blockEntries] of collected) {
    if (blockEntries.length === 0) continue;
    const fp = blockEntries.map((e) => {
      const sref = rowSpeakers?.[String(e.instruction_index)];
      const speaker = sref?.literal_name ?? `#${sref?.source_raw_data ?? ""}`;
      return `${speaker}|${e.original_text}`;
    }).join("\n");
    blocks.set(blockId, { fingerprint: fp, entries: blockEntries });
    // First wins — later identical blocks within the same tab won't override
    // (rare but possible: repeated narration).
    if (!byFingerprint.has(fp)) byFingerprint.set(fp, blockId);
  }

  return { blockIdOf, blocks, byFingerprint };
}

/** Get (or compute + cache) the block maps for a tab. */
export function getBlockMaps(tab: TabState): BlockMaps {
  const cached = cache.get(tab);
  if (cached && cached.entriesRef === tab.entries && cached.rowSpeakersRef === tab.rowSpeakers) {
    return cached.maps;
  }
  const maps = buildMaps(tab.entries, tab.rowSpeakers);
  cache.set(tab, { entriesRef: tab.entries, rowSpeakersRef: tab.rowSpeakers, maps });
  return maps;
}

/** True when every editable entry in the block has a non-empty translation. */
export function isBlockFullyTranslated(block: BlockInfo): boolean {
  for (const e of block.entries) {
    if (!e.translated_text.trim()) return false;
  }
  return true;
}

/** True when at least one editable entry in the block lacks a translation. */
export function blockHasEmptyRows(block: BlockInfo): boolean {
  for (const e of block.entries) {
    if (!e.translated_text.trim()) return true;
  }
  return false;
}
