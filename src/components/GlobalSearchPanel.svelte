<script lang="ts">
  /**
   * Side panel that searches all open tabs' entries for a query string.
   * Used for post-translation QC — finding all instances of a name or term
   * across multiple BINs without manually scrolling each tab.
   *
   * - Searches both `original_text` (JP) and `translated_text` (EN).
   * - Case-insensitive substring match.
   * - Results grouped by tab. Click → switch tab, scroll to row, brief flash.
   * - Phase 2 (find-and-replace) wired below — opt-in per result via checkbox
   *   with confirmation dialog before applying.
   */
  import { store } from "../lib/stores.svelte";
  import type { DialogueEntry } from "../lib/types";
  import ResizeHandle from "./ResizeHandle.svelte";
  import { persistAllSettingsDebounced } from "../lib/settings";

  interface Result {
    tabIndex: number;
    tabName: string;
    rowIndex: number;
    rowLabel: string;
    rowId: number;
    field: "original" | "translated";
    text: string;
    matchStart: number;
    matchLen: number;
  }

  let query = $state("");
  let replaceWith = $state("");
  let searchScope = $state<"both" | "original" | "translated">("both");
  let caseSensitive = $state(false);
  /** Restrict search to the active tab. Useful for the "rename a character
   *  and sweep this BIN" workflow without cross-tab noise. */
  let activeTabOnly = $state(false);
  let selectedIds = $state<Set<string>>(new Set());

  function rowKey(r: Result): string {
    return `${r.tabIndex}:${r.rowIndex}:${r.field}`;
  }

  let results = $derived.by<Result[]>(() => {
    const q = query.trim();
    if (!q) return [];
    const needle = caseSensitive ? q : q.toLowerCase();
    const out: Result[] = [];
    // Active-tab-only scope shrinks iteration to one tab. Indices must remain
    // store-relative so jump-to-row's `tabIndex` switches to the right tab.
    const tabIndices = activeTabOnly && store.activeTab
      ? [store.activeTabIndex]
      : store.tabs.map((_, i) => i);
    for (const ti of tabIndices) {
      const tab = store.tabs[ti];
      const tabName = tab.sourceBinFilename || `Tab ${ti + 1}`;
      for (let ri = 0; ri < tab.entries.length; ri++) {
        const e = tab.entries[ri];
        if (e.deleted || e.is_context_row) continue;
        if (searchScope === "both" || searchScope === "original") {
          const idx = findIndex(e.original_text, needle, caseSensitive);
          if (idx >= 0) {
            out.push({
              tabIndex: ti, tabName, rowIndex: ri,
              rowLabel: e.label, rowId: e.instruction_index,
              field: "original", text: e.original_text,
              matchStart: idx, matchLen: q.length,
            });
          }
        }
        if (searchScope === "both" || searchScope === "translated") {
          const idx = findIndex(e.translated_text, needle, caseSensitive);
          if (idx >= 0) {
            out.push({
              tabIndex: ti, tabName, rowIndex: ri,
              rowLabel: e.label, rowId: e.instruction_index,
              field: "translated", text: e.translated_text,
              matchStart: idx, matchLen: q.length,
            });
          }
        }
      }
    }
    return out;
  });

  function findIndex(haystack: string, needle: string, cs: boolean): number {
    if (!haystack || !needle) return -1;
    return cs
      ? haystack.indexOf(needle)
      : haystack.toLowerCase().indexOf(needle);
  }

  // Group results by tab for display.
  let grouped = $derived.by(() => {
    const groups = new Map<number, { tabName: string; items: Result[] }>();
    for (const r of results) {
      let g = groups.get(r.tabIndex);
      if (!g) {
        g = { tabName: r.tabName, items: [] };
        groups.set(r.tabIndex, g);
      }
      g.items.push(r);
    }
    return [...groups.entries()].map(([tabIndex, { tabName, items }]) => ({ tabIndex, tabName, items }));
  });

  function jumpTo(r: Result) {
    store.activeTabIndex = r.tabIndex;
    const tab = store.tabs[r.tabIndex];
    if (!tab) return;
    tab.selectedRowIndex = r.rowIndex;
    // Bump the nonce so even the same row clicked twice re-fires the scroll.
    store.pendingScrollRequest = {
      tabId: tab.id,
      rowIndex: r.rowIndex,
      nonce: Date.now(),
    };
  }

  function toggleSelected(r: Result) {
    const k = rowKey(r);
    if (selectedIds.has(k)) selectedIds.delete(k);
    else selectedIds.add(k);
    selectedIds = new Set(selectedIds); // trigger reactivity
  }

  function selectAll() {
    selectedIds = new Set(results.map(rowKey));
  }
  function selectNone() {
    selectedIds = new Set();
  }

  let selectedCount = $derived(selectedIds.size);

  function performReplace() {
    if (!query.trim() || selectedCount === 0) return;
    const confirmed = confirm(
      `Replace "${query}" with "${replaceWith}" in ${selectedCount} location(s) across ${grouped.length} tab(s)?\n\n` +
      `This cannot be undone in bulk — only the editor's per-cell history applies.`,
    );
    if (!confirmed) return;
    applyReplace(results.filter((r) => selectedIds.has(rowKey(r))));
  }

  /** One-click replace across EVERY match (all tabs), no per-result selection. */
  function performReplaceAll() {
    if (!query.trim() || results.length === 0) return;
    const confirmed = confirm(
      `Replace "${query}" with "${replaceWith}" in ALL ${results.length} match(es) across ${grouped.length} tab(s)?\n\n` +
      `This cannot be undone in bulk — only the editor's per-cell history applies.`,
    );
    if (!confirmed) return;
    applyReplace(results);
  }

  function applyReplace(targets: Result[]) {
    const q = query;
    const cs = caseSensitive;
    const repl = replaceWith;
    let applied = 0;

    // Group changes by tab so we mutate each tab's entries array once.
    const byTab = new Map<number, Result[]>();
    for (const r of targets) {
      let arr = byTab.get(r.tabIndex);
      if (!arr) { arr = []; byTab.set(r.tabIndex, arr); }
      arr.push(r);
    }

    // Snapshot affected tabs first so the bulk replace can be undone in one click.
    const affected = [...byTab.keys()].map((ti) => store.tabs[ti]).filter(Boolean);
    store.captureBulkUndo(`Replace "${q}"`, affected);

    for (const [tabIndex, items] of byTab) {
      const tab = store.tabs[tabIndex];
      if (!tab) continue;
      // Materialize the entries array fresh so Svelte 5's keyed each blocks
      // see the change. Mutate per-row text fields, then assign.
      const updated = tab.entries.map((e, i) => {
        const rowItems = items.filter(it => it.rowIndex === i);
        if (rowItems.length === 0) return e;
        const next = { ...e };
        for (const it of rowItems) {
          if (it.field === "original") {
            next.original_text = replaceInString(next.original_text, q, repl, cs);
          } else {
            next.translated_text = replaceInString(next.translated_text, q, repl, cs);
          }
          applied++;
        }
        return next;
      });
      tab.entries = updated;
      tab.isDirty = true;
    }

    store.statusMessage = `Replaced ${applied} occurrence${applied === 1 ? "" : "s"}.`;
    selectedIds = new Set();
  }

  /** Replace ALL occurrences of needle in haystack; case-sensitive or not. */
  function replaceInString(haystack: string, needle: string, repl: string, cs: boolean): string {
    if (!needle) return haystack;
    if (cs) return haystack.split(needle).join(repl);
    // Case-insensitive: walk through and replace
    let out = "";
    let i = 0;
    const lower = haystack.toLowerCase();
    const nLower = needle.toLowerCase();
    while (i < haystack.length) {
      const idx = lower.indexOf(nLower, i);
      if (idx < 0) { out += haystack.slice(i); break; }
      out += haystack.slice(i, idx) + repl;
      i = idx + needle.length;
    }
    return out;
  }

  // Snippet helpers — show ~40 chars of context around the match.
  function snippetBefore(r: Result): string {
    const start = Math.max(0, r.matchStart - 30);
    const prefix = start > 0 ? "…" : "";
    return prefix + r.text.slice(start, r.matchStart);
  }
  function snippetMatch(r: Result): string {
    return r.text.slice(r.matchStart, r.matchStart + r.matchLen);
  }
  function snippetAfter(r: Result): string {
    const end = Math.min(r.text.length, r.matchStart + r.matchLen + 30);
    const suffix = end < r.text.length ? "…" : "";
    return r.text.slice(r.matchStart + r.matchLen, end) + suffix;
  }
