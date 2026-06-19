<script lang="ts">
  import { untrack } from "svelte";
  import EditorRow from "./EditorRow.svelte";
  import ContextRow from "./ContextRow.svelte";
  import { store } from "../lib/stores.svelte";
  import { furiganaDisplayLength, stripQuoteBrackets } from "../lib/types";

  interface Props {
    onInsertInstruction: (afterIndex: number) => void;
  }

  let { onInsertInstruction }: Props = $props();

  let containerEl = $state<HTMLDivElement | null>(null);

  /** Active tab's entries — explicit derived to avoid getter-chain reactivity edge cases
   *  during high-frequency translation event mutations */
  let activeEntries = $derived(store.tabs[store.activeTabIndex]?.entries ?? []);

  /** For each wait-for-input row index, compute char counts of all entries since the previous wait-for-input.
   *  Strips 「」 brackets which don't count toward in-game display length. */
  let charCounts = $derived.by(() => {
    const tab = store.activeTab;
    if (!tab) return new Map<number, { original: number; translated: number }>();
    const counts = new Map<number, { original: number; translated: number }>();
    let originalAcc = 0;
    let translatedAcc = 0;

    for (let i = 0; i < tab.entries.length; i++) {
      const e = tab.entries[i];
      if (e.label === "wait-for-input") {
        counts.set(i, { original: originalAcc, translated: translatedAcc });
        originalAcc = 0;
        translatedAcc = 0;
      } else if ((e.label === "show-text" || e.label === "display-furigana") && !e.deleted) {
        const origLen = e.label === "display-furigana"
          ? furiganaDisplayLength(e.original_text)
          : e.original_text.length;
        const transLen = e.label === "display-furigana"
          ? furiganaDisplayLength(e.translated_text)
          : e.translated_text.length;
        originalAcc += stripQuoteBrackets(origLen, e.original_text);
        translatedAcc += stripQuoteBrackets(transLen, e.translated_text);
      }
    }
    return counts;
  });

  /** For each show-text/display-furigana row, compute the total translated chars
   *  across all entries in the same line group (bounded by end-text-line).
   *  Also keyed on end-text-line rows themselves for display. */
  let lineGroupCounts = $derived.by(() => {
    const tab = store.activeTab;
    if (!tab) return new Map<number, { original: number; translated: number }>();
    const counts = new Map<number, { original: number; translated: number }>();
    let groupIndices: number[] = [];
    let originalAcc = 0;
    let translatedAcc = 0;
    let originalLine = "";
    let translatedLine = "";

    function flushGroup(endIdx?: number) {
      const origCount = stripQuoteBrackets(originalAcc, originalLine);
      const transCount = stripQuoteBrackets(translatedAcc, translatedLine);
      for (const idx of groupIndices) {
        counts.set(idx, { original: origCount, translated: transCount });
      }
      if (endIdx !== undefined) {
        counts.set(endIdx, { original: origCount, translated: transCount });
      }
      groupIndices = [];
      originalAcc = 0;
      translatedAcc = 0;
      originalLine = "";
      translatedLine = "";
    }

    for (let i = 0; i < tab.entries.length; i++) {
      const e = tab.entries[i];
      if (e.label === "end-text-line") {
        flushGroup(i);
      } else if ((e.label === "show-text" || e.label === "display-furigana") && !e.deleted) {
        groupIndices.push(i);
        if (e.label === "display-furigana") {
          const origDisplay = e.original_text.match(/^(.+?)\s*\[.+?\]$/);
          const transDisplay = e.translated_text.match(/^(.+?)\s*\[.+?\]$/);
          originalLine += origDisplay ? origDisplay[1] : e.original_text;
          translatedLine += transDisplay ? transDisplay[1] : e.translated_text;
          originalAcc += furiganaDisplayLength(e.original_text);
          translatedAcc += furiganaDisplayLength(e.translated_text);
        } else {
          originalLine += e.original_text;
          translatedLine += e.translated_text;
          originalAcc += e.original_text.length;
          translatedAcc += e.translated_text.length;
        }
      }
    }
    // Trailing entries after last end-text-line
    if (groupIndices.length > 0) flushGroup();
    return counts;
  });

  function handleKeydown(e: KeyboardEvent) {
    const tab = store.activeTab;
    if (!tab || tab.selectedRowIndex === null) return;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      const next = findNextDialogueRow(tab.selectedRowIndex, 1);
      if (next !== null) {
        tab.selectedRowIndex = next;
        scrollToRow(next);
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      const prev = findNextDialogueRow(tab.selectedRowIndex, -1);
      if (prev !== null) {
        tab.selectedRowIndex = prev;
        scrollToRow(prev);
      }
    }
  }

  function findNextDialogueRow(from: number, direction: 1 | -1): number | null {
    const tab = store.activeTab;
    if (!tab) return null;
    let idx = from + direction;
    while (idx >= 0 && idx < tab.entries.length) {
      if (!tab.entries[idx].is_context_row && tab.entries[idx].label !== "end-text-line") return idx;
      idx += direction;
    }
    return null;
  }

  function scrollToRow(index: number) {
    if (!containerEl) return;
    // Find the actual row DOM element by data attribute
    const row = containerEl.querySelector(`[data-row-index="${index}"]`);
    if (row) {
      row.scrollIntoView({ block: "nearest" });
    }
  }

  /** Brief highlight flash on a row — used by global search jump-to. Adds a
   *  CSS class, removes it after the animation completes. */
  function flashRow(index: number) {
    if (!containerEl) return;
    const row = containerEl.querySelector(`[data-row-index="${index}"]`) as HTMLElement | null;
    if (!row) return;
    row.classList.remove("jump-flash"); // restart animation if class is already there
    // Force a reflow so removing+adding actually re-triggers the animation.
    void row.offsetWidth;
    row.classList.add("jump-flash");
    setTimeout(() => row.classList.remove("jump-flash"), 1500);
  }

  /** Persist the active tab's scroll offset on every scroll so switching tabs
   *  can restore it. Cheap — just a number write to the tab proxy. */
  function handleScroll() {
    if (!containerEl) return;
    const tab = store.activeTab;
    if (tab) tab.scrollTop = containerEl.scrollTop;
  }

  // Restore the incoming tab's saved scroll position on tab switch. Depends
  // only on activeTabIndex; the saved scrollTop is read untracked so this
  // doesn't re-fire on every scroll event (which would fight the user). The
  // {#key activeTabIndex} table rebuild happens first, so we wait a microtask.
  $effect(() => {
    const idx = store.activeTabIndex;
    void idx;
    const target = untrack(() => store.tabs[store.activeTabIndex]?.scrollTop ?? 0);
    queueMicrotask(() => {
      if (containerEl) containerEl.scrollTop = target;
    });
  });

  // Double-clicking the active tab requests a scroll-to-top. Reset both the
  // DOM and the remembered offset so it doesn't snap back on next switch.
  $effect(() => {
    const req = store.scrollToTopRequest;
    if (!req) return;
    const tab = store.activeTab;
    if (!tab || tab.id !== req.tabId) return;
    queueMicrotask(() => {
      if (containerEl) containerEl.scrollTop = 0;
      tab.scrollTop = 0;
      if (store.scrollToTopRequest === req) store.scrollToTopRequest = null;
    });
  });

  // React to global-search jump requests. The nonce ensures clicking the same
  // row twice still triggers a re-scroll + flash.
  $effect(() => {
    const req = store.pendingScrollRequest;
    if (!req) return;
    const tab = store.activeTab;
    if (!tab || tab.id !== req.tabId) return;
    // Run after the DOM has had a chance to render the (possibly newly-active) tab.
    queueMicrotask(() => {
      scrollToRow(req.rowIndex);
      // Slight delay to let scroll settle before flashing.
      setTimeout(() => flashRow(req.rowIndex), 50);
      // Consume the request — clear after we've acted on it.
      if (store.pendingScrollRequest === req) {
        store.pendingScrollRequest = null;
      }
    });
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div
  class="editor-table-container"
  bind:this={containerEl}
  onscroll={handleScroll}
>
  {#key store.activeTabIndex}
    <table class="editor-table">
      <thead>
        <tr>
          <th class="th-label">Function Call</th>
          <th class="th-original">Original Text</th>
          <th class="th-translated">Translated Text</th>
          <th class="th-notes">Notes</th>
        </tr>
      </thead>
      <tbody>
        {#each activeEntries as entry, index (entry.instruction_index)}
          {#if entry.is_context_row || entry.label === "end-text-line"}
            <ContextRow {entry} {index} {onInsertInstruction} charCount={charCounts.get(index)} />
          {:else}
            <EditorRow {entry} {index} {onInsertInstruction} lineGroupCount={lineGroupCounts.get(index)} />
          {/if}
        {/each}
      </tbody>
    </table>
  {/key}
</div>

<style>
  .editor-table-container {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    min-height: 0;
  }

  .editor-table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
  }

  thead {
    position: sticky;
    top: 0;
    z-index: 10;
  }

  th {
    padding: 8px 10px;
    text-align: left;
    font-size: 0.8rem;
    font-weight: 600;
    color: #8ab4f8;
    background: #16213e;
    border-bottom: 2px solid #3a3a5a;
    border-right: 1px solid #2a2a4a;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .th-label {
    width: 140px;
  }

  .th-original {
    width: 30%;
  }

  .th-translated {
    width: 30%;
  }

  .th-notes {
    /* takes remaining space */
  }

  /* Brief amber flash when a row is jumped-to from global search.
     `:global` because the row elements live in EditorRow.svelte. */
  :global(.jump-flash) {
    animation: jumpFlash 1.4s ease-out;
  }
  @keyframes jumpFlash {
    0%   { background-color: rgba(255, 200, 90, 0.55); }
    100% { background-color: transparent; }
  }
</style>
