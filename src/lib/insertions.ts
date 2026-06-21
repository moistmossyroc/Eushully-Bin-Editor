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

/** Opcodes for the native AGE text-box break. */
const OP_WAIT_FOR_INPUT = 0x72;
const OP_END_TEXT_LINE = 0x6f;
const OP_NEW_BOX_OPEN = 0x1f5;

/**
 * Insert a text-box BREAK after `entry` so the text that follows renders in a
 * NEW box. Takes the pre-fetched `bridge` clone (the BIN's box-transition: it
 * carries the engine "render/advance box" call and ends in new-box-open 0x1F5)
 * and the source box's `speakerSetup` opcodes (which re-assert the name plate +
 * color on the new box — Sankai establishes the speaker per-box, so a
 * continuation box would otherwise fall back to narrator).
 *
 * Layout produced (matching the original order — setup BEFORE the box-open):
 *   entry, wait-for-input, end-text-line(close), <bridge middle incl 0x186ba>,
 *   <speakerSetup>, new-box-open(0x1F5), next show-text
 *
 * Reuses an existing line-break 0x6F after `entry` as the close (skips the
 * bridge's leading 0x6F) to avoid a blank first line. Returns null on success.
 */
export function insertTextBoxBridge(
  entry: DialogueEntry,
  bridge: SerializedInstruction[],
  speakerSetup: SerializedInstruction[],
): string | null {
  const tab = store.activeTab;
  if (!tab) return "No active tab.";
  if (!bridge || bridge.length === 0) return "No usable textbox bridge in this BIN.";

  const idx = tab.entries.findIndex((e) => e.instruction_index === entry.instruction_index);
  if (idx === -1) return "Entry not found.";
  const next = tab.entries[idx + 1];

  const argsOf = (bi: SerializedInstruction): SerializedArg[] =>
    bi.arguments.map((a) => ({ arg_type: a.arg_type, raw_value: a.raw_data, string_value: a.decoded_string }));
  const ins = (i: number, bi: SerializedInstruction) =>
    insertInstructionVisually(i, bi.label, bi.opcode, argsOf(bi), true);

  // The bridge's last instruction is the box-open (0x1F5); speaker setup must
  // go right before it. Split the bridge into [middle…] + open.
  const open = bridge[bridge.length - 1];
  const middleEnd = bridge.length - 1;

  // 1. wait-for-input (the click) immediately after the entry.
  let lastIdx = insertInstructionVisually(
    entry.instruction_index, "wait-for-input", OP_WAIT_FOR_INPUT, [{ arg_type: 0, raw_value: 0 }], true,
  );

  // 2. Bridge middle (close + render-box call …). Reuse an existing line-break
  //    0x6F as the close when present (skip the bridge's leading 0x6F).
  let bridgeStart = 0;
  if (next != null && next.opcode === OP_END_TEXT_LINE && bridge[0].opcode === OP_END_TEXT_LINE) {
    bridgeStart = 1;
    lastIdx = next.instruction_index; // anchor the rest after the existing 0x6F
  }
  for (let k = bridgeStart; k < middleEnd; k++) lastIdx = ins(lastIdx, bridge[k]);

  // 3. Speaker/character setup (re-assert name plate + color), before the open.
  for (const s of speakerSetup) lastIdx = ins(lastIdx, s);

  // 4. The box-open (0x1F5) — the next existing show-text renders in a fresh box.
  ins(lastIdx, open);

  tab.isDirty = true;
  store.statusMessage = "Inserted box break — following text starts a new box (click to advance)";
  return null;
}

/**
 * Paginate an over-long text box: insert a native box break (wait-for-input +
 * end-text-line) after every `maxLinesPerBox`-th display line, so a box that
 * wrapped past its line limit splits into multiple boxes the player clicks
 * through (mirroring how the original script paginates long speech).
 *
 * `boxStartEntry` is the box's first dialogue entry (from `findBoxOverflows`).
 * Display lines are counted the same way as the box-overflow detector: a run of
 * non-empty show-text/display-furigana is one line; `end-text-line` ends it.
 *
 * Returns the number of box breaks inserted, or an error string.
 */
