/**
 * Project-load integrity for inserted instructions (box bridges, split lines).
 *
 * Background — the LaDEA "box opens but text is invisible until the backlog"
 * bug: projects are saved with `resolveInsertionsForExport` output (after_index
 * flattened to the real anchor, each anchor group in REVERSE visual order) and,
 * historically, no link back to the visual row. Reload relinked each inserted
 * instruction to the first unused visual row with the same label+opcode. A box
 * bridge repeats opcodes with different args (u004213E0 0…1, u0041A7B0 2/1,
 * mov ×4, call ×3), so the reversed save order crossed the links and the export
 * emitted every same-opcode arg in reverse — ending the bridge on
 * `u004213E0 0` (window hidden) right before the box-open.
 *
 * Three layers, all pure (no store / IPC), so they're unit-testable in Node:
 *  1. Exact relink via the persisted `synthetic_index` (new projects).
 *  2. Legacy relink: reverse each saved anchor group and zip it with the run of
 *     synthetic rows that follows the anchor — the inverse of the save order.
 *     Falls back to the old greedy match per group only if the shapes disagree.
 *  3. Template repair: any inserted run whose opcode sequence matches the scene's
 *     bridge template but whose args are a same-opcode PERMUTATION of it (i.e.
 *     a project already re-saved in the scrambled state) is rewritten from the
 *     template.
 */
import type { DialogueEntry, InsertedInstruction, SerializedArg, SerializedInstruction } from "./types";

export type RelinkMethod = "exact" | "legacy" | "greedy";

export interface RelinkResult {
  /** Same objects as the input, with `_syntheticIndex` (re)assigned. */
  inserted: InsertedInstruction[];
  /** Which strategy linked the most instructions (for diagnostics). */
  method: RelinkMethod;
  /** Instructions that couldn't be linked to any visual row. */
  unlinkedCount: number;
}

const OP_END_TEXT_LINE = 0x6f;

const sameShape = (ins: InsertedInstruction, entry: DialogueEntry): boolean =>
  ins.label === entry.label && ins.opcode === entry.opcode;

/**
 * Greedy fallback (the original behaviour): first unused synthetic row with the
 * same label+opcode. Only used when exact/legacy linking can't apply.
 */
function linkGreedy(
  instructions: InsertedInstruction[],
  syntheticRows: DialogueEntry[],
  usedIndices: Set<number>,
): void {
  for (const ins of instructions) {
    const match = syntheticRows.find((e) => sameShape(ins, e) && !usedIndices.has(e.instruction_index));
    if (!match) continue;
    ins._syntheticIndex = match.instruction_index;
    usedIndices.add(match.instruction_index);
  }
}

/**
 * Collect, per anchor (a real, non-negative instruction index), the run of
 * synthetic rows that immediately follows that anchor's row in visual order.
 * Every synthetic row chained after anchor A sits in this contiguous run.
 */
function syntheticRunsByAnchor(entries: DialogueEntry[]): Map<number, DialogueEntry[]> {
  const runs = new Map<number, DialogueEntry[]>();
  let currentAnchor: number | null = null;
  for (const entry of entries) {
    if (entry.instruction_index >= 0) {
      currentAnchor = entry.instruction_index;
      continue;
    }
    if (currentAnchor === null) continue;
    const run = runs.get(currentAnchor) ?? [];
    run.push(entry);
    runs.set(currentAnchor, run);
  }
  return runs;
}

/**
 * Relink loaded inserted instructions to their visual rows.
 * Mutates only `_syntheticIndex` on the given instruction objects.
 */
export function relinkInsertedInstructions(
  entries: DialogueEntry[],
  inserted: InsertedInstruction[],
): RelinkResult {
  const syntheticRows = entries.filter((e) => e.instruction_index < 0);
  const syntheticIds = new Set(syntheticRows.map((e) => e.instruction_index));
  const usedIndices = new Set<number>();
  for (const ins of inserted) delete ins._syntheticIndex;

  // 1. Exact: persisted link present and pointing at a real visual row.
  const canLinkExactly = (ins: InsertedInstruction): boolean =>
    ins.synthetic_index !== undefined && syntheticIds.has(ins.synthetic_index);
  if (inserted.length > 0 && inserted.every(canLinkExactly)) {
    for (const ins of inserted) {
      ins._syntheticIndex = ins.synthetic_index;
      usedIndices.add(ins.synthetic_index!);
    }
    return { inserted, method: "exact", unlinkedCount: 0 };
  }

  // 2. Legacy: saved groups are in reverse visual order per resolved anchor.
  const runs = syntheticRunsByAnchor(entries);
  const groups = new Map<number, InsertedInstruction[]>();
  for (const ins of inserted) {
    const group = groups.get(ins.after_index) ?? [];
    group.push(ins);
    groups.set(ins.after_index, group);
  }
  const unresolvedGroups: InsertedInstruction[] = [];
  let legacyLinked = 0;
  for (const [anchor, group] of groups) {
    const run = runs.get(anchor);
    const visualOrderGroup = [...group].reverse();
    const shapesAgree =
      anchor >= 0 &&
      run !== undefined &&
      run.length === visualOrderGroup.length &&
      visualOrderGroup.every((ins, i) => sameShape(ins, run[i]) && !usedIndices.has(run[i].instruction_index));
    if (!shapesAgree) {
      unresolvedGroups.push(...group);
      continue;
    }
    visualOrderGroup.forEach((ins, i) => {
      ins._syntheticIndex = run![i].instruction_index;
      usedIndices.add(run![i].instruction_index);
    });
    legacyLinked += group.length;
  }

  // 3. Greedy for whatever the legacy pass couldn't place.
  linkGreedy(unresolvedGroups, syntheticRows, usedIndices);

  const unlinkedCount = inserted.filter((ins) => ins._syntheticIndex === undefined).length;
  const method: RelinkMethod = legacyLinked >= inserted.length - legacyLinked ? "legacy" : "greedy";
  return { inserted, method, unlinkedCount };
}

