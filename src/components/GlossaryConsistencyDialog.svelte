<script lang="ts">
  /**
   * Glossary consistency review: flags translated rows where a glossary term's
   * Japanese appears in the original but its English rendering is missing from
   * the translation (possible drift / forgotten term). Review flags, not errors
   * — click a row to jump to it and confirm or fix.
   */
  import { store } from "../lib/stores.svelte";
  import { findGlossaryInconsistencies, type GlossaryFlag } from "../lib/glossaryCheck";

  interface Props { onClose: () => void; }
  let { onClose }: Props = $props();

  let flags = $derived.by<GlossaryFlag[]>(() => {
    const tab = store.activeTab;
    if (!tab) return [];
    return findGlossaryInconsistencies(tab.entries, store.mergedGlossary);
  });

  /** Distinct terms involved, for the summary. */
  let termCount = $derived(new Set(flags.map((f) => f.term)).size);

  function truncate(s: string, n: number): string {
    return s.length <= n ? s : s.slice(0, n) + "…";
  }

  function jumpTo(flag: GlossaryFlag) {
    const tab = store.activeTab;
    if (!tab) return;
    // rowIndex was computed against the current entries array; re-find by id to
    // be safe against any intervening edits.
    const idx = tab.entries.findIndex((e) => e.instruction_index === flag.rowId);
    const target = idx >= 0 ? idx : flag.rowIndex;
    tab.selectedRowIndex = target;
    store.pendingScrollRequest = { tabId: tab.id, rowIndex: target, nonce: Date.now() };
    onClose();
  }
</script>

<div class="dialog-backdrop" onclick={onClose} onkeydown={(e) => e.key === "Escape" && onClose()} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <header>
      <h2>Glossary Consistency</h2>
      <button class="close-btn" onclick={onClose}>×</button>
    </header>

    {#if store.mergedGlossary.filter((g) => g.english.trim()).length === 0}
      <div class="empty">No glossary entries with English renderings yet — add some to check consistency.</div>
    {:else if flags.length === 0}
      <div class="empty">✓ All glossary terms are rendered consistently in the translated rows.</div>
    {:else}
      <div class="summary">
        <strong>{flags.length}</strong> row{flags.length === 1 ? "" : "s"} where a glossary term's English may be missing
        ({termCount} distinct term{termCount === 1 ? "" : "s"}). Review flags — a term is sometimes legitimately rephrased.
      </div>
      <div class="table-scroll">
        <table>
          <thead>
            <tr>
              <th class="col-term">Term → glossary</th>
              <th class="col-text">Original / your translation</th>
            </tr>
          </thead>
          <tbody>
            {#each flags as f (f.rowId + "|" + f.term)}
              <tr>
                <td class="col-term">
                  <span class="jp">{f.term}</span>
                  <span class="arrow">→</span>
                  <span class="en">{f.glossaryEn}</span>
                </td>
                <td class="col-text">
                  <button class="jump" onclick={() => jumpTo(f)} title="Go to row #{f.rowId} in the editor">
                    <span class="orig">{truncate(f.original, 80)}</span>
                    <span class="trans">{truncate(f.translated, 100)}</span>
                  </button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}

    <footer>
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
    width: 880px; max-width: 90vw; max-height: 85vh;
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
  .table-scroll { flex: 1; overflow-y: auto; min-height: 0; }
  table { width: 100%; border-collapse: collapse; font-size: 0.82rem; }
  th { position: sticky; top: 0; background: #16213e; color: #8ab4f8; padding: 8px 10px; text-align: left; font-weight: 600; border-bottom: 1px solid #2a2a4a; }
  td { padding: 6px 10px; color: #c0c0e0; border-bottom: 1px solid #20204a; vertical-align: top; }
  .col-term { width: 240px; }
  .col-term .jp { color: #e0c0a0; }
  .col-term .arrow { color: #6a6a8a; margin: 0 6px; }
  .col-term .en { color: #8ab4f8; font-weight: 600; }
  .jump { display: block; width: 100%; text-align: left; background: none; border: none; padding: 0; margin: 0; cursor: pointer; font: inherit; }
  .jump .orig { display: block; color: #9a9ab8; font-size: 0.78rem; }
  .jump .trans { display: block; color: #d0d0e0; line-height: 1.4; }
  .jump:hover .trans { color: #8ab4f8; text-decoration: underline; }
  footer { padding: 12px 20px; border-top: 1px solid #2a2a4a; display: flex; justify-content: flex-end; }
  .btn { padding: 6px 14px; border: 1px solid #3a3a5a; border-radius: 4px; background: #1a1a3e; color: #c0c0e0; cursor: pointer; font-size: 0.85rem; }
  .btn:hover { background: #2a2a5e; }
</style>
