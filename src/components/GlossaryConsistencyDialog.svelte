<script lang="ts">
  /**
   * Glossary consistency review — ACROSS ALL TABS. Flags translated rows where a
   * glossary term's Japanese appears in the original but its English rendering is
   * missing from the translation (possible drift / forgotten term). Results are
   * grouped by tab → row; each row lists its flagged terms and can be fixed two
   * ways: edit the translation inline, or jump to the row in its tab.
   */
  import { store } from "../lib/stores.svelte";
  import { findGlossaryInconsistenciesAllTabs } from "../lib/glossaryCheck";
  import { isKatakanaTerm } from "../lib/jpMatch";
  import { toastInfo } from "../lib/toast";

  type Filter = "all" | "katakana" | "other";

  interface Props { onClose: () => void; }
  let { onClose }: Props = $props();

  interface RowGroup {
    rowIndex: number;
    rowId: number;
    original: string;
    translated: string;
    terms: { jp: string; en: string }[];
  }
  interface TabGroup {
    tabIndex: number;
    tabId: string;
    tabName: string;
    rows: RowGroup[];
  }

  let hasGlossary = $derived(
    store.globalGlossary.some((g) => g.english.trim()) ||
      store.tabs.some((t) => t.glossary.some((g) => g.english.trim())),
  );

  let flags = $derived.by(() =>
    findGlossaryInconsistenciesAllTabs(
      store.tabs.map((t, i) => ({
        id: t.id,
        name: t.sourceBinFilename || "Untitled",
        index: i,
        entries: t.entries,
        glossary: t.glossary,
      })),
      store.globalGlossary,
    ),
  );

  // Filter by term script — katakana terms are almost always names.
  let filter = $state<Filter>("all");
  let kataCount = $derived(flags.filter((f) => isKatakanaTerm(f.term)).length);
  let otherCount = $derived(flags.length - kataCount);
  let filteredFlags = $derived(
    filter === "all"
      ? flags
      : filter === "katakana"
        ? flags.filter((f) => isKatakanaTerm(f.term))
        : flags.filter((f) => !isKatakanaTerm(f.term)),
  );

  let groups = $derived.by<TabGroup[]>(() => {
    const byTab = new Map<number, TabGroup>();
    for (const f of filteredFlags) {
      const ti = f.tabIndex ?? 0;
      let tg = byTab.get(ti);
      if (!tg) {
        tg = { tabIndex: ti, tabId: f.tabId ?? "", tabName: f.tabName ?? "Untitled", rows: [] };
        byTab.set(ti, tg);
      }
      let rg = tg.rows.find((r) => r.rowId === f.rowId);
      if (!rg) {
        rg = { rowIndex: f.rowIndex, rowId: f.rowId, original: f.original, translated: f.translated, terms: [] };
        tg.rows.push(rg);
      }
      rg.terms.push({ jp: f.term, en: f.glossaryEn });
    }
    return [...byTab.values()].sort((a, b) => a.tabIndex - b.tabIndex);
  });

  let totalRows = $derived(groups.reduce((n, g) => n + g.rows.length, 0));
  let termCount = $derived(new Set(filteredFlags.map((f) => f.term)).size);

  // Inline-edit drafts keyed by `${tabIndex}|${rowId}`.
  let drafts = $state<Record<string, string>>({});
  const keyOf = (tabIndex: number, rowId: number) => `${tabIndex}|${rowId}`;
  const changed = (tabIndex: number, row: RowGroup) => {
    const v = drafts[keyOf(tabIndex, row.rowId)];
    return v !== undefined && v !== row.translated;
  };

  /** Resolve a draft key to its live tab + row + current translation. */
  function resolveDraft(key: string): { tab: (typeof store.tabs)[number]; rowIndex: number; value: string } | null {
    const sep = key.indexOf("|");
    const tabIndex = Number(key.slice(0, sep));
    const rowId = Number(key.slice(sep + 1));
    const tab = store.tabs[tabIndex];
    if (!tab) return null;
    const rowIndex = tab.entries.findIndex((e) => e.instruction_index === rowId);
    if (rowIndex < 0) return null;
    return { tab, rowIndex, value: drafts[key] };
  }

  // Total unsaved edits — counted across the FULL set (filter-independent) so
  // switching the filter never hides pending work or loses edits.
  let pendingCount = $derived.by(() => {
    let n = 0;
    for (const key of Object.keys(drafts)) {
      const r = resolveDraft(key);
      if (r && r.value !== r.tab.entries[r.rowIndex].translated_text) n++;
    }
    return n;
  });

  function applyEdit(tabIndex: number, row: RowGroup) {
    const key = keyOf(tabIndex, row.rowId);
    const value = drafts[key];
    if (value === undefined || value === row.translated) return;
    const tab = store.tabs[tabIndex];
    if (!tab) return;
    tab.entries = tab.entries.map((e, i) =>
      i === row.rowIndex ? { ...e, translated_text: value } : e,
    );
    tab.isDirty = true;
    delete drafts[key];
    toastInfo("Translation updated");
  }

  /** Commit EVERY unsaved inline edit at once (across all tabs, regardless of the
   *  active filter), captured as a single bulk-undo so the whole batch reverts in
   *  one click. */
  function applyAll() {
    const pending: { tabIndex: number; rowIndex: number; value: string }[] = [];
    for (const key of Object.keys(drafts)) {
      const r = resolveDraft(key);
      if (!r || r.value === r.tab.entries[r.rowIndex].translated_text) continue;
      pending.push({ tabIndex: store.tabs.indexOf(r.tab), rowIndex: r.rowIndex, value: r.value });
    }
    if (pending.length === 0) return;

    const affectedIdx = [...new Set(pending.map((p) => p.tabIndex))];
    const affectedTabs = affectedIdx.map((i) => store.tabs[i]).filter(Boolean);
    store.captureBulkUndo("Glossary fixes", affectedTabs);

    for (const ti of affectedIdx) {
      const tab = store.tabs[ti];
      if (!tab) continue;
      const byIndex = new Map(pending.filter((p) => p.tabIndex === ti).map((p) => [p.rowIndex, p.value]));
      tab.entries = tab.entries.map((e, i) =>
        byIndex.has(i) ? { ...e, translated_text: byIndex.get(i)! } : e,
      );
      tab.isDirty = true;
    }
    drafts = {};
    toastInfo(`Applied ${pending.length} fix${pending.length === 1 ? "" : "es"} across ${affectedIdx.length} tab${affectedIdx.length === 1 ? "" : "s"} — undo available`);
  }

  function jumpTo(tabIndex: number, row: RowGroup) {
    const tab = store.tabs[tabIndex];
    if (!tab) return;
    store.switchTab(tabIndex);
    const idx = tab.entries.findIndex((e) => e.instruction_index === row.rowId);
    const target = idx >= 0 ? idx : row.rowIndex;
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
      <h2>Glossary Consistency — all tabs</h2>
      <button class="close-btn" onclick={onClose}>×</button>
    </header>

    {#if !hasGlossary}
      <div class="empty">No glossary entries with English renderings yet — add some to check consistency.</div>
    {:else if flags.length === 0}
      <div class="empty">✓ All glossary terms are rendered consistently across every open tab.</div>
    {:else}
      <div class="filter-bar">
        <span class="flabel">Show:</span>
        <button class="fbtn" class:active={filter === "all"} onclick={() => (filter = "all")}>All ({flags.length})</button>
        <button class="fbtn" class:active={filter === "katakana"} onclick={() => (filter = "katakana")}>Katakana · names ({kataCount})</button>
        <button class="fbtn" class:active={filter === "other"} onclick={() => (filter = "other")}>Other ({otherCount})</button>
      </div>
      {#if totalRows === 0}
        <div class="empty">No {filter === "katakana" ? "katakana" : "non-katakana"} terms flagged.</div>
      {:else}
      <div class="summary">
        <strong>{totalRows}</strong> row{totalRows === 1 ? "" : "s"} across <strong>{groups.length}</strong>
        tab{groups.length === 1 ? "" : "s"} where a glossary term's English may be missing
        ({termCount} distinct term{termCount === 1 ? "" : "s"}). Fix inline or open the row — a term is
        sometimes legitimately rephrased.
      </div>
      <div class="scroll">
        {#each groups as g (g.tabId)}
          <div class="tab-section">
            <div class="tab-head">{g.tabName} <span class="tab-count">{g.rows.length}</span></div>
            {#each g.rows as row (row.rowId)}
              <div class="row">
                <div class="terms">
                  {#each row.terms as t}
                    <span class="chip"><span class="jp">{t.jp}</span> <span class="arrow">→</span> <span class="en">{t.en}</span></span>
                  {/each}
                </div>
                <div class="orig" title={row.original}>{truncate(row.original, 120)}</div>
                <div class="edit">
                  <textarea
                    rows="2"
                    value={drafts[keyOf(g.tabIndex, row.rowId)] ?? row.translated}
                    oninput={(e) => (drafts[keyOf(g.tabIndex, row.rowId)] = e.currentTarget.value)}
                  ></textarea>
                  <div class="row-actions">
                    <button class="apply" disabled={!changed(g.tabIndex, row)} onclick={() => applyEdit(g.tabIndex, row)}>Apply</button>
                    <button class="open" onclick={() => jumpTo(g.tabIndex, row)} title="Open this row in the editor">Open row →</button>
                  </div>
                </div>
              </div>
            {/each}
          </div>
        {/each}
      </div>
      {/if}
    {/if}

    <footer>
      {#if pendingCount > 0}
        <span class="pending-note">{pendingCount} edited, not yet applied</span>
      {/if}
      <button class="btn" onclick={onClose}>Close</button>
      {#if flags.length > 0}
        <button class="btn primary" disabled={pendingCount === 0} onclick={applyAll}>
          Apply all edits ({pendingCount})
        </button>
      {/if}
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
    width: 920px; max-width: 92vw; max-height: 85vh;
    display: flex; flex-direction: column; box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  header {
    padding: 14px 20px; border-bottom: 1px solid #2a2a4a;
    display: flex; justify-content: space-between; align-items: center;
  }
  h2 { margin: 0; color: #8ab4f8; font-size: 1.05rem; }
  .close-btn { background: none; border: none; color: #a0a0c0; font-size: 1.4rem; cursor: pointer; padding: 0 6px; line-height: 1; }
  .close-btn:hover { color: #fff; }
  .empty { padding: 32px 24px; text-align: center; color: #a0a0c0; font-size: 0.9rem; }
  .summary { padding: 12px 20px; font-size: 0.85rem; color: #c0c0e0; border-bottom: 1px solid #2a2a4a; }
  .summary strong { color: #f0a070; }
  .filter-bar { display: flex; align-items: center; gap: 6px; padding: 10px 20px; border-bottom: 1px solid #2a2a4a; }
  .flabel { color: #8a8aae; font-size: 0.8rem; margin-right: 2px; }
  .fbtn { padding: 3px 12px; border: 1px solid #34345a; border-radius: 999px; background: #16162e; color: #b0b0d0; cursor: pointer; font-size: 0.78rem; }
  .fbtn:hover { background: #22224a; }
  .fbtn.active { background: #2a3a6e; border-color: #6c9bd2; color: #cfe0ff; }
  .scroll { flex: 1; overflow-y: auto; min-height: 0; padding: 4px 0; }
  .tab-section { border-bottom: 1px solid #20204a; }
  .tab-head {
    position: sticky; top: 0; z-index: 1; background: #16213e; color: #8ab4f8;
    padding: 6px 20px; font-size: 0.82rem; font-weight: 600; border-bottom: 1px solid #2a2a4a;
  }
  .tab-count { color: #6a6a8a; font-weight: 400; }
  .row { padding: 10px 20px; border-bottom: 1px solid #20204a; }
  .terms { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 5px; }
  .chip { font-size: 0.76rem; background: #232348; border: 1px solid #34345a; border-radius: 4px; padding: 1px 7px; }
  .chip .jp { color: #e0c0a0; }
  .chip .arrow { color: #6a6a8a; margin: 0 3px; }
  .chip .en { color: #8ab4f8; font-weight: 600; }
  .orig { color: #9a9ab8; font-size: 0.78rem; margin-bottom: 5px; }
  .edit { display: flex; gap: 8px; align-items: flex-start; }
  textarea {
    flex: 1; resize: vertical; background: #12122c; color: #e0e0f0; border: 1px solid #34345a;
    border-radius: 4px; padding: 6px 8px; font: inherit; font-size: 0.82rem; line-height: 1.4;
  }
  textarea:focus { outline: none; border-color: #6c9bd2; }
  .row-actions { display: flex; flex-direction: column; gap: 6px; flex-shrink: 0; }
  .apply { padding: 5px 12px; border: 1px solid #3e6a54; border-radius: 4px; background: #2a4a3e; color: #c0f0d0; cursor: pointer; font-size: 0.8rem; }
  .apply:hover:not(:disabled) { background: #3a5a4e; }
  .apply:disabled { opacity: 0.4; cursor: default; }
  .open { padding: 5px 12px; border: 1px solid #3a3a5a; border-radius: 4px; background: #1a1a3e; color: #8ab4f8; cursor: pointer; font-size: 0.8rem; white-space: nowrap; }
  .open:hover { background: #2a2a5e; }
  footer { padding: 12px 20px; border-top: 1px solid #2a2a4a; display: flex; justify-content: flex-end; align-items: center; gap: 8px; }
  .pending-note { margin-right: auto; color: #f0a070; font-size: 0.8rem; }
  .btn { padding: 6px 14px; border: 1px solid #3a3a5a; border-radius: 4px; background: #1a1a3e; color: #c0c0e0; cursor: pointer; font-size: 0.85rem; }
  .btn:hover { background: #2a2a5e; }
  .btn.primary { background: #2a4a3e; border-color: #3e6a54; color: #c0f0d0; }
  .btn.primary:hover:not(:disabled) { background: #3a5a4e; }
  .btn:disabled { opacity: 0.5; cursor: default; }
</style>