export async function paginateBox(boxStartEntry: DialogueEntry, maxLinesPerBox: number): Promise<string | number> {
  const tab = store.activeTab;
  if (!tab) return "No active tab.";
  if (maxLinesPerBox < 1) return "Max lines per box must be ≥ 1.";

  const startIdx = tab.entries.indexOf(boxStartEntry);
  if (startIdx === -1) return "Box start not found.";

  // Walk the box, recording the LAST content entry of each completed line.
  const lineLastEntry: DialogueEntry[] = [];
  let curLast: DialogueEntry | null = null;
  for (let i = startIdx; i < tab.entries.length; i++) {
    const e = tab.entries[i];
    if (e.deleted) continue;
    if (e.label === "wait-for-input" || e.opcode === OP_NEW_BOX_OPEN) break; // box end
    if (e.label === "end-text-line") {
      if (curLast) { lineLastEntry.push(curLast); curLast = null; }
      continue;
    }
    if (e.label === "show-text" || e.label === "display-furigana") {
      if (e.translated_text.trim() === "") continue;
      curLast = e;
    }
  }
  if (curLast) lineLastEntry.push(curLast); // trailing line (no 0x6F before box end)

  // Balanced distribution: split into the MINIMUM number of boxes, with lines
  // spread as evenly as possible — so 4 lines/3-max → 2+2 (not 3+1), 7 → 3+2+2,
  // never an orphan single-line box. Each box stays within maxLinesPerBox.
  const total = lineLastEntry.length;
  const numBoxes = Math.ceil(total / maxLinesPerBox);
  const breakAfter: DialogueEntry[] = [];
  if (numBoxes > 1) {
    const base = Math.floor(total / numBoxes);
    const rem = total % numBoxes; // first `rem` boxes get one extra line
    let cum = 0;
    for (let b = 0; b < numBoxes - 1; b++) {
      cum += base + (b < rem ? 1 : 0);
      breakAfter.push(lineLastEntry[cum - 1]); // break after this box's last line
    }
  }
  if (breakAfter.length === 0) return "Box already fits the line limit.";

  // Fetch the box-transition bridge (carries the engine box-advance call) once.
  let bridge: SerializedInstruction[];
  try {
    bridge = await getBridgeClone(tab.id);
  } catch (err) {
    return `No usable textbox bridge in this BIN: ${err}`;
  }
  // Speaker carry-over to continuation boxes is NOT possible: in Sankai the
  // name plate + color + portrait are all established per-box by the voice
  // (play-voice) block, which crashes when cloned and sets nothing without the
  // voice. So continuation boxes render in narrator color (text is correct).
  // (`getSpeakerSetup`/`get_speaker_setup` are kept but unused — see the
  // sankai_line_break_handling memory for the full investigation.)
  const speakerSetup: SerializedInstruction[] = [];

  // Insert in REVERSE so earlier entries' positions stay valid.
  for (const e of [...breakAfter].reverse()) {
    const err = insertTextBoxBridge(e, bridge, speakerSetup);
    if (typeof err === "string") return err; // bridge unavailable etc.
  }
  tab.isDirty = true;
  store.statusMessage = `Paginated box into ${breakAfter.length + 1} boxes`;
  return breakAfter.length;
}

/**
 * For a line group whose combined length exceeds the per-line cap, insert
 * `end-text-line` opcodes between entries to split the group into multiple
 * display LINES. This is **line-only**: it never opens a new text box — adding
 * a box is the separate, explicit `insertTextBoxBridge` action. (Engines like
 * Sankai paginate a long run of lines themselves; where an engine needs a hard
 * box break, the user inserts a bridge manually.)
 *
 * Greedy: walk the group accumulating per-line length; when the next entry
 * would overflow the current line, mark a break before it.
 *
 * Returns the number of line breaks inserted, or an error string.
 */
export function splitOverflowingGroup(
  groupMember: DialogueEntry,
  lineCharCap: number,
): string | number {
  const tab = store.activeTab;
  if (!tab) return "No active tab.";
  if (lineCharCap <= 0) return "No line cap set.";

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

  // Plan line breaks: the instruction_index of each entry that should start a
  // new display line (line-only — never a box bridge).
  const breakBeforeIds: number[] = [];
  let runningLen = 0;
  for (let i = 0; i < groupEntries.length; i++) {
    const len = singleEntryVisibleLength(groupEntries[i]);
    if (i === 0) {
      runningLen = len;
      continue;
    }
    if (runningLen + len > lineCharCap) {
      breakBeforeIds.push(groupEntries[i].instruction_index);
      runningLen = len;
    } else {
      runningLen += len;
    }
  }

  if (breakBeforeIds.length === 0) {
    return "Could not find a valid split point (one entry alone exceeds the cap).";
  }

  // Insert ETLs in REVERSE order so earlier insertion indices stay stable.
  let inserted = 0;
  for (const beforeId of [...breakBeforeIds].reverse()) {
    const beforeIdx = tab.entries.findIndex((e) => e.instruction_index === beforeId);
    if (beforeIdx <= 0) continue;
    const prevEntry = tab.entries[beforeIdx - 1];
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
  }

  tab.isDirty = true;
  store.statusMessage = `Group split: inserted ${inserted} line break(s)`;
  return inserted;
}