/** Comparable signature of one argument list. */
const argsSignature = (args: { arg_type: number; value: number; text?: string }[]): string =>
  args.map((a) => `${a.arg_type}:${a.value}:${a.text ?? ""}`).join(",");

const insertedArgsSignature = (ins: InsertedInstruction): string =>
  argsSignature(ins.arguments.map((a) => ({ arg_type: a.arg_type, value: a.raw_value, text: a.string_value })));

const templateArgsSignature = (instr: SerializedInstruction): string =>
  argsSignature(instr.arguments.map((a) => ({ arg_type: a.arg_type, value: a.raw_data, text: a.decoded_string })));

const templateArgsToInserted = (instr: SerializedInstruction): SerializedArg[] =>
  instr.arguments.map((a) => ({ arg_type: a.arg_type, raw_value: a.raw_data, string_value: a.decoded_string }));

/** True when `run` holds exactly the template's args per opcode, just reordered. */
function isSameOpcodePermutation(run: InsertedInstruction[], template: SerializedInstruction[]): boolean {
  const bag = new Map<string, number>();
  template.forEach((t) => {
    const key = `${t.opcode}|${templateArgsSignature(t)}`;
    bag.set(key, (bag.get(key) ?? 0) + 1);
  });
  for (const ins of run) {
    const key = `${ins.opcode}|${insertedArgsSignature(ins)}`;
    const remaining = bag.get(key) ?? 0;
    if (remaining === 0) return false;
    bag.set(key, remaining - 1);
  }
  return true;
}

export interface BridgeRepairResult {
  /** Bridge runs whose args were rewritten from the template. */
  repairedRuns: number;
  /** Instructions whose args changed. */
  repairedInstructions: number;
}

/**
 * Rewrite scrambled bridge runs from the scene's bridge template.
 *
 * Matches the template WITHOUT its leading end-text-line (insertTextBoxBridge
 * reuses an existing line-break 0x6F as the close, so the leading 0x6F may be
 * an original/other row). A run is repaired only when its opcode sequence
 * equals the template core AND its args are a same-opcode permutation of the
 * template's — so a deliberately different bridge (borrowed donor, older
 * template) is never touched. Mutates `arguments` on linked instructions.
 */
export function repairScrambledBridges(
  entries: DialogueEntry[],
  inserted: InsertedInstruction[],
  bridgeTemplate: SerializedInstruction[],
): BridgeRepairResult {
  const result: BridgeRepairResult = { repairedRuns: 0, repairedInstructions: 0 };
  const firstCore = bridgeTemplate.findIndex((t) => t.opcode !== OP_END_TEXT_LINE);
  if (firstCore === -1) return result;
  const core = bridgeTemplate.slice(firstCore);
  if (core.length < 2) return result; // a lone box-open has nothing to scramble

  const bySynthetic = new Map<number, InsertedInstruction>();
  for (const ins of inserted) if (ins._syntheticIndex !== undefined) bySynthetic.set(ins._syntheticIndex, ins);

  // Linked instruction per visual position (null for original / unlinked rows).
  const linkedAt: (InsertedInstruction | null)[] = entries.map((e) =>
    e.instruction_index < 0 ? bySynthetic.get(e.instruction_index) ?? null : null,
  );

  let position = 0;
  while (position + core.length <= entries.length) {
    const run: InsertedInstruction[] = [];
    for (let k = 0; k < core.length; k++) {
      const ins = linkedAt[position + k];
      if (!ins || ins.opcode !== core[k].opcode) break;
      run.push(ins);
    }
    if (run.length !== core.length) {
      position++;
      continue;
    }
    const alreadyCorrect = run.every((ins, k) => insertedArgsSignature(ins) === templateArgsSignature(core[k]));
    if (!alreadyCorrect && isSameOpcodePermutation(run, core)) {
      run.forEach((ins, k) => {
        if (insertedArgsSignature(ins) === templateArgsSignature(core[k])) return;
        ins.arguments = templateArgsToInserted(core[k]);
        result.repairedInstructions++;
      });
      result.repairedRuns++;
    }
    position += core.length;
  }
  return result;
}
