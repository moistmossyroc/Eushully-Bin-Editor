import { store } from "./stores.svelte";
import type { DialogueEntry, InsertedInstruction, SerializedArg, SerializedInstruction, SplitPlan } from "./types";
import { getBridgeClone } from "./ipc";

/**
 * Insert an instruction both visually (into entries array) and for export
 * (into insertedInstructions). The new row appears immediately in the table.
 *
 * The visual entry's instruction_index is set to a unique negative value.
 * The corresponding InsertedInstruction is linked by storing the same
 * synthetic index in a `_syntheticIndex` field on the InsertedInstruction.
 */
export function insertInstructionVisually(
  afterInstructionIndex: number,
  label: string,
  opcode: number,
  args: SerializedArg[],
  isContextRow: boolean,
  originalText: string = "",
  translatedText: string = "",
): number {
  const tab = store.activeTab;
  if (!tab) return 0;

  // Generate a unique negative index by finding the current minimum and going one lower
  const existingMin = tab.entries.reduce(
    (min, e) => Math.min(min, e.instruction_index),
    0,
  );
  const syntheticIndex = existingMin - 1;

  // 1. Add to insertedInstructions for export
  const exportInstruction: InsertedInstruction = {
    after_index: afterInstructionIndex,
    opcode,
    label,
    arguments: args,
    _syntheticIndex: syntheticIndex,
  };
  tab.insertedInstructions = [...tab.insertedInstructions, exportInstruction];

  // 2. Create a visual DialogueEntry and splice it into the entries array
  const visualEntry: DialogueEntry = {
    instruction_index: syntheticIndex,
    opcode,
    label,
    original_text: originalText,
    translated_text: translatedText,
    notes: "[Inserted]",
    is_context_row: isContextRow,
    string_arg_indices: [],
    deleted: false,
  };

  // Find where to insert: right after the entry with matching instruction_index
  const insertAfterPos = tab.entries.findIndex(
    (e) => e.instruction_index === afterInstructionIndex
  );

  if (insertAfterPos !== -1) {
    tab.entries.splice(insertAfterPos + 1, 0, visualEntry);
    tab.entries = tab.entries;
  } else {
    tab.entries = [...tab.entries, visualEntry];
  }

  tab.isDirty = true;
  store.statusMessage = `Inserted ${label} after instruction #${afterInstructionIndex}`;
  return syntheticIndex;
}

/**
 * Resolve inserted instructions for export to Rust.
 *
 * The frontend uses negative synthetic indices for chaining (insert B after
 * inserted A). Rust expects `after_index` to be a real (non-negative)
 * instruction index (`usize`). This function walks the chain and resolves
 * each negative `after_index` to the real ancestor, preserving insertion
 * order via the visual entries array.
 */
