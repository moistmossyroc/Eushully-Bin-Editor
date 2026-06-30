<script lang="ts">
  /**
   * Export Review — inspect and adjust which cells export. Shows, across the
   * active tab or ALL open tabs, the rows the omit decision affects (omitted +
   * empties that would export raw Japanese), with a filter to expand to every
   * editable row. Each row: an Omit toggle, its notes, and the text. Click the
   * original to expand surrounding context; double-click the original or click
   * the translation to jump to that row in the editor.
   */
  import { store } from "../lib/stores.svelte";
  import { isEditableRow, isOmitEligible } from "../lib/exportOmit";
  import type { DialogueEntry } from "../lib/types";
  import { onMount } from "svelte";

  interface Props { onClose: () => void; }
  let { onClose }: Props = $props();

  type Filter = "needs" | "omitted" | "included" | "all";
  // Seed from the persisted view so reopening resumes the same scope/filter.
  let scope = $state<"all" | "tab">(store.exportReview.scope);
  let filter = $state<Filter>(store.exportReview.filter);
  let expanded = $state<Record<string, boolean>>({});
  let scrollEl = $state<HTMLDivElement>();

  // Write scope/filter back so they survive close → reopen.
  $effect(() => {
    store.exportReview.scope = scope;
    store.exportReview.filter = filter;
  });

  // Restore the saved scroll offset once the list is mounted.
  onMount(() => {
    if (scrollEl) scrollEl.scrollTop = store.exportReview.scrollTop;
  });

  const keyOf = (tabIndex: number, rowId: number) => `${tabIndex}|${rowId}`;

  interface ReviewRow {
    rowIndex: number;
    rowId: number;
    label: string;
    original: string;
    translated: string;
    notes: string;
    omitted: boolean;
    empty: boolean;
  }
  interface TabGroup { tabIndex: number; tabId: string; tabName: string; rows: ReviewRow[]; }

  function scanned(): { tab: (typeof store.tabs)[number]; index: number }[] {
    if (scope === "tab") {
      const i = store.activeTabIndex;
      return i >= 0 && store.tabs[i] ? [{ tab: store.tabs[i], index: i }] : [];
    }
    return store.tabs.map((tab, index) => ({ tab, index }));
  }

  function matches(e: DialogueEntry, omitted: boolean, f: Filter, fileName: string): boolean {
    if (e.deleted || e.is_context_row) return false;
    if (f === "omitted") return omitted;
    if (f === "included") return isEditableRow(e, fileName) && !omitted;
    if (f === "needs") return omitted || (isOmitEligible(e, fileName) && !omitted);
    return isEditableRow(e, fileName) || omitted; // "all"
  }

  let groups = $derived.by<TabGroup[]>(() => {
    const out: TabGroup[] = [];
    for (const { tab, index } of scanned()) {
      if (!tab) continue;
      const rows: ReviewRow[] = [];
      for (let i = 0; i < tab.entries.length; i++) {
        const e = tab.entries[i];
        const omitted = e.omit_export === true;
        if (!matches(e, omitted, filter, tab.sourceBinFilename)) continue;
        rows.push({
          rowIndex: i,
          rowId: e.instruction_index,
          label: e.label,
          original: e.original_text,
          translated: e.translated_text,
          notes: e.notes ?? "",
          omitted,
          empty: e.translated_text.trim() === "",
        });
      }
      if (rows.length > 0) {
        out.push({ tabIndex: index, tabId: tab.id, tabName: tab.sourceBinFilename || "Untitled", rows });
      }
    }
    return out;
  });

  let totalRows = $derived(groups.reduce((n, g) => n + g.rows.length, 0));
  // Only reserve the notes column when something in view actually has notes.
  let anyNotes = $derived(groups.some((g) => g.rows.some((r) => r.notes.trim() !== "")));

  // All three chip counts in one pass (respecting the current scope).
  let counts = $derived.by(() => {
    let needs = 0, omitted = 0, included = 0, all = 0;
    for (const { tab } of scanned()) {
      if (!tab) continue;
      for (const e of tab.entries) {
        if (e.deleted || e.is_context_row) continue;
        const om = e.omit_export === true;
        const editable = isEditableRow(e, tab.sourceBinFilename);
        if (om) omitted++;
        if (editable && !om) included++;
        if (om || (isOmitEligible(e, tab.sourceBinFilename) && !om)) needs++;
        if (editable || om) all++;
      }
    }
    return { needs, omitted, included, all };
  });

  function setOmit(tabIndex: number, rowId: number, value: boolean) {
    const tab = store.tabs[tabIndex];
    if (!tab) return;
    tab.entries = tab.entries.map((e) => (e.instruction_index === rowId ? { ...e, omit_export: value } : e));
    tab.isDirty = true;
  }

  /** Omit / include every currently-visible row, captured as one bulk-undo. */
  function bulkVisible(value: boolean) {
    const affected = groups.filter((g) => g.rows.some((r) => r.omitted !== value));
    if (affected.length === 0) return;
    const tabs = affected.map((g) => store.tabs[g.tabIndex]).filter(Boolean);
    store.captureBulkUndo(value ? "Omit cells" : "Include cells", tabs);
    for (const g of affected) {
      const tab = store.tabs[g.tabIndex];
      if (!tab) continue;
      const ids = new Set(g.rows.map((r) => r.rowId));
      tab.entries = tab.entries.map((e) => (ids.has(e.instruction_index) ? { ...e, omit_export: value } : e));
      tab.isDirty = true;
    }
  }

  function contextRows(tabIndex: number, rowIndex: number) {
    const tab = store.tabs[tabIndex];
    if (!tab) return [];
    const start = Math.max(0, rowIndex - 3);
    const end = Math.min(tab.entries.length, rowIndex + 4);
    const out: { i: number; isTarget: boolean; label: string; original: string; translated: string }[] = [];
    for (let i = start; i < end; i++) {
      const e = tab.entries[i];
      out.push({ i, isTarget: i === rowIndex, label: e.label, original: e.original_text, translated: e.translated_text });
    }
    return out;
  }

  function toggleExpand(tabIndex: number, rowId: number) {
    const k = keyOf(tabIndex, rowId);
    expanded[k] = !expanded[k];
  }

  function jump(tabIndex: number, rowIndex: number, rowId: number) {
    const tab = store.tabs[tabIndex];
    if (!tab) return;
    store.switchTab(tabIndex);
    const idx = tab.entries.findIndex((e) => e.instruction_index === rowId);
    const target = idx >= 0 ? idx : rowIndex;
    tab.selectedRowIndex = target;
    store.pendingScrollRequest = { tabId: tab.id, rowIndex: target, nonce: Date.now() };
    onClose();
  }

  function truncate(s: string, n: number): string {
    return s.length <= n ? s : s.slice(0, n) + "…";
  }
