<script lang="ts">
  /**
   * Reviewable glossary auto-apply. Proposes SAFE, deterministic substitutions
   * (fill an untranslated exact-term row; replace a Japanese term that leaked
   * verbatim into the English) and lets the user review/deselect each one before
   * applying. Nothing is mutated until "Apply" — and the apply is captured for
   * one-click bulk undo.
   */
  import { store } from "../lib/stores.svelte";
  import { findGlossaryAutoApply, type GlossaryAutoChange } from "../lib/glossaryAutoApply";
  import { toastInfo } from "../lib/toast";

  interface Props { onClose: () => void; }
  let { onClose }: Props = $props();

  let changes = $derived.by<GlossaryAutoChange[]>(() => {
    const tab = store.activeTab;
    if (!tab) return [];
    return findGlossaryAutoApply(tab.entries, store.mergedGlossary);
  });

  /** rowIds the user has unchecked (default = everything selected). */
  let excluded = $state(new Set<number>());

  let selectedCount = $derived(changes.filter((c) => !excluded.has(c.rowId)).length);

  function toggle(rowId: number) {
    const next = new Set(excluded);
    if (next.has(rowId)) next.delete(rowId);
    else next.add(rowId);
    excluded = next;
  }
  function selectAll() { excluded = new Set(); }
  function selectNone() { excluded = new Set(changes.map((c) => c.rowId)); }

  function truncate(s: string, n: number): string {
    return s.length <= n ? s : s.slice(0, n) + "…";
  }

  function jumpTo(c: GlossaryAutoChange) {
    const tab = store.activeTab;
    if (!tab) return;
    const idx = tab.entries.findIndex((e) => e.instruction_index === c.rowId);
    const target = idx >= 0 ? idx : c.rowIndex;
    tab.selectedRowIndex = target;
    store.pendingScrollRequest = { tabId: tab.id, rowIndex: target, nonce: Date.now() };
    onClose();
  }

  function applySelected() {
    const tab = store.activeTab;
    if (!tab) return;
    const toApply = changes.filter((c) => !excluded.has(c.rowId));
    if (toApply.length === 0) return;

    // Snapshot first so the whole batch is undoable in one click.
    store.captureBulkUndo("Glossary auto-apply", [tab]);

    // Index-based map: `changes` was derived from the current entries and we
    // apply immediately (modal), so positions are stable. Fresh object spreads
    // for changed rows keep keyed {#each} children in sync (reactivity gotcha).
    const byIndex = new Map(toApply.map((c) => [c.rowIndex, c.after]));
    tab.entries = tab.entries.map((e, i) =>
      byIndex.has(i) ? { ...e, translated_text: byIndex.get(i)! } : e,
    );
    tab.isDirty = true;

    toastInfo(`Applied ${toApply.length} glossary change${toApply.length === 1 ? "" : "s"} — undo available`);
    onClose();
  }
</script>

<div class="dialog-backdrop" onclick={onClose} onkeydown={(e) => e.key === "Escape" && onClose()} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <header>
      <h2>Glossary Auto-Apply</h2>
      <button class="close-btn" onclick={onClose}>×</button>
    </header>

    {#if store.mergedGlossary.filter((g) => g.english.trim()).length === 0}
      <div class="empty">No glossary entries with English renderings yet — add some first.</div>
    {:else if changes.length === 0}
      <div class="empty">✓ Nothing to auto-apply — no exact-term fills or leaked Japanese terms found.</div>
    {:else}
      <div class="summary">
        <strong>{changes.length}</strong> safe change{changes.length === 1 ? "" : "s"} proposed
        (<span class="kind fill">fill</span> = exact-term row, <span class="kind replace">replace</span> = Japanese leaked into the English).
        Deselect anything you don't want, then apply.
        <span class="select-actions">
          <button class="link" onclick={selectAll}>all</button> /
          <button class="link" onclick={selectNone}>none</button>
        </span>
      </div>
      <div class="table-scroll">
        <table>
          <thead>
            <tr>
              <th class="col-check"></th>
              <th class="col-term">Term → glossary</th>
              <th class="col-text">Change</th>
            </tr>
          </thead>
          <tbody>
            {#each changes as c (c.rowId)}
              <tr>
                <td class="col-check">
                  <input type="checkbox" checked={!excluded.has(c.rowId)} onchange={() => toggle(c.rowId)} />
                </td>
                <td class="col-term">
                  <span class="kind {c.kind}">{c.kind}</span>
                  <span class="jp">{truncate(c.term, 40)}</span>
                  <span class="arrow">→</span>
                  <span class="en">{truncate(c.glossaryEn, 40)}</span>
                </td>
                <td class="col-text">
                  <button class="jump" onclick={() => jumpTo(c)} title="Go to row #{c.rowId} in the editor">
                    <span class="before">{c.before.trim() === "" ? "(empty)" : truncate(c.before, 90)}</span>
                    <span class="after"><span class="aarrow">→</span> {truncate(c.after, 90)}</span>
                  </button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}

    <footer>
      <button class="btn" onclick={onClose}>Cancel</button>
      <button class="btn primary" disabled={selectedCount === 0} onclick={applySelected}>
        Apply {selectedCount} selected
      </button>
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
  .summary strong { color: #6ad08a; }
  .select-actions { margin-left: 8px; }
  .link { background: none; border: none; color: #8ab4f8; cursor: pointer; font-size: 0.82rem; padding: 0 2px; text-decoration: underline; }
  .table-scroll { flex: 1; overflow-y: auto; min-height: 0; }
  table { width: 100%; border-collapse: collapse; font-size: 0.82rem; }
  th { position: sticky; top: 0; background: #16213e; color: #8ab4f8; padding: 8px 10px; text-align: left; font-weight: 600; border-bottom: 1px solid #2a2a4a; }
  td { padding: 6px 10px; color: #c0c0e0; border-bottom: 1px solid #20204a; vertical-align: top; }
  .col-check { width: 28px; text-align: center; }
  .col-term { width: 300px; }
  .col-term .jp { color: #e0c0a0; }
  .col-term .arrow { color: #6a6a8a; margin: 0 6px; }
  .col-term .en { color: #8ab4f8; font-weight: 600; }
  .kind { display: inline-block; font-size: 0.68rem; padding: 1px 6px; border-radius: 3px; margin-right: 6px; text-transform: uppercase; letter-spacing: 0.03em; }
  .kind.fill { background: #1e3a2e; color: #6ad08a; border: 1px solid #2e5a44; }
  .kind.replace { background: #3a2e1e; color: #f0a070; border: 1px solid #5a442e; }
  .jump { display: block; width: 100%; text-align: left; background: none; border: none; padding: 0; margin: 0; cursor: pointer; font: inherit; }
  .jump .before { display: block; color: #9a7a7a; font-size: 0.78rem; text-decoration: line-through; }
  .jump .after { display: block; color: #d0e8d0; line-height: 1.4; }
  .jump .after .aarrow { color: #6ad08a; }
  .jump:hover .after { color: #6ad08a; }
  footer { padding: 12px 20px; border-top: 1px solid #2a2a4a; display: flex; justify-content: flex-end; gap: 8px; }
  .btn { padding: 6px 14px; border: 1px solid #3a3a5a; border-radius: 4px; background: #1a1a3e; color: #c0c0e0; cursor: pointer; font-size: 0.85rem; }
  .btn:hover { background: #2a2a5e; }
  .btn.primary { background: #2a4a3e; border-color: #3e6a54; color: #c0f0d0; }
  .btn.primary:hover:not(:disabled) { background: #3a5a4e; }
  .btn:disabled { opacity: 0.5; cursor: default; }
</style>
