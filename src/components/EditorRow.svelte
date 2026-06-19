<script lang="ts">
  import type { DialogueEntry, InsertedInstruction, SerializedArg } from "../lib/types";
  import { furiganaDisplayLength, stripQuoteBrackets, displaySafe } from "../lib/types";
  import { store } from "../lib/stores.svelte";
  import { insertInstructionVisually, deleteInsertedEntryForever, isAutoSplitContinuation, applySplitPlan, unsplitContinuation } from "../lib/insertions";
  import { optimizeEntry, autoSplitEntry } from "../lib/ipc";
  import * as blockMemoryMod from "../lib/blockMemory";
  import { confirmDialog } from "../lib/confirm";
  import { clampToViewport } from "../lib/clampMenu";
  import { toastError } from "../lib/toast";

  interface Props {
    entry: DialogueEntry;
    index: number;
    onInsertInstruction: (afterIndex: number) => void;
    lineGroupCount?: { original: number; translated: number };
  }

  let { entry, index, onInsertInstruction, lineGroupCount }: Props = $props();

  /** Only these labels allow editing the translated text column */
  const EDITABLE_LABELS = new Set([
    "show-text",
    "display-furigana",
    "comment",
    "set-string",
    "draw-string",
  ]);

  /** Parente (Unity Lua) tabs: every extracted row is a translatable string
   *  literal, regardless of its enclosing-function label. */
  let isParente = $derived(store.activeTab?.format === "parente");
  let isTranslatable = $derived(isParente || EDITABLE_LABELS.has(entry.label));

  /** Hover state for the 💡 chip's preview popup — shows the full block fill
   *  before the user clicks, so they can verify what will be applied. */
  let memoryPreviewOpen = $state(false);

  /** Block-level translation memory: a 💡 chip appears on the FIRST empty
   *  row of a block when the ENTIRE block's fingerprint (speaker + originals
   *  in order) matches a fully-translated block in another tab. Clicking
   *  the chip fills every empty row in the block from the matched source —
   *  per-position copy across the parallel block entries.
   *
   *  Block-level (not per-row) so ambiguous short lines like "ええ" don't
   *  get the wrong translation suggested just because another tab happens
   *  to have the same Japanese in an unrelated conversation. */
  let blockMemoryMatch = $derived.by<{
    sourceBlock: import("../lib/blockMemory").BlockInfo;
    targetBlock: import("../lib/blockMemory").BlockInfo;
    tabName: string;
    emptyCount: number;
  } | null>(() => {
    if (!isTranslatable) return null;
    if (entry.translated_text.trim().length > 0) return null;
    const tab = store.activeTab;
    if (!tab) return null;

    const { getBlockMaps, blockHasEmptyRows, isBlockFullyTranslated } =
      blockMemoryMod;
    const maps = getBlockMaps(tab);
    const blockId = maps.blockIdOf.get(entry.instruction_index);
    if (blockId === undefined) return null;
    const myBlock = maps.blocks.get(blockId);
    if (!myBlock) return null;
    if (!blockHasEmptyRows(myBlock)) return null;

    // Only show the chip on the FIRST empty row in the block — avoids
    // visually crowding multiple chips that all do the same fill.
    const firstEmpty = myBlock.entries.find((e) => !e.translated_text.trim());
    if (!firstEmpty || firstEmpty.instruction_index !== entry.instruction_index) return null;

    // Look for a fully-translated block in any OTHER tab with the same fingerprint.
    for (const other of store.tabs) {
      if (other === tab) continue;
      const otherMaps = getBlockMaps(other);
      const otherBlockId = otherMaps.byFingerprint.get(myBlock.fingerprint);
      if (otherBlockId === undefined) continue;
      const otherBlock = otherMaps.blocks.get(otherBlockId);
      if (!otherBlock || !isBlockFullyTranslated(otherBlock)) continue;
      const emptyCount = myBlock.entries.filter((e) => !e.translated_text.trim()).length;
      return {
        sourceBlock: otherBlock,
        targetBlock: myBlock,
        tabName: other.sourceBinFilename || "another tab",
        emptyCount,
      };
    }
    return null;
  });

  function applyMemorySuggestion() {
    if (!blockMemoryMatch) return;
    const { sourceBlock, targetBlock } = blockMemoryMatch;
    // Walk source and target in lock-step. Lengths match (same fingerprint =
    // same number of editable lines in same order). Only fill empties so any
    // partial in-progress edits stay intact.
    const n = Math.min(sourceBlock.entries.length, targetBlock.entries.length);
    let filled = 0;
    for (let i = 0; i < n; i++) {
      const tgt = targetBlock.entries[i];
      if (tgt.translated_text.trim()) continue;
      tgt.translated_text = sourceBlock.entries[i].translated_text;
      filled++;
    }
    if (filled > 0) {
      const tab = store.activeTab;
      if (tab) tab.isDirty = true;
    }
  }

  /** Visible character count for this cell — strips furigana bracket content and quote brackets */
  let cellDisplayLength = $derived.by(() => {
    const baseLen = entry.label === "display-furigana"
      ? furiganaDisplayLength(entry.translated_text)
      : entry.translated_text.length;
    const baseText = entry.label === "display-furigana"
      ? (entry.translated_text.match(/^(.+?)\s*\[.+?\]$/)?.[1] ?? entry.translated_text)
      : entry.translated_text;
    return stripQuoteBrackets(baseLen, baseText);
  });

  /** Resolve speaker name from row speakers + character table */
  let speakerDisplay = $derived.by(() => {
    const tab = store.activeTab;
    if (!tab) return null;
    const ref = tab.rowSpeakers[String(entry.instruction_index)];
    if (!ref) return null;

    // If the set-string source was a literal string, use it directly
    if (ref.literal_name) return ref.literal_name;

    // Otherwise look up the source variable ID in the character table
    const char = tab.characters.find((c) => c.pointer === ref.source_raw_data);
    if (char && char.name.length > 0) return char.name;

    // Fall back to numeric ID display
    return "#" + ref.source_raw_data;
  });

  let speakerResolved = $derived.by(() => {
    const tab = store.activeTab;
    if (!tab) return false;
    const ref = tab.rowSpeakers[String(entry.instruction_index)];
    if (!ref) return false;
    if (ref.literal_name) return true;
    return tab.characters.some((c) => c.pointer === ref.source_raw_data && c.name.length > 0);
  });

  /** Rows that actually render as dialogue in-game — only these have an
   *  in-game line cap concept. Comments, set-string, draw-string aren't
   *  shown to the player so their length doesn't matter. */
  const RENDERED_DIALOGUE_LABELS = new Set(["show-text", "display-furigana"]);
  let isRenderedDialogue = $derived(RENDERED_DIALOGUE_LABELS.has(entry.label));

  /** Whether this row's translation exceeds the line character cap */
  let isOverLimit = $derived(
    isRenderedDialogue &&
    entry.translated_text.trim() !== "" &&
    store.translatedLineCharCap > 0 &&
    (lineGroupCount?.translated ?? cellDisplayLength) > store.translatedLineCharCap
  );

  let isAutoSplitRow = $derived(isAutoSplitContinuation(entry));

  // Line-splitting within a box needs no bridge (we synthesize end-text-line),
  // so it's offered whenever a row overflows the line cap. A missing bridge only
  // blocks spilling into an extra box — that case is handled at split time with
  // a clear error if the text won't fit one box.
  let canAutoSplit = $derived(
    isOverLimit &&
    store.autoSplitEnabled &&
    store.activeTab?.bridgeTemplateQuality !== null &&
    !isAutoSplitRow
  );

  let splitting = $state(false);

  async function handleAutoSplit() {
    const tab = store.activeTab;
    if (!tab || splitting) return;
    splitting = true;
    try {
      const plan = await autoSplitEntry(
        tab.id,
        entry.translated_text,
        entry.instruction_index,
        store.translatedLineCharCap,
        store.translatedCharCap,
        store.maxLinesPerBox,
      );
      const result = applySplitPlan(plan);
      if (typeof result === "string") {
        store.statusMessage = result;
      }
    } catch (err) {
      store.statusMessage = `Auto-split failed: ${err}`;
    }
    splitting = false;
    showContextMenu = false;
  }

  async function handleUnsplit() {
    showContextMenu = false;
    const error = unsplitContinuation(entry);
    if (error) {
      store.statusMessage = error;
    }
  }

  let optimizing = $state(false);

  async function optimizeLine() {
    const tab = store.activeTab;
    if (!tab || !store.apiKeyConfigured) return;

    const lineCap = store.translatedLineCharCap;
    const blockCap = store.translatedCharCap;
    if (lineCap === 0 && blockCap === 0) return;

    optimizing = true;
    showContextMenu = false;

    const currentLen = lineGroupCount?.translated ?? cellDisplayLength;
    const previousTranslation = entry.translated_text;

    // Build context window (previous 15 entries)
    const contextStart = Math.max(0, index - 15);
    const context = tab.entries.slice(contextStart, index).filter(
      (e) => !e.is_context_row && !e.deleted
    );

    // Compute block length
    let blockLength = 0;
    for (let j = index; j >= 0; j--) {
      if (tab.entries[j].label === "wait-for-input") break;
      if ((tab.entries[j].label === "show-text" || tab.entries[j].label === "display-furigana") && !tab.entries[j].deleted) {
        const len = tab.entries[j].label === "display-furigana"
          ? furiganaDisplayLength(tab.entries[j].translated_text)
          : tab.entries[j].translated_text.length;
        blockLength += stripQuoteBrackets(len, tab.entries[j].translated_text);
      }
    }

    try {
      const result = await optimizeEntry(
        entry,
        context,
        store.mergedGlossary,
        tab.claudeModel,
        lineCap,
        blockCap,
        currentLen,
        blockLength,
        store.minimalNotes,
      );

      const newLen = result.translation.length;

      let notes = `[LLM: ${tab.claudeModel}] [Optimized] ${currentLen}->${newLen} chars`;
      if (result.notes && result.notes.trim() !== "") {
        notes += ` | ${result.notes}`;
      }
      notes += ` | Pre-optimize: "${previousTranslation}"`;

      tab.entries[index].translated_text = result.translation;
      tab.entries[index].notes = notes;
      tab.isDirty = true;

      store.statusMessage = `Optimized: ${currentLen} -> ${newLen} chars`;
    } catch (err) {
      store.statusMessage = `Optimization failed: ${err}`;
    }

    optimizing = false;
  }

  let editingTranslated = $state(false);
  let editingNotes = $state(false);

  function handleTranslatedInput(e: Event) {
    const tab = store.activeTab;
    if (!tab) return;
    const target = e.target as HTMLTextAreaElement;
    tab.entries[index].translated_text = target.value;
    tab.isDirty = true;
  }

  function handleNotesInput(e: Event) {
    const tab = store.activeTab;
    if (!tab) return;
    const target = e.target as HTMLTextAreaElement;
    tab.entries[index].notes = target.value;
    tab.isDirty = true;
  }

  /** Review status tags — mutually exclusive (Reviewed vs Needs Edit), Flagged is independent */
  const REVIEW_TAGS = ["Reviewed", "Needs Edit", "Flagged"] as const;
  type ReviewTag = typeof REVIEW_TAGS[number];
  const STATUS_TAGS: ReviewTag[] = ["Reviewed", "Needs Edit"];

  function hasTag(tag: ReviewTag): boolean {
    return entry.notes.includes(`[${tag}]`);
  }

  function toggleTag(tag: ReviewTag) {
    const tab = store.activeTab;
    if (!tab) return;
    let notes = entry.notes;

    if (hasTag(tag)) {
      // Remove the tag
      notes = notes.replace(`[${tag}] `, "").replace(`[${tag}]`, "");
    } else {
      // For status tags, remove the other status tag first
      if (STATUS_TAGS.includes(tag)) {
        for (const other of STATUS_TAGS) {
          if (other !== tag) {
            notes = notes.replace(`[${other}] `, "").replace(`[${other}]`, "");
          }
        }
      }
      // Prepend the tag
      notes = `[${tag}] ${notes}`.trimEnd();
    }

    tab.entries[index].notes = notes.trim();
    tab.isDirty = true;
  }

  /** Toggle whether this entry's strings get exported. Checkbox state:
   *  checked = include (omit_export=false), unchecked = omit (omit_export=true). */
  function handleToggleOmit(checked: boolean) {
    const tab = store.activeTab;
    if (!tab) return;
    tab.entries[index].omit_export = !checked;
    tab.isDirty = true;
  }

  function handleRowClick() {
    const tab = store.activeTab;
    if (tab) tab.selectedRowIndex = index;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      editingTranslated = false;
      editingNotes = false;
    }
  }

  function toggleDeleted() {
    const tab = store.activeTab;
    if (!tab) return;
    tab.entries[index].deleted = !tab.entries[index].deleted;
    tab.isDirty = true;
  }

  function insertSimple(label: string, opcode: number, args: SerializedArg[], isContext: boolean) {
    insertInstructionVisually(entry.instruction_index, label, opcode, args, isContext);
    showContextMenu = false;
  }

  function contextMenuItems() {
    return [
      {
        label: "Insert end-text-line",
        action: () => insertSimple("end-text-line", 0x6f, [{ arg_type: 0, raw_value: 0 }], true),
      },
      {
        label: "Insert wait-for-input",
        action: () => insertSimple("wait-for-input", 0x72, [{ arg_type: 0, raw_value: 0 }], true),
      },
      {
        label: "Insert comment",
        action: () => insertSimple("comment", 0x1a7, [{ arg_type: 3, raw_value: 0, string_value: "" }], false),
      },
      {
        label: "Insert show-text / display-furigana...",
        action: () => { showContextMenu = false; onInsertInstruction(entry.instruction_index); },
      },
      ...(isOverLimit && store.apiKeyConfigured && !optimizing
        ? [
            { type: "separator" as const },
            {
              label: optimizing ? "Optimizing..." : "Optimize line (Claude)",
              action: optimizeLine,
            },
          ]
        : []),
      ...(canAutoSplit
        ? [
            {
              label: splitting ? "Splitting..." : "Auto-split into boxes",
              action: handleAutoSplit,
            },
          ]
        : []),
      ...(isAutoSplitRow
        ? [
            {
              label: "Unsplit back into parent",
              action: handleUnsplit,
            },
          ]
        : []),
      ...(entry.instruction_index >= 0
        ? [
            { type: "separator" as const },
            {
              label: "Show in BIN Viewer",
              action: () => {
                showContextMenu = false;
                const tab = store.activeTab;
                if (!tab) return;
                store.pendingBinViewerJump = {
                  tabId: tab.id,
                  instructionIndex: entry.instruction_index,
                  nonce: Date.now(),
                };
                store.binViewerOpen = true;
              },
            },
          ]
        : []),
      { type: "separator" as const },
      { label: entry.deleted ? "Restore row" : "Delete row", action: toggleDeleted },
      ...(entry.instruction_index < 0
        ? [
            {
              label: "Delete forever",
              action: async () => {
                showContextMenu = false;
                const confirmed = await confirmDialog({
                  title: "Delete forever",
                  message: `Permanently delete this inserted ${entry.label}? This cannot be undone.`,
                  confirmLabel: "Delete forever",
                  danger: true,
                });
                if (!confirmed) return;
                const error = deleteInsertedEntryForever(entry);
                if (error) toastError(error);
              },
              danger: true,
            },
          ]
        : []),
    ];
  }

  let showContextMenu = $state(false);
  let contextMenuPos = $state({ x: 0, y: 0 });
  let menuGeneration = $state(-1);

  // Close this menu when another one opens
  $effect(() => {
    const tab = store.activeTab;
    if (tab && tab.contextMenuGeneration !== menuGeneration && showContextMenu) {
      showContextMenu = false;
    }
  });

  function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    const tab = store.activeTab;
    if (!tab) return;
    // Parente rows have no BIN instructions — the insert / BIN-viewer /
    // auto-split actions don't apply, so skip the menu entirely.
    if (tab.format === "parente") return;
    // Close all other menus first
    tab.closeAllContextMenus();
    contextMenuPos = { x: e.clientX, y: e.clientY };
    showContextMenu = true;
    menuGeneration = tab.contextMenuGeneration;

    const close = () => {
      showContextMenu = false;
      window.removeEventListener("click", close);
    };
    setTimeout(() => window.addEventListener("click", close), 0);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<tr
  class="editor-row"
  class:selected={store.activeTab?.selectedRowIndex === index}
  class:deleted={entry.deleted}
  class:auto-split-continuation={isAutoSplitRow}
  data-row-index={index}
  onclick={handleRowClick}
  oncontextmenu={handleContextMenu}