</script>

<div class="dialog-backdrop" onclick={onClose} onkeydown={(e) => e.key === "Escape" && onClose()} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <header>
      <h2>Export Review</h2>
      <button class="close-btn" onclick={onClose}>×</button>
    </header>

    <div class="controls">
      <div class="seg">
        <button class="seg-btn" class:active={scope === "all"} onclick={() => (scope = "all")}>All tabs</button>
        <button class="seg-btn" class:active={scope === "tab"} onclick={() => (scope = "tab")}>This tab</button>
      </div>
      <div class="seg">
        <button class="seg-btn" class:active={filter === "needs"} onclick={() => (filter = "needs")}>Needs review ({counts.needs})</button>
        <button class="seg-btn omit" class:active={filter === "omitted"} onclick={() => (filter = "omitted")}>Omitted ({counts.omitted})</button>
        <button class="seg-btn incl" class:active={filter === "included"} onclick={() => (filter = "included")}>Included ({counts.included})</button>
        <button class="seg-btn" class:active={filter === "all"} onclick={() => (filter = "all")}>All editable ({counts.all})</button>
      </div>
    </div>

    {#if totalRows === 0}
      <div class="empty">Nothing to review for this filter{scope === "tab" ? " in this tab" : ""}.</div>
    {:else}
      <div class="scroll" bind:this={scrollEl} onscroll={() => { if (scrollEl) store.exportReview.scrollTop = scrollEl.scrollTop; }}>
        {#each groups as g (g.tabId)}
          <div class="tab-section">
            <div class="tab-head">{g.tabName} <span class="tab-count">{g.rows.length}</span></div>
            {#each g.rows as row (row.rowId)}
              <div class="row" class:omitted={row.omitted}>
                <label
                  class="include-toggle"
                  class:omitted={row.omitted}
                  title={row.omitted
                    ? "OMITTED — won't render in-game"
                    : row.empty
                      ? "Will export — but it's empty, so the original Japanese renders in-game"
                      : "Will export"}
                >
                  <input type="checkbox" checked={!row.omitted} onchange={(e) => setOmit(g.tabIndex, row.rowId, !e.currentTarget.checked)} />
                  <span>Export</span>
                </label>
                <div class="body">
                  <!-- svelte-ignore a11y_click_events_have_key_events -->
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <div
                    class="orig"
                    title="Click for context · double-click to open the row"
                    onclick={() => toggleExpand(g.tabIndex, row.rowId)}
                    ondblclick={() => jump(g.tabIndex, row.rowIndex, row.rowId)}
                  >
                    <span class="lbl">{row.label}</span>{truncate(row.original, 130)}
                  </div>
                  {#if row.empty}
                    <div class="trans empty-trans">(no translation — would export the original Japanese)</div>
                  {:else}
                    <!-- svelte-ignore a11y_click_events_have_key_events -->
                    <!-- svelte-ignore a11y_no_static_element_interactions -->
                    <div class="trans" title="Open this row in the editor" onclick={() => jump(g.tabIndex, row.rowIndex, row.rowId)}>
                      {truncate(row.translated, 150)}
                    </div>
                  {/if}
                  {#if expanded[keyOf(g.tabIndex, row.rowId)]}
                    <div class="context">
                      {#each contextRows(g.tabIndex, row.rowIndex) as c (c.i)}
                        <div class="ctx-row" class:target={c.isTarget}>
                          <span class="ctx-lbl">{c.label}</span>
                          <span class="ctx-jp">{truncate(c.original, 70)}</span>
                          {#if c.translated.trim()}<span class="ctx-en">{truncate(c.translated, 70)}</span>{/if}
                        </div>
                      {/each}
                    </div>
                  {/if}
                </div>
                {#if anyNotes}
                  <div class="notes-col" class:has-note={row.notes.trim() !== ""}>
                    {#if row.notes.trim()}<span class="note-ic">📝</span> {row.notes}{/if}
                  </div>
                {/if}
              </div>
            {/each}
          </div>
        {/each}
      </div>
    {/if}

    <footer>
      <span class="foot-note">{totalRows} shown</span>
      <button class="btn" onclick={() => bulkVisible(true)} disabled={totalRows === 0}>Omit all shown</button>
      <button class="btn" onclick={() => bulkVisible(false)} disabled={totalRows === 0}>Include all shown</button>
      <button class="btn" onclick={onClose}>Close</button>
    </footer>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed; inset: 0; background: rgba(0, 0, 0, 0.6);
    display: flex; align-items: center; justify-content: center; z-index: 1000;
  }
  .dialog {
    background: #1a1a3e; border: 1px solid #3a3a5a; border-radius: 6px;
    width: 980px; max-width: 94vw; max-height: 86vh;
    display: flex; flex-direction: column; box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  header { padding: 14px 20px; border-bottom: 1px solid #2a2a4a; display: flex; justify-content: space-between; align-items: center; }
  h2 { margin: 0; color: #8ab4f8; font-size: 1.05rem; }
  .close-btn { background: none; border: none; color: #a0a0c0; font-size: 1.4rem; cursor: pointer; padding: 0 6px; line-height: 1; }
  .close-btn:hover { color: #fff; }
  .controls { display: flex; gap: 16px; padding: 10px 20px; border-bottom: 1px solid #2a2a4a; flex-wrap: wrap; }
  .seg { display: flex; gap: 4px; }
  .seg-btn { padding: 3px 12px; border: 1px solid #34345a; border-radius: 999px; background: #16162e; color: #b0b0d0; cursor: pointer; font-size: 0.78rem; }
  .seg-btn:hover { background: #22224a; }
  .seg-btn.active { background: #2a3a6e; border-color: #6c9bd2; color: #cfe0ff; }
  .seg-btn.omit.active { background: #4a1f2f; border-color: #c06070; color: #f0b0b8; }
  .seg-btn.incl.active { background: #1f3a4a; border-color: #60a0c0; color: #b0e0f0; }
  .empty { padding: 32px 24px; text-align: center; color: #a0a0c0; font-size: 0.9rem; }
  .scroll { flex: 1; overflow-y: auto; min-height: 0; }
  .tab-section { border-bottom: 1px solid #20204a; }
  .tab-head { position: sticky; top: 0; z-index: 1; background: #16213e; color: #8ab4f8; padding: 6px 20px; font-size: 0.82rem; font-weight: 600; border-bottom: 1px solid #2a2a4a; }
  .tab-count { color: #6a6a8a; font-weight: 400; }
  .row { display: flex; gap: 12px; padding: 8px 20px; border-bottom: 1px solid #20204a; align-items: flex-start; }
  .row.omitted { background: #1d1626; }
  /* Same red/blue Export toggle as the editor rows: blue = will export,
     red = omitted. Scannable at a glance. */
  .include-toggle {
    display: inline-flex; align-items: center; gap: 4px; flex-shrink: 0;
    padding: 3px 9px; border: 1px solid #3a5a7a; border-radius: 4px;
    background: #16243e; color: #8ab4f8; font-size: 0.72rem; cursor: pointer; user-select: none; width: 70px;
  }
  .include-toggle:hover { background: #1e3050; }
  .include-toggle input[type="checkbox"] { margin: 0; cursor: pointer; accent-color: #8ab4f8; }
  .include-toggle.omitted { background: #3a1a2a; border-color: #5a2a3a; color: #e06060; }
  .include-toggle.omitted input[type="checkbox"] { accent-color: #e06060; }
  .body { flex: 1; min-width: 0; }
  .orig { color: #c8c8e0; font-size: 0.84rem; cursor: pointer; }
  .orig:hover { color: #fff; }
  .lbl { color: #6a6a8a; font-size: 0.7rem; margin-right: 6px; }
  .trans { color: #9aa8c8; font-size: 0.8rem; margin-top: 2px; cursor: pointer; }
  .trans:hover { color: #8ab4f8; text-decoration: underline; }
  .empty-trans { color: #8a7a5a; font-style: italic; cursor: default; }
  .notes-col {
    flex-shrink: 0; width: 240px; align-self: stretch;
    border-left: 1px solid #2a2a4a; padding-left: 12px;
    color: #c8b890; font-size: 0.78rem; line-height: 1.35; word-break: break-word;
  }
  .notes-col.has-note { background: #221d14; }
  .note-ic { opacity: 0.7; }
  .context { margin-top: 6px; border-left: 2px solid #34345a; padding-left: 10px; }
  .ctx-row { display: flex; gap: 10px; font-size: 0.74rem; padding: 1px 0; color: #80809a; }
  .ctx-row.target { color: #d0d0e8; background: #23234a; border-radius: 3px; padding: 1px 4px; }
  .ctx-lbl { color: #5a5a7a; min-width: 90px; }
  .ctx-jp { color: #b0a090; }
  .ctx-en { color: #90a0c0; }
  footer { padding: 12px 20px; border-top: 1px solid #2a2a4a; display: flex; justify-content: flex-end; align-items: center; gap: 8px; }
  .foot-note { margin-right: auto; color: #8a8aae; font-size: 0.8rem; }
  .btn { padding: 6px 14px; border: 1px solid #3a3a5a; border-radius: 4px; background: #1a1a3e; color: #c0c0e0; cursor: pointer; font-size: 0.85rem; }
  .btn:hover:not(:disabled) { background: #2a2a5e; }
  .btn:disabled { opacity: 0.5; cursor: default; }
</style>
