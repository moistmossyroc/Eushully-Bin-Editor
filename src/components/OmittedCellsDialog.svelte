<script lang="ts">
  import { store } from "../lib/stores.svelte";
  import { findOverflows } from "../lib/overflow";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  /** Every row currently excluded from export. Broader than the bulk-omit
   *  eligibility set — lists ALL omit_export rows (incl. set-string / furigana
   *  toggles) so nothing the user marked is hidden. Re-derives live. */
  let omittedRows = $derived.by(() => {
    const tab = store.activeTab;
    if (!tab) return [];
    const out: {
      rowIndex: number;
      id: number;
      label: string;
      original: string;
      translated: string;
      speaker?: string;
    }[] = [];
    for (let i = 0; i < tab.entries.length; i++) {
      const e = tab.entries[i];
      if (e.deleted || e.is_context_row) continue;
      if (e.omit_export !== true) continue;
      const sref = tab.rowSpeakers?.[String(e.instruction_index)];
      out.push({
        rowIndex: i,
        id: e.instruction_index,
        label: e.label,
        original: e.original_text,
        translated: e.translated_text,
        speaker: sref?.literal_name ?? undefined,
      });
    }
    return out;
  });

  let lastScan = $state<string | null>(null);

  /** Force every derived stat (omit toggle, overflow count, progress dot) to
   *  recompute by rebuilding the entries array. Fixes the case where a cell
   *  changed during import via direct mutation that Svelte didn't track, so it
   *  "didn't register" in the omit/overflow tallies. Pure refresh — no data
   *  change, so it doesn't mark the tab dirty. */
  function rescan() {
    const tab = store.activeTab;
    if (!tab) return;
    tab.entries = tab.entries.map((e) => ({ ...e }));
    const omitted = tab.entries.filter(
      (e) => !e.deleted && !e.is_context_row && e.omit_export === true,
    ).length;
    const over =
      store.translatedLineCharCap > 0
        ? findOverflows(tab.entries, store.translatedLineCharCap).length
        : 0;
    lastScan = `${omitted} omitted cell${omitted === 1 ? "" : "s"}, ${over} overflow${over === 1 ? "" : "s"}`;
    store.statusMessage = `Rescanned: ${lastScan}.`;
  }

  /** Re-include a single row in export (clears omit_export). Fresh array so the
   *  keyed editor rows re-render. */
  function includeRow(id: number) {
    const tab = store.activeTab;
    if (!tab) return;
    tab.entries = tab.entries.map((e) =>
      e.instruction_index === id ? { ...e, omit_export: false } : e,
    );
    tab.isDirty = true;
  }

  /** Re-include every listed row at once. */
  function includeAll() {
    const tab = store.activeTab;
    if (!tab) return;
    const ids = new Set(omittedRows.map((r) => r.id));
    if (ids.size === 0) return;
    tab.entries = tab.entries.map((e) =>
      ids.has(e.instruction_index) ? { ...e, omit_export: false } : e,
    );
    tab.isDirty = true;
  }

  /** Jump to the row in the editor table and close. */
  function jump(rowIndex: number) {
    const tab = store.activeTab;
    if (!tab) return;
    tab.selectedRowIndex = rowIndex;
    store.pendingScrollRequest = { tabId: tab.id, rowIndex, nonce: Date.now() };
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
      <h2>Omitted Cells</h2>
      <button class="close-btn" onclick={onClose}>×</button>
    </header>

    <div class="summary">
      <strong>{omittedRows.length}</strong> cell{omittedRows.length === 1 ? "" : "s"}
      excluded from export (BIN gets empty strings — nothing renders in-game).
      {#if lastScan}<span class="scan-note">Last rescan: {lastScan}</span>{/if}
    </div>

    <div class="bulk-actions">
      <button class="btn rescan" onclick={rescan} title="Recompute omit/overflow tallies — fixes a cell that didn't register after import">
        Rescan omitted &amp; overflow
      </button>
      {#if omittedRows.length > 0}
        <button class="btn include" onclick={includeAll} title="Re-include every listed cell (their original text will export again)">
          Re-include all ({omittedRows.length})
        </button>
      {/if}
    </div>

    {#if omittedRows.length === 0}
      <div class="empty">No cells are currently omitted from export.</div>
    {:else}
      <div class="table-scroll">
        <table>
          <thead>
            <tr>
              <th class="col-label">Function</th>
              <th class="col-text">Original</th>
              <th class="col-text">Translated</th>
              <th class="col-actions">Action</th>
            </tr>
          </thead>
          <tbody>
            {#each omittedRows as r (r.id)}
              <tr>
                <td class="col-label">
                  <span class="lbl">{r.label}</span>
                  {#if r.speaker}<span class="speaker">{r.speaker}</span>{/if}
                </td>
                <td class="col-text">
                  <button class="jump-text" title={`Go to row #${r.id} in the editor`} onclick={() => jump(r.rowIndex)}>
                    {truncate(r.original, 80) || "(empty)"}
                  </button>
                </td>
                <td class="col-text dim">{truncate(r.translated, 80) || "—"}</td>
                <td class="col-actions">
                  <button class="row-btn" onclick={() => includeRow(r.id)} title="Re-include this cell in export">
                    Include
                  </button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}

    <footer>
      <button class="btn cancel" onclick={onClose}>Close</button>
    </footer>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed; inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex; align-items: center; justify-content: center;
    z-index: 1000;
  }
  .dialog {
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    width: 820px; max-width: 90vw; max-height: 85vh;
    display: flex; flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  header {
    padding: 14px 20px;
    border-bottom: 1px solid #2a2a4a;
    display: flex; justify-content: space-between; align-items: center;
  }
  h2 { margin: 0; color: #8ab4f8; font-size: 1.05rem; }
  .close-btn {
    background: none; border: none; color: #a0a0c0;
    font-size: 1.4rem; cursor: pointer; padding: 0 6px; line-height: 1;
  }
  .close-btn:hover { color: #fff; }
  .summary {
    padding: 12px 20px;
    font-size: 0.85rem;
    color: #c0c0e0;
    border-bottom: 1px solid #2a2a4a;
  }
  .summary strong { color: #f0a070; }
  .scan-note { margin-left: 10px; color: #6fd886; font-size: 0.78rem; }
  .bulk-actions {
    padding: 10px 20px;
    display: flex; gap: 8px;
    border-bottom: 1px solid #2a2a4a;
  }
  .empty {
    padding: 32px 24px;
    text-align: center;
    color: #a0a0c0;
    font-size: 0.9rem;
  }
  .table-scroll { flex: 1; overflow-y: auto; min-height: 0; }
  table { width: 100%; border-collapse: collapse; font-size: 0.82rem; }
  th {
    position: sticky; top: 0; z-index: 2;
    background: #16213e;
    color: #8ab4f8;
    padding: 8px 10px;
    text-align: left; font-weight: 600;
    border-bottom: 1px solid #2a2a4a;
  }
  td {
    padding: 6px 10px;
    color: #c0c0e0;
    border-bottom: 1px solid #20204a;
    vertical-align: top;
  }
  .col-label { width: 150px; }
  .lbl { color: #8aa0ff; font-family: monospace; font-size: 0.72rem; }
  .speaker {
    display: inline-block; margin-left: 4px;
    color: #6fd886; font-size: 0.68rem;
    background: #102a20; padding: 1px 5px; border-radius: 3px;
  }
  .col-text { color: #d8a86f; line-height: 1.4; }
  .col-text.dim { color: #8a8aaa; }
  .col-actions { width: 90px; white-space: nowrap; }
  .jump-text {
    display: block; width: 100%; text-align: left;
    background: none; border: none; padding: 0; margin: 0;
    color: inherit; font: inherit; line-height: 1.4; cursor: pointer;
  }
  .jump-text:hover { color: #8ab4f8; text-decoration: underline; }
  footer {
    padding: 12px 20px;
    border-top: 1px solid #2a2a4a;
    display: flex; justify-content: flex-end;
  }
  .btn {
    padding: 6px 14px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .btn:hover:not(:disabled) { background: #2a2a5e; }
  .btn.rescan { border-color: #6c9bd2; color: #8ab4f8; }
  .btn.include { border-color: #d2a06c; color: #f0c080; }
  .row-btn {
    padding: 3px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #20204a;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.75rem;
  }
  .row-btn:hover { background: #2a2a5e; }
</style>