export function resolveInsertionsForExport(
  insertedInstructions: InsertedInstruction[],
  entries: DialogueEntry[],
): InsertedInstruction[] {
  // Sync translated text from dialogue entries into show-text
  // InsertedInstructions so user edits to continuation rows are exported.
  const SHOW_TEXT_OPCODE = 0x6E;
  for (const ins of insertedInstructions) {
    if (ins._syntheticIndex === undefined || ins.opcode !== SHOW_TEXT_OPCODE) continue;
    const entry = entries.find((e) => e.instruction_index === ins._syntheticIndex);
    if (!entry || entry.translated_text.trim() === "") continue;
    for (let i = ins.arguments.length - 1; i >= 0; i--) {
      if (ins.arguments[i].arg_type === 2) {
        ins.arguments[i].string_value = entry.translated_text;
        break;
      }
    }
  }

  // Build a lookup: syntheticIndex -> InsertedInstruction
  const bySynthetic = new Map<number, InsertedInstruction>();
  for (const ins of insertedInstructions) {
    if (ins._syntheticIndex !== undefined) {
      bySynthetic.set(ins._syntheticIndex, ins);
    }
  }

  // Resolve a negative after_index to the real ancestor index
  function resolveAfterIndex(afterIndex: number): number {
    const visited = new Set<number>();
    let current = afterIndex;
    while (current < 0) {
      if (visited.has(current)) {
        // Circular chain safety — shouldn't happen, but bail out
        return 0;
      }
      visited.add(current);
      const parent = bySynthetic.get(current);
      if (!parent) return 0; // orphaned reference
      current = parent.after_index;
    }
    return current;
  }

  // Map visual entry position by synthetic index for ordering
  const entryOrder = new Map<number, number>();
  entries.forEach((e, i) => {
    if (e.instruction_index < 0) {
      entryOrder.set(e.instruction_index, i);
    }
  });

  // Resolve and tag with visual order, then sort.
  // Rust processes insertions in descending after_index order, inserting at
  // after_index+1 each time. For insertions sharing the same resolved
  // after_index, the LAST one processed ends up first visually. So we sort
  // in REVERSE visual order within each group — Rust's descending pass will
  // then produce the correct visual sequence.
  const resolved = insertedInstructions.map((ins) => ({
    after_index: resolveAfterIndex(ins.after_index),
    opcode: ins.opcode,
    label: ins.label,
    arguments: ins.arguments,
    _visualOrder: ins._syntheticIndex !== undefined
      ? (entryOrder.get(ins._syntheticIndex) ?? 0)
      : 0,
  }));

  resolved.sort((a, b) => {
    if (a.after_index !== b.after_index) return a.after_index - b.after_index;
    // Reverse visual order within same group (Rust inserts from the back)
    return b._visualOrder - a._visualOrder;
  });

  // Strip the temporary _visualOrder before returning
  return resolved.map(({ _visualOrder, ...rest }) => rest);
}

/**
 * Permanently remove a user-inserted entry from both the entries array
 * and the insertedInstructions list.
 *
 * Returns an error message if the entry can't be deleted, or null on success.
 */
export function deleteInsertedEntryForever(entry: DialogueEntry): string | null {
  const tab = store.activeTab;
  if (!tab) return "No active tab.";

  if (entry.instruction_index >= 0) {
    return "Only user-inserted entries can be permanently deleted.";
  }

  // Check if any other inserted instruction depends on this entry's index
  const dependents = tab.insertedInstructions.filter(
    (ins) => ins.after_index === entry.instruction_index
  );
  if (dependents.length > 0) {
    const depLabels = dependents.map((d) => d.label).join(", ");
    return `Cannot delete: other inserted instructions depend on this entry (${depLabels}). Delete those first.`;
  }

  // Remove from insertedInstructions (match by _syntheticIndex)
  tab.insertedInstructions = tab.insertedInstructions.filter(
    (ins) => ins._syntheticIndex !== entry.instruction_index
  );

  // Remove from entries
  const entryIdx = tab.entries.indexOf(entry);
  if (entryIdx !== -1) {
    tab.entries.splice(entryIdx, 1);
    tab.entries = tab.entries;
  }

  tab.isDirty = true;
  store.statusMessage = `Permanently removed ${entry.label}`;
  return null;
}

/** Note prefix used to tag auto-split continuation rows. */
const AUTO_SPLIT_NOTE_PREFIX = "[Auto-split from row #";

/** Check if an entry is an auto-split continuation row. */
export function isAutoSplitContinuation(entry: DialogueEntry): boolean {
  return entry.notes.startsWith(AUTO_SPLIT_NOTE_PREFIX);
}

/**
 * Parse the source instruction_index from a continuation row's note.
 * Returns null if the note doesn't match the expected format.
 */