>
  <td class="cell-label" title="Instruction #{entry.instruction_index}">
    <span class="label-text">{entry.label}</span>
    {#if speakerDisplay}
      <span class="speaker-badge" class:unresolved={!speakerResolved}>
        {speakerDisplay}
      </span>
    {/if}
    <span class="index-badge">#{entry.instruction_index}</span>
  </td>

  <td class="cell-original">
    <div class="text-content">{displaySafe(entry.original_text)}</div>
  </td>

  <td
    class="cell-translated"
    class:readonly={!isTranslatable}
    class:over-line-cap={isRenderedDialogue && store.translatedLineCharCap > 0 && (lineGroupCount?.translated ?? cellDisplayLength) > store.translatedLineCharCap}
  >
    {#if isTranslatable}
      <textarea
        class="cell-input"
        class:active={editingTranslated}
        value={entry.translated_text}
        oninput={handleTranslatedInput}
        onfocus={() => (editingTranslated = true)}
        onblur={() => (editingTranslated = false)}
        onkeydown={handleKeydown}
        placeholder="Translation..."
        rows="1"
      ></textarea>
      {#if entry.translated_text.length > 0}
        <span
          class="line-char-count"
          class:over-line-cap={isRenderedDialogue && store.translatedLineCharCap > 0 && (lineGroupCount?.translated ?? cellDisplayLength) > store.translatedLineCharCap}
        >{lineGroupCount?.translated ?? cellDisplayLength}</span>
      {/if}
      {#if blockMemoryMatch}
        {@const myIdx = blockMemoryMatch.targetBlock.entries.findIndex((e) => e.instruction_index === entry.instruction_index)}
        {@const myTranslation = myIdx >= 0 && myIdx < blockMemoryMatch.sourceBlock.entries.length
          ? blockMemoryMatch.sourceBlock.entries[myIdx].translated_text
          : ""}
        <div class="memory-suggest-wrap">
          <button
            type="button"
            class="memory-suggest"
            onmouseenter={() => (memoryPreviewOpen = true)}
            onmouseleave={() => (memoryPreviewOpen = false)}
            onclick={applyMemorySuggestion}
          >
            <span class="memory-icon">💡</span>
            <span class="memory-text">{myTranslation}</span>
            <span class="memory-source">— fills {blockMemoryMatch.emptyCount} row{blockMemoryMatch.emptyCount === 1 ? "" : "s"} from {blockMemoryMatch.tabName}</span>
          </button>
          {#if memoryPreviewOpen}
            <div class="memory-preview" role="tooltip">
              <div class="memory-preview-header">
                Preview · {blockMemoryMatch.emptyCount} empty row{blockMemoryMatch.emptyCount === 1 ? "" : "s"} will be filled
                <span class="memory-preview-source">from {blockMemoryMatch.tabName}</span>
              </div>
              <table class="memory-preview-table">
                <tbody>
                  {#each blockMemoryMatch.targetBlock.entries as tgt, ti}
                    {@const src = blockMemoryMatch.sourceBlock.entries[ti]}
                    {@const willFill = !tgt.translated_text.trim() && !!src && !!src.translated_text}
                    {@const isCurrent = tgt.instruction_index === entry.instruction_index}
                    <tr class:will-fill={willFill} class:already-filled={!willFill && tgt.translated_text.trim()} class:current-row={isCurrent}>
                      <td class="mp-marker">{willFill ? "+" : tgt.translated_text.trim() ? "•" : ""}</td>
                      <td class="mp-orig">{tgt.original_text}</td>
                      <td class="mp-arrow">→</td>
                      <td class="mp-trans">{willFill ? (src?.translated_text ?? "") : tgt.translated_text || "—"}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
              <div class="memory-preview-footer">
                <span class="legend"><span class="dot dot-fill"></span>will fill</span>
                <span class="legend"><span class="dot dot-keep"></span>already translated (kept)</span>
                <span class="apply-hint">Click chip to apply</span>
              </div>
            </div>
          {/if}
        </div>
      {/if}
    {:else}
      <div class="text-content readonly-text">
        {displaySafe(entry.translated_text || "")}
      </div>
    {/if}
  </td>

  <td class="cell-notes">
    {#if isTranslatable}
      <div class="review-tags">
        <button
          class="tag-btn tag-reviewed"
          class:active={hasTag("Reviewed")}
          onclick={() => toggleTag("Reviewed")}
          title="Mark as reviewed"
        >R</button>
        <button
          class="tag-btn tag-needs-edit"
          class:active={hasTag("Needs Edit")}
          onclick={() => toggleTag("Needs Edit")}
          title="Mark as needs edit"
        >E</button>
        <button
          class="tag-btn tag-flagged"
          class:active={hasTag("Flagged")}
          onclick={() => toggleTag("Flagged")}
          title="Flag for attention"
        >F</button>
        {#if entry.translated_text.trim() === "" || entry.omit_export === true}
          <label
            class="include-toggle"
            class:omitted={entry.omit_export === true}
            title={entry.omit_export === true
              ? "This row will be OMITTED on export (original text won't render in-game)."
              : "This row will be exported as-is — since it has no translation, the original Japanese will render in-game. Uncheck to omit instead."}
          >
            <input
              type="checkbox"
              checked={entry.omit_export !== true}
              onchange={(e) => handleToggleOmit((e.currentTarget as HTMLInputElement).checked)}
            />
            <span>Export</span>
          </label>
        {/if}
      </div>
    {/if}
    <textarea
      class="cell-input"
      class:active={editingNotes}
      value={entry.notes}
      oninput={handleNotesInput}
      onfocus={() => (editingNotes = true)}
      onblur={() => (editingNotes = false)}
      onkeydown={handleKeydown}
      placeholder="Notes..."
      rows="1"
    ></textarea>
  </td>
</tr>

{#if showContextMenu}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="context-menu"
    use:clampToViewport
    style="position: fixed; left: {contextMenuPos.x}px; top: {contextMenuPos.y}px; z-index: 1000;"
  >
    {#each contextMenuItems() as item}
      {#if 'type' in item && item.type === "separator"}
        <div class="context-menu-separator"></div>
      {:else}
        <div class="context-menu-item" class:danger={'danger' in item && item.danger} onclick={item.action}>
          {item.label}
        </div>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .editor-row {
    border-bottom: 1px solid #2a2a4a;
    transition: background 0.1s;
  }

  .editor-row:hover {
    background: #1e1e42;
  }

  .editor-row.selected {
    background: #1e2a4e;
  }

  .editor-row.deleted {
    opacity: 0.35;
    text-decoration: line-through;
  }

  .editor-row.auto-split-continuation {
    background: color-mix(in srgb, var(--row-bg, #1a1a2e) 94%, #4488ff 6%);
  }

  td {
    padding: 6px 10px;
    vertical-align: top;
    font-size: 0.85rem;
    border-right: 1px solid #2a2a4a;
  }

  .cell-label {
    width: 140px;
    min-width: 140px;
    max-width: 140px;
    color: #8ab4f8;
    font-family: "Consolas", "Courier New", monospace;
    font-size: 0.8rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .label-text {
    display: block;
  }

  .speaker-badge {
    display: block;
    font-size: 0.7rem;
    color: #6cc070;
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .speaker-badge.unresolved {
    color: #888;
    font-family: "Consolas", "Courier New", monospace;
  }

  .index-badge {
    display: block;
    font-size: 0.7rem;
    color: #666;
    margin-top: 2px;
  }

  .cell-original {
    width: 30%;
    color: #c0c0e0;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .cell-translated {
    width: 30%;
  }

  .cell-translated.readonly {
    color: #666;
  }

  .cell-notes {
    color: #999;
  }

  .review-tags {
    display: flex;
    gap: 2px;
    margin-bottom: 2px;
  }

  .tag-btn {
    width: 18px;
    height: 18px;
    padding: 0;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: transparent;
    color: #555;
    font-size: 0.65rem;
    font-weight: 600;
    cursor: pointer;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .tag-btn:hover {
    border-color: #6a6a8a;
    color: #999;
  }

  .tag-btn.tag-reviewed.active {
    background: #1a3a2e;
    border-color: #2a5a3e;
    color: #6fd886;
  }

  .tag-btn.tag-needs-edit.active {
    background: #3a2a1a;
    border-color: #5a3a2a;
    color: #e8a838;
  }

  .tag-btn.tag-flagged.active {
    background: #3a1a2a;
    border-color: #5a2a3a;
    color: #e06060;
  }

  .include-toggle {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 1px 5px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #1a1a3e;
    color: #a0c0e0;
    font-size: 0.65rem;
    cursor: pointer;
    user-select: none;
  }

  .include-toggle:hover {
    background: #2a2a5e;
  }

  .include-toggle input[type="checkbox"] {
    margin: 0;
    cursor: pointer;
    accent-color: #8ab4f8;
  }

  .include-toggle.omitted {
    background: #3a1a2a;
    border-color: #5a2a3a;
    color: #e06060;
  }

  .text-content {
    min-height: 1.2em;
  }

  .readonly-text {
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* Seamless inline textarea — looks like plain text until focused */
  .cell-input {
    display: block;
    width: 100%;
    min-height: 1.4em;
    padding: 0;
    margin: 0;
    border: 1px solid transparent;
    border-radius: 3px;
    background: transparent;
    color: inherit;
    font-family: inherit;
    font-size: inherit;
    line-height: 1.4;
    resize: vertical;
    outline: none;
    overflow: hidden;
    white-space: pre-wrap;
    word-break: break-word;
    field-sizing: content;
  }

  .cell-input::placeholder {
    color: #444;
  }

  .cell-input:hover {
    border-color: #3a3a5a;
  }

  .cell-input:focus,
  .cell-input.active {
    border-color: #6c9bd2;
    background: rgba(108, 155, 210, 0.06);
    overflow: auto;
  }

  .line-char-count {
    display: block;
    font-size: 0.65rem;
    color: #555;
    font-family: "Consolas", "Courier New", monospace;
    text-align: right;
    margin-top: 2px;
  }

  .cell-translated.over-line-cap {
    background: rgba(220, 160, 40, 0.12);
  }

  .line-char-count.over-line-cap {
    color: #e0a030;
  }

  /* Translation-memory suggestion chip: appears below empty translatable
   * cells when another tab has translated the same Japanese already.
   * Single click adopts the suggestion. */
  .memory-suggest-wrap {
    position: relative;
    margin-top: 3px;
  }
  .memory-suggest {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    padding: 3px 6px;
    background: rgba(120, 160, 220, 0.08);
    border: 1px dashed #3a4a7a;
    border-radius: 3px;
    color: #8aa0ff;
    font-size: 0.72rem;
    cursor: pointer;
    text-align: left;
    line-height: 1.2;
    overflow: hidden;
  }
  .memory-suggest:hover {
    background: rgba(120, 160, 220, 0.18);
    border-style: solid;
    border-color: #5a7aff;
    color: #c0d0ff;
  }
  .memory-icon { flex-shrink: 0; }
  .memory-text {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #c0c0e0;
  }
  .memory-source {
    flex-shrink: 0;
    color: #6a7aa0;
    font-size: 0.68rem;
    font-style: italic;
  }

  /* Hover preview popup — shows the entire block's source→target mapping
   * so the user can verify the fill before clicking. Positioned below the
   * chip with high z-index so it floats above sibling rows. */
  .memory-preview {
    position: absolute;
    top: 100%;
    left: 0;
    z-index: 50;
    margin-top: 4px;
    background: #15152a;
    border: 1px solid #5a7aff;
    border-radius: 4px;
    padding: 8px 10px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.55);
    min-width: 480px;
    max-width: 700px;
    max-height: 400px;
    overflow-y: auto;
    cursor: default;
  }
  .memory-preview-header {
    font-size: 0.72rem;
    color: #c0d0ff;
    font-weight: 600;
    margin-bottom: 6px;
    padding-bottom: 4px;
    border-bottom: 1px solid #2a3a6e;
  }
  .memory-preview-source {
    color: #6a7aa0;
    font-weight: 400;
    font-style: italic;
    margin-left: 6px;
  }
  .memory-preview-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.72rem;
  }
  .memory-preview-table td {
    padding: 3px 4px;
    border-bottom: 1px solid #1a1a30;
    vertical-align: top;
    line-height: 1.35;
  }
  .mp-marker {
    width: 16px;
    text-align: center;
    color: #5a7aff;
    font-weight: 700;
    font-family: monospace;
  }
  .already-filled .mp-marker { color: #4ad080; }
  .mp-orig {
    color: #d8a86f;
    width: 45%;
    word-break: break-word;
  }
  .mp-arrow {
    color: #555577;
    width: 16px;
    text-align: center;
  }
  .mp-trans {
    color: #c0d0ff;
    word-break: break-word;
  }
  .already-filled .mp-trans { color: #6fd8a4; opacity: 0.7; }
  tr.current-row {
    background: rgba(120, 160, 220, 0.15);
  }
  tr.current-row .mp-marker {
    color: #ffd060;
  }

  .memory-preview-footer {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 6px;
    padding-top: 6px;
    border-top: 1px solid #2a3a6e;
    font-size: 0.66rem;
    color: #8a8aaa;
  }
  .legend { display: inline-flex; align-items: center; gap: 4px; }
  .dot {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .dot-fill { background: #5a7aff; }
  .dot-keep { background: #4ad080; }
  .apply-hint {
    margin-left: auto;
    color: #c0c0e0;
    font-style: italic;
  }

  .context-menu {
    background: #1e1e42;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    padding: 4px 0;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
  }

  .context-menu-item {
    padding: 6px 16px;
    cursor: pointer;
    font-size: 0.85rem;
    color: #c0c0e0;
  }

  .context-menu-item:hover {
    background: #2a2a5e;
  }

  .context-menu-separator {
    height: 1px;
    background: #2a2a4a;
    margin: 4px 0;
  }

  .context-menu-item.danger {
    color: #f06060;
  }

  .context-menu-item.danger:hover {
    background: #3a1a1a;
  }
</style>
