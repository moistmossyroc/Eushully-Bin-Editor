<script lang="ts">
  import type { DialogueEntry } from "../lib/types";
  import { displaySafe } from "../lib/types";
  import { store } from "../lib/stores.svelte";
  import { insertInstructionVisually, deleteInsertedEntryForever } from "../lib/insertions";
  import { confirmDialog } from "../lib/confirm";
  import { clampToViewport } from "../lib/clampMenu";
  import { toastError } from "../lib/toast";

  interface Props {
    entry: DialogueEntry;
    index: number;
    onInsertInstruction: (afterIndex: number) => void;
    charCount?: { original: number; translated: number };
  }

  let { entry, index, onInsertInstruction, charCount }: Props = $props();

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

  function toggleDeleted() {
    const tab = store.activeTab;
    if (!tab) return;
    tab.entries[index].deleted = !tab.entries[index].deleted;
    tab.isDirty = true;
    showContextMenu = false;
  }

  function insertSimple(label: string, opcode: number, args: { arg_type: number; raw_value: number; string_value?: string }[], isContext: boolean) {
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
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<tr
  class="context-row"
  class:deleted={entry.deleted}
  class:block-boundary={entry.label === "wait-for-input"}
  oncontextmenu={handleContextMenu}
>
  <td class="cell-label" title="Instruction #{entry.instruction_index}">
    <span class="label-text">{entry.label}</span>
    <span class="index-badge">#{entry.instruction_index}</span>
  </td>
  {#if charCount}
    <td class="cell-original">
      <span class="char-count">{charCount.original} chars</span>
    </td>
    <td class="cell-translated" class:over-cap={store.translatedCharCap > 0 && charCount.translated > store.translatedCharCap}>
      <span class="char-count">{charCount.translated} chars</span>
    </td>
    <td class="cell-notes"></td>
  {:else}
    <td class="cell-original" colspan="3">
      <span class="context-text">{displaySafe(entry.original_text)}</span>
    </td>
  {/if}
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
  .context-row {
    border-bottom: 1px solid #222240;
    background: rgba(0, 0, 0, 0.15);
  }

  .context-row.deleted {
    opacity: 0.35;
    text-decoration: line-through;
  }

  /* Block-boundary row (wait-for-input). Mirrors the engine's "click to
   * advance" beat — the natural unit of dialog memory matching. A solid
   * accent line under the row makes the boundary scannable so the user can
   * see at a glance which rows are grouped into one block.
   * Block bounds aren't user-configurable; they come from the BIN itself. */
  .context-row.block-boundary {
    background: linear-gradient(to bottom, rgba(108, 155, 210, 0.04), rgba(108, 155, 210, 0.10));
    border-bottom: 2px solid #6c9bd2;
  }
  .context-row.block-boundary .label-text {
    color: #6c9bd2;
    font-weight: 600;
  }

  td {
    padding: 4px 10px;
    vertical-align: middle;
    font-size: 0.8rem;
    border-right: 1px solid #222240;
  }

  .cell-label {
    width: 140px;
    min-width: 140px;
    max-width: 140px;
    color: #5a6a8a;
    font-family: "Consolas", "Courier New", monospace;
    font-size: 0.75rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .label-text {
    display: block;
  }

  .index-badge {
    display: block;
    font-size: 0.65rem;
    color: #444;
    margin-top: 1px;
  }

  .cell-original {
    color: #666;
    font-style: italic;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .context-text {
    opacity: 0.7;
  }

  .cell-translated {
    color: #666;
    border-right: 1px solid #222240;
  }

  .cell-notes {
    border-right: none;
  }

  .char-count {
    font-size: 0.75rem;
    color: #6c9bd2;
    font-family: "Consolas", "Courier New", monospace;
  }

  .cell-translated.over-cap {
    background: rgba(220, 50, 50, 0.15);
  }

  .cell-translated.over-cap .char-count {
    color: #e05050;
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