</script>

{#if store.globalSearchPanelOpen}
  <aside class="search-panel" style="width: {store.panelWidths.search}px">
    <ResizeHandle
      width={store.panelWidths.search}
      minWidth={280}
      maxWidth={900}
      onResize={(w) => (store.panelWidths.search = w)}
      onResizeEnd={() => persistAllSettingsDebounced()}
    />
    <header class="panel-header">
      <h3>Global Search</h3>
      <button
        class="close-btn"
        onclick={() => (store.globalSearchPanelOpen = false)}
        title="Close (Ctrl+Shift+F to toggle)"
      >×</button>
    </header>

    <div class="search-fields">
      <input
        type="text"
        class="query-input"
        placeholder="Search across all open tabs..."
        bind:value={query}
        autofocus
      />
      <input
        type="text"
        class="replace-input"
        placeholder="Replace with... (optional)"
        bind:value={replaceWith}
      />

      <div class="options">
        <label class="opt">
          <input type="radio" bind:group={searchScope} value="both" /> Both
        </label>
        <label class="opt">
          <input type="radio" bind:group={searchScope} value="original" /> Original (JP)
        </label>
        <label class="opt">
          <input type="radio" bind:group={searchScope} value="translated" /> Translated (EN)
        </label>
        <label class="opt">
          <input type="checkbox" bind:checked={caseSensitive} /> Case-sensitive
        </label>
        <label class="opt" title="Restrict search/replace to the currently active tab only">
          <input type="checkbox" bind:checked={activeTabOnly} disabled={store.tabs.length <= 1} />
          Active tab only
        </label>
      </div>
    </div>

    <div class="results-header">
      <span class="result-count">
        {results.length} result{results.length === 1 ? "" : "s"}
        {#if grouped.length > 1} across {grouped.length} tabs{/if}
      </span>
      {#if replaceWith.length > 0 && results.length > 0}
        <div class="replace-actions">
          <button class="link" onclick={selectAll}>Select all</button>
          <button class="link" onclick={selectNone}>None</button>
          <button
            class="replace-btn"
            disabled={selectedCount === 0}
            onclick={performReplace}
          >
            Replace selected ({selectedCount})
          </button>
          <button
            class="replace-btn"
            onclick={performReplaceAll}
            title="Replace every match across all shown tabs in one step"
          >
            Replace all ({results.length})
          </button>
        </div>
      {/if}
    </div>

    <div class="results-scroll">
      {#each grouped as group (group.tabIndex)}
        <div class="group">
          <div class="group-header">{group.tabName} <span class="group-count">({group.items.length})</span></div>
          {#each group.items as r}
            <div
              class="result-row"
              class:active={selectedIds.has(rowKey(r))}
            >
              {#if replaceWith.length > 0}
                <input
                  type="checkbox"
                  class="result-check"
                  checked={selectedIds.has(rowKey(r))}
                  onchange={() => toggleSelected(r)}
                  title="Include in replace"
                />
              {/if}
              <button class="result-body" onclick={() => jumpTo(r)} title="Jump to this row">
                <div class="result-meta">
                  <span class="result-label">{r.rowLabel}</span>
                  <span class="result-id">#{r.rowId}</span>
                  <span class="result-field {r.field}">{r.field === "original" ? "JP" : "EN"}</span>
                </div>
                <div class="result-snippet">
                  {snippetBefore(r)}<mark>{snippetMatch(r)}</mark>{snippetAfter(r)}
                </div>
              </button>
            </div>
          {/each}
        </div>
      {/each}
      {#if query.trim() && results.length === 0}
        <p class="empty">No matches.</p>
      {:else if !query.trim()}
        <p class="empty">Type to search across all open tabs.</p>
      {/if}
    </div>
  </aside>
{/if}

<style>
  .search-panel {
    position: relative;
    flex-shrink: 0;
    background: #15152a;
    border-left: 1px solid #2a2a4a;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid #2a2a4a;
  }
  .panel-header h3 {
    margin: 0;
    font-size: 0.95rem;
    color: #c0c0e0;
  }
  .close-btn {
    background: transparent;
    border: none;
    color: #8a8aaa;
    font-size: 1.2rem;
    cursor: pointer;
    padding: 0 4px;
  }
  .close-btn:hover { color: #fff; }

  .search-fields {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 14px;
    border-bottom: 1px solid #2a2a4a;
  }
  .query-input, .replace-input {
    padding: 6px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    font-size: 0.85rem;
    outline: none;
    font-family: inherit;
  }
  .query-input:focus, .replace-input:focus { border-color: #5a7aff; }

  .options {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    font-size: 0.75rem;
    color: #8a8aaa;
  }
  .opt { display: inline-flex; align-items: center; gap: 4px; cursor: pointer; }

  .results-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 14px;
    border-bottom: 1px solid #2a2a4a;
    font-size: 0.75rem;
    color: #8a8aaa;
  }
  .replace-actions { display: flex; align-items: center; gap: 6px; }
  .link {
    background: transparent;
    border: none;
    color: #8aa0ff;
    font-size: 0.75rem;
    cursor: pointer;
    padding: 0 2px;
  }
  .link:hover { color: #b0c0ff; }
  .replace-btn {
    padding: 4px 10px;
    border: 1px solid #6a4040;
    border-radius: 3px;
    background: #2a1a1a;
    color: #e8a0a0;
    font-size: 0.75rem;
    cursor: pointer;
  }
  .replace-btn:hover:not(:disabled) { background: #3a2020; }
  .replace-btn:disabled { opacity: 0.5; cursor: not-allowed; }

  .results-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 6px 0;
  }

  .group { margin-bottom: 8px; }
  .group-header {
    padding: 6px 14px;
    font-size: 0.75rem;
    font-weight: 600;
    color: #8aa0ff;
    background: #1a1a35;
    border-bottom: 1px solid #2a2a4a;
  }
  .group-count { color: #555577; font-weight: normal; }

  .result-row {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 4px 10px;
    border-bottom: 1px solid #1a1a30;
  }
  .result-row.active { background: #1f1f3a; }
  .result-row:hover { background: #1d1d36; }

  .result-check {
    margin-top: 6px;
    cursor: pointer;
  }
  .result-body {
    flex: 1;
    min-width: 0;
    background: transparent;
    border: none;
    text-align: left;
    color: #c0c0e0;
    cursor: pointer;
    padding: 4px 0;
  }
  .result-meta {
    display: flex;
    gap: 6px;
    font-size: 0.7rem;
    color: #6a6a8a;
    margin-bottom: 2px;
  }
  .result-label { color: #8aa0ff; }
  .result-id { color: #6a6a8a; }
  .result-field {
    margin-left: auto;
    padding: 0 5px;
    border-radius: 2px;
    font-weight: 600;
  }
  .result-field.original { background: #2a1f10; color: #d8a86f; }
  .result-field.translated { background: #102a20; color: #6fd8a4; }

  .result-snippet {
    font-size: 0.8rem;
    line-height: 1.3;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .result-snippet mark {
    background: #ffd060;
    color: #000;
    padding: 0 1px;
    border-radius: 2px;
  }

  .empty {
    padding: 20px 14px;
    text-align: center;
    color: #555577;
    font-size: 0.85rem;
  }

  /* Scrollbar */
</style>
