// Dependency-free unit test for insertionRelink.ts (no TS test runner in repo).
// Run: npm run test:relink   (Node ≥22.6: --experimental-strip-types)
import assert from "node:assert/strict";
import { relinkInsertedInstructions, repairScrambledBridges } from "./insertionRelink.ts";
import type { DialogueEntry, InsertedInstruction, SerializedInstruction } from "./types";

// A LaDEA-shaped bridge: same opcodes repeat with DIFFERENT args.
const BRIDGE: [number, string, number[]][] = [
  [0x21b, "u004213E0", [0]], [0x71, "u0041A7B0", [2]], [0x71, "u0041A7B0", [1]],
  [0x8f, "call", [0x1d845]], [0x8f, "call", [0x1d89a]], [0x21b, "u004213E0", [1]], [0x1f5, "u00416120", []],
];
const template: SerializedInstruction[] = [
  { opcode: 0x6f, label: "end-text-line", arguments: [{ arg_type: 0, raw_data: 0 }] },
  ...BRIDGE.map(([opcode, label, vals]) => ({ opcode, label, arguments: vals.map((v) => ({ arg_type: 0, raw_data: v })) })),
];

const entry = (instruction_index: number, opcode: number, label: string): DialogueEntry => ({
  instruction_index, opcode, label, original_text: "", translated_text: "", notes: "[Inserted]",
  is_context_row: true, string_arg_indices: [], deleted: false,
});

/** Scene with two anchors, each followed by one inserted bridge; returns entries + in-session instructions. */
function buildScene(): { entries: DialogueEntry[]; session: InsertedInstruction[] } {
  const entries: DialogueEntry[] = [];
  const session: InsertedInstruction[] = [];
  let synthetic = -1;
  for (const anchor of [10, 20]) {
    entries.push(entry(anchor, 0x6e, "show-text"));
    let previous = anchor;
    for (const [opcode, label, vals] of BRIDGE) {
      entries.push(entry(synthetic, opcode, label));
      session.push({ after_index: previous, opcode, label, arguments: vals.map((v) => ({ arg_type: 0, raw_value: v })), _syntheticIndex: synthetic });
      previous = synthetic--;
    }
  }
  return { entries, session };
}

/** What the legacy save wrote: resolved anchors, reverse visual order per group, no link. */
const legacySave = (session: InsertedInstruction[]): InsertedInstruction[] => {
  const anchorOf = (i: number) => (i < 7 ? 10 : 20);
  return session
    .map((ins, i) => ({ ...ins, after_index: anchorOf(i), _syntheticIndex: undefined, _order: i }))
    .sort((a, b) => a.after_index - b.after_index || b._order - a._order)
    .map(({ _order, ...rest }) => structuredClone(rest));
};
const argsInVisualOrder = (entries: DialogueEntry[], inserted: InsertedInstruction[]) => {
  const bySynthetic = new Map(inserted.map((ins) => [ins._syntheticIndex, ins]));
  return entries.filter((e) => e.instruction_index < 0).map((e) => bySynthetic.get(e.instruction_index)!.arguments.map((a) => a.raw_value));
};
const expectedArgs = [...BRIDGE, ...BRIDGE].map(([, , vals]) => vals);

// 1. Legacy relink restores exact links (the old greedy crossed same-opcode rows).
{
  const { entries, session } = buildScene();
  const loaded = legacySave(session);
  const result = relinkInsertedInstructions(entries, loaded);
  assert.equal(result.method, "legacy");
  assert.equal(result.unlinkedCount, 0);
  assert.deepEqual(argsInVisualOrder(entries, loaded), expectedArgs);
  assert.equal(repairScrambledBridges(entries, loaded, template).repairedRuns, 0, "correct runs must not be touched");
}

// 2. Exact relink via persisted synthetic_index.
{
  const { entries, session } = buildScene();
  const saved = session.map((ins) => ({ ...structuredClone(ins), synthetic_index: ins._syntheticIndex, _syntheticIndex: undefined }));
  assert.equal(relinkInsertedInstructions(entries, saved).method, "exact");
  assert.deepEqual(argsInVisualOrder(entries, saved), expectedArgs);
}

// 3. A scrambled legacy save (same-opcode args permuted) is repaired from the template.
{
  const { entries, session } = buildScene();
  const loaded = legacySave(session);
  relinkInsertedInstructions(entries, loaded);
  const swap = (a: InsertedInstruction, b: InsertedInstruction) => ([a.arguments, b.arguments] = [b.arguments, a.arguments]);
  const linked = new Map(loaded.map((ins) => [ins._syntheticIndex, ins]));
  swap(linked.get(-1)!, linked.get(-6)!); // u004213E0 0 ↔ 1  (the LaDEA symptom)
  swap(linked.get(-2)!, linked.get(-3)!); // u0041A7B0 2 ↔ 1
  const repair = repairScrambledBridges(entries, loaded, template);
  assert.equal(repair.repairedRuns, 1);
  assert.equal(repair.repairedInstructions, 4);
  assert.deepEqual(argsInVisualOrder(entries, loaded), expectedArgs);
}

// 4. A run that is NOT a permutation of the template (different bridge) is left alone.
{
  const { entries, session } = buildScene();
  const loaded = legacySave(session);
  relinkInsertedInstructions(entries, loaded);
  const linked = new Map(loaded.map((ins) => [ins._syntheticIndex, ins]));
  linked.get(-4)!.arguments = [{ arg_type: 0, raw_value: 0xdead }]; // foreign call target
  assert.equal(repairScrambledBridges(entries, loaded, template).repairedRuns, 0);
}

console.log("insertionRelink: all tests passed");