function parseAutoSplitSource(notes: string): number | null {
  const match = notes.match(/^\[Auto-split from row #(-?\d+),/);
  return match ? parseInt(match[1], 10) : null;
}

/**
 * Apply a SplitPlan from the Rust auto_split_entry command.
 *
 * The BIN format uses end-text-line (opcode 0x6F) as a SEPARATE instruction
 * to break lines within a box — `\n` characters in show-text aren't rendered
 * as line breaks. So each wrapped line becomes its own show-text, with an
 * end-text-line between each pair.
 *
 * Insertion pattern for a 3-line, 2-box split:
 * ```
 *   source ST(box0 line0)   ← updated in place
 *   new ETL                 ← inserted (closes box0 line0)
 *   new ST(box0 line1)      ← inserted
 *   new ETL                 ← inserted (closes box0 line1)
 *   new ST(box0 line2)      ← inserted
 *   new ETL                 ← inserted (closes box0 line2)
 *   new bridge instructions ← inserted (box transition)
 *   new ST(box1 line0)      ← inserted
 *   new ETL                 ← inserted
 *   new ST(box1 line1)      ← inserted
 *   original ETL (unchanged) ← stays, terminates box1 line1
 * ```
 *
 * Returns the total number of inserted entries (lines + bridges + ETLs),
 * or an error string. Returns 0 when nothing was inserted (source already
 * matched, shouldn't happen but defensive).
 */
export function applySplitPlan(plan: SplitPlan): string | number {
  const tab = store.activeTab;
  if (!tab) return "No active tab.";

  const sourceIdx = tab.entries.findIndex(
    (e) => e.instruction_index === plan.source_entry_index,
  );
  if (sourceIdx === -1) return "Source entry not found.";

  const source = tab.entries[sourceIdx];
  const totalBoxes = plan.box_lines.length;
  const totalLines = plan.box_lines.reduce((sum, b) => sum + b.length, 0);

  // Need ETL template if we'll insert any line breaks.
  if (totalLines > 1 && !plan.end_text_line_template) {
    return "Cannot insert line breaks: no end-text-line template available in this BIN.";
  }

  // 1. Update source entry with first line of first box
  source.translated_text = plan.box_lines[0][0];

  // 2. Walk through boxes/lines, inserting ETL + ST (and bridge between boxes)
  let prevInstructionIndex = source.instruction_index;
  let inserted = 0;

  const insertETL = (): number => {
    const tpl = plan.end_text_line_template!;
    const etlArgs: SerializedArg[] = tpl.arguments.map((a) => ({
      arg_type: a.arg_type,
      raw_value: a.raw_data,
      string_value: a.decoded_string,
    }));
    const idx = insertInstructionVisually(
      prevInstructionIndex,
      tpl.label,
      tpl.opcode,
      etlArgs,
      true, // ETL is a context row (not translatable)
    );
    prevInstructionIndex = idx;
    inserted++;
    return idx;
  };

  const insertShowText = (text: string, boxNum: number, lineNum: number, totalLinesInBox: number): number => {
    const contArgs: SerializedArg[] = plan.show_text_template_args.map((a) => ({
      arg_type: a.arg_type,
      raw_value: a.raw_data,
      string_value: a.decoded_string,
    }));
    for (let j = contArgs.length - 1; j >= 0; j--) {
      if (contArgs[j].arg_type === 2) {
        contArgs[j].string_value = text;
        break;
      }
    }
    const idx = insertInstructionVisually(
      prevInstructionIndex,
      source.label,
      source.opcode,
      contArgs,
      false,
      "",
      text,
    );
    const entry = tab.entries.find((e) => e.instruction_index === idx);
    if (entry) {
      entry.notes = `[Auto-split from row #${source.instruction_index}, box ${boxNum + 1}/${totalBoxes}, line ${lineNum + 1}/${totalLinesInBox}]`;
    }
    prevInstructionIndex = idx;
    inserted++;
    return idx;
  };

  // Box 0: source is line 0. Insert (ETL, ST) for each additional line.
  for (let lineNum = 1; lineNum < plan.box_lines[0].length; lineNum++) {
    insertETL();
    insertShowText(plan.box_lines[0][lineNum], 0, lineNum, plan.box_lines[0].length);
  }

  // Boxes 1..N: terminate previous box with ETL, then bridge, then (ST, [ETL, ST]…)
  for (let boxNum = 1; boxNum < totalBoxes; boxNum++) {
    // End the previous box's last line with an ETL (so the bridge has a
    // clean termination before it).
    insertETL();

    // Bridge instructions for this box transition.
    const bridgeInstrs = plan.bridge_instructions_per_split[boxNum - 1] ?? [];
    for (const bi of bridgeInstrs) {
      const args: SerializedArg[] = bi.arguments.map((a) => ({
        arg_type: a.arg_type,
        raw_value: a.raw_data,
        string_value: a.decoded_string,
      }));
      const idx = insertInstructionVisually(
        prevInstructionIndex,
        bi.label,
        bi.opcode,
        args,
        true,
      );
      prevInstructionIndex = idx;
      inserted++;
    }

    // First line of new box (no leading ETL — the bridge opens a fresh box).
    const lines = plan.box_lines[boxNum];
    insertShowText(lines[0], boxNum, 0, lines.length);
    for (let lineNum = 1; lineNum < lines.length; lineNum++) {
      insertETL();
      insertShowText(lines[lineNum], boxNum, lineNum, lines.length);
    }
  }

  tab.isDirty = true;
  if (totalBoxes === 1) {
    store.statusMessage = `Wrapped row into ${plan.box_lines[0].length} line(s) in 1 box`;
  } else {
    store.statusMessage = `Split into ${totalBoxes} boxes (${totalLines} lines total)`;
  }
  return inserted;
}

/**
 * Merge auto-split continuation rows back into their parent entry.
 *
 * Walks forward from the parent through the entries array, gathering every
 * inserted row (negative instruction_index) that belongs to this split
 * group — continuation show-text rows, end-text-line opcodes between them,
 * and bridge instructions for box transitions. Concatenates the show-text
 * lines back onto the parent and deletes everything else.
 *
 * Returns an error message or null on success.
 */
export function unsplitContinuation(entry: DialogueEntry): string | null {
  const tab = store.activeTab;
  if (!tab) return "No active tab.";

  const sourceInstrIdx = parseAutoSplitSource(entry.notes);
  if (sourceInstrIdx === null) {
    return "Not an auto-split continuation row.";
  }

  const parentIdx = tab.entries.findIndex(
    (e) => e.instruction_index === sourceInstrIdx,
  );
  if (parentIdx === -1) return "Parent entry not found.";
  const parent = tab.entries[parentIdx];

  // Walk forward from parent. Collect all inserted rows (negative index)
  // that belong to this split group until we hit a positive-index entry
  // (which marks the end of our inserted run).
  const groupContinuationIds = new Set<number>();
  const toDelete: DialogueEntry[] = [];
  const continuationTexts: string[] = [];

  for (let i = parentIdx + 1; i < tab.entries.length; i++) {
    const e = tab.entries[i];
    if (e.instruction_index >= 0) {
      // Reached an original entry — stop scanning unless it's one we already
      // know about (shouldn't happen since originals have non-negative idx).
      break;
    }
    const noteSource = parseAutoSplitSource(e.notes);
    if (noteSource === sourceInstrIdx) {
      // Show-text continuation row belonging to this split
      groupContinuationIds.add(e.instruction_index);
      continuationTexts.push(e.translated_text);
      toDelete.push(e);
    } else if (e.notes === "[Inserted]" && e.is_context_row) {
      // Bridge or ETL inserted as part of the split. Belongs to this group
      // only if we're still in the contiguous run starting from parent.
      toDelete.push(e);
    } else {
      // Foreign inserted row (different parent or manually inserted) — stop
      // to avoid collateral damage.
      break;
    }
  }

  if (continuationTexts.length === 0) {
    return "No continuation rows found to merge.";
  }

  // Restore parent text — original parent text + each continuation
  const allTexts = [parent.translated_text, ...continuationTexts];
  parent.translated_text = allTexts.filter(Boolean).join(" ");

  // Delete in reverse order so deleteInsertedEntryForever's dependency
  // check sees the chain from the leaves backward.
  for (const ent of [...toDelete].reverse()) {
    deleteInsertedEntryForever(ent);
  }

  tab.isDirty = true;
  store.statusMessage = `Unsplit — merged ${continuationTexts.length + 1} lines back into parent`;
  return null;
}

/** Visible display length for a single dialogue entry (furigana-aware, strips 「」). */
function singleEntryVisibleLength(e: DialogueEntry): number {
  // Pull these helpers from types.ts to avoid a circular import.
  // For display-furigana, only the kanji portion before "[reading]" occupies space.
  let baseLen: number;
  if (e.label === "display-furigana") {
    const match = e.translated_text.match(/^(.+?)\s*\[.+?\]$/);
    baseLen = match ? match[1].length : e.translated_text.length;
  } else {
    baseLen = e.translated_text.length;
  }
  // Strip 「」 brackets (don't count toward in-game display)
  let adjusted = baseLen;
  if (e.translated_text.startsWith("「")) adjusted--;
  if (e.translated_text.endsWith("」")) adjusted--;
  return Math.max(0, adjusted);
}

/** Find a representative end-text-line opcode in the tab's entries to use as a template. */
function findEndTextLineTemplate(): {
  opcode: number;
  label: string;
  arguments: { arg_type: number; raw_data: number; decoded_string?: string }[];
} | null {
  // end-text-line (opcode 0x6F) has well-defined arguments: 1 arg, type 0,
  // raw_data 0 (verified across scenes). We synthesize it directly rather than
  // requiring an existing 0x6F to clone — so line-splitting works even in BINs
  // with no native line breaks (rare; e.g. auto-wrap SYS4 scenes). A synthetic
  // 0x6F is structurally identical to a cloned one, so this is safe.
  return {
    opcode: 0x6F,
    label: "end-text-line",
    arguments: [{ arg_type: 0, raw_data: 0 }],
  };
}

/**
 * Count how many display lines are already present in the current text box
 * starting at the group's start position. Walks backward from `groupStartIdx`
 * looking for a box boundary (wait-for-input or new-box-open bridge opcode),
 * counting each end-text-line opcode encountered along the way (each ETL = 1
 * completed line). The group itself starts the next line (count + 1).
 */
function countLinesAlreadyInBox(groupStartIdx: number): number {
  const tab = store.activeTab;
  if (!tab) return 0;
  const OP_NEW_BOX_OPEN = 0x1f5;
  const BOX_BOUNDARY_LABELS = new Set(["wait-for-input"]);

  let etlCount = 0;
  for (let i = groupStartIdx - 1; i >= 0; i--) {
    const e = tab.entries[i];
    if (e.deleted) continue;
    if (e.opcode === OP_NEW_BOX_OPEN) break; // bridge to new box — stop counting
    if (BOX_BOUNDARY_LABELS.has(e.label)) break;
    if (e.label === "end-text-line") etlCount++;
  }
  return etlCount;
}

/**
 * For a line group whose combined length exceeds the cap, insert
 * `end-text-line` opcodes between entries to split the group into multiple
 * display lines. If the resulting line count would exceed
 * `store.maxLinesPerBox`, insert a **bridge** (new text box) instead of an
 * end-text-line at that boundary, so the overflow continues in a new box.
 *
 * Greedy algorithm:
 *   1. Walk through the group accumulating per-line length.
 *   2. When the next entry would overflow the current line, mark a break
 *      before it.
 *   3. When breaks would push the box past max-lines-per-box, mark that
 *      break as a "bridge break" (insert bridge instead of ETL).
 *
 * Returns the number of insertions made, or an error string.
 */
export async function splitOverflowingGroup(
  groupMember: DialogueEntry,
  lineCharCap: number,
  maxLinesPerBox: number,
): Promise<string | number> {
  const tab = store.activeTab;
  if (!tab) return "No active tab.";
  if (lineCharCap <= 0) return "No line cap set.";
  if (maxLinesPerBox < 1) return "Max lines per box must be ≥ 1.";

  const memberIdx = tab.entries.indexOf(groupMember);
  if (memberIdx === -1) return "Group member entry not found.";

  const isGroupable = (e: DialogueEntry) =>
    (e.label === "show-text" || e.label === "display-furigana") && !e.deleted;

  let start = memberIdx;
  while (start > 0 && isGroupable(tab.entries[start - 1])) start--;

  let end = memberIdx;
  while (end < tab.entries.length - 1 && isGroupable(tab.entries[end + 1])) end++;

  const groupEntries = tab.entries.slice(start, end + 1);
  const totalLength = groupEntries.reduce(
    (sum, e) => sum + singleEntryVisibleLength(e),
    0,
  );
  if (totalLength <= lineCharCap) {
    return "Group is not over the line cap.";
  }

  const etlTemplate = findEndTextLineTemplate();
  if (!etlTemplate) {
    return "No end-text-line opcode found in this BIN to use as a template.";
  }

  // Plan breaks: { beforeId, kind: 'etl' | 'bridge' }
  // The group becomes its OWN first line. Existing lines in this box (from
  // earlier ETLs) count toward the per-box limit too.
  const linesAlreadyInBox = countLinesAlreadyInBox(start);
  type Break = { beforeId: number; kind: "etl" | "bridge" };
  const breaks: Break[] = [];

  let runningLen = 0;
  let currentBoxLines = linesAlreadyInBox + 1; // group starts a new line in this box

  for (let i = 0; i < groupEntries.length; i++) {
    const e = groupEntries[i];
    const len = singleEntryVisibleLength(e);
    if (i === 0) {
      runningLen = len;
      continue;
    }
    if (runningLen + len > lineCharCap) {
      // Need a break before this entry. Decide ETL or bridge based on box capacity.
      if (currentBoxLines + 1 > maxLinesPerBox) {
        breaks.push({ beforeId: e.instruction_index, kind: "bridge" });
        currentBoxLines = 1; // new box, this entry starts line 1
      } else {
        breaks.push({ beforeId: e.instruction_index, kind: "etl" });
        currentBoxLines++;
      }
      runningLen = len;
    } else {
      runningLen += len;
    }
  }

  if (breaks.length === 0) {
    return "Could not find a valid split point (one entry alone exceeds the cap).";
  }

  // If any break is a bridge, fetch the bridge clone template up front.
  let bridgeInstrs: SerializedInstruction[] | null = null;
  if (breaks.some((b) => b.kind === "bridge")) {
    try {
      bridgeInstrs = await getBridgeClone(tab.id);
    } catch (err) {
      return `Need to bridge into a new box but bridge template unavailable: ${err}`;
    }
  }

  // Apply breaks in REVERSE order so earlier insertion indices stay stable.
  let inserted = 0;
  for (const brk of [...breaks].reverse()) {
    const beforeIdx = tab.entries.findIndex(
      (e) => e.instruction_index === brk.beforeId,
    );
    if (beforeIdx <= 0) continue;
    const prevEntry = tab.entries[beforeIdx - 1];

    if (brk.kind === "etl") {
      const etlArgs: SerializedArg[] = etlTemplate.arguments.map((a) => ({
        arg_type: a.arg_type,
        raw_value: a.raw_data,
        string_value: a.decoded_string,
      }));
      insertInstructionVisually(
        prevEntry.instruction_index,
        etlTemplate.label,
        etlTemplate.opcode,
        etlArgs,
        true,
      );
      inserted++;
    } else {
      // Bridge: first close the prior line with an ETL, then insert the
      // bridge instruction sequence. Inserts go in order; after each one,
      // the previous index for the next insert is the latest inserted entry.
      let lastInsertedIdx = prevEntry.instruction_index;
      const etlArgs: SerializedArg[] = etlTemplate.arguments.map((a) => ({
        arg_type: a.arg_type,
        raw_value: a.raw_data,
        string_value: a.decoded_string,
      }));
      lastInsertedIdx = insertInstructionVisually(
        lastInsertedIdx,
        etlTemplate.label,
        etlTemplate.opcode,
        etlArgs,
        true,
      );
      inserted++;
      for (const bi of bridgeInstrs!) {
        const args: SerializedArg[] = bi.arguments.map((a) => ({
          arg_type: a.arg_type,
          raw_value: a.raw_data,
          string_value: a.decoded_string,
        }));
        lastInsertedIdx = insertInstructionVisually(
          lastInsertedIdx,
          bi.label,
          bi.opcode,
          args,
          true,
        );
        inserted++;
      }
    }
  }

  tab.isDirty = true;
  const etlCount = breaks.filter((b) => b.kind === "etl").length;
  const bridgeCount = breaks.filter((b) => b.kind === "bridge").length;
  const parts: string[] = [];
  if (etlCount > 0) parts.push(`${etlCount} line break(s)`);
  if (bridgeCount > 0) parts.push(`${bridgeCount} new box(es)`);
  store.statusMessage = `Group split: inserted ${parts.join(" + ")}`;
  return inserted;
}
