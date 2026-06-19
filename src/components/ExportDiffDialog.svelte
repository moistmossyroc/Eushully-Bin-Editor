<script lang="ts">
  /**
   * Pre-export summary: shows what will change in the exported BIN relative
   * to the source. Catches "I meant to export X but accidentally hit Export
   * on tab Y" and "did I really mean to mark 50 entries as omit?" — cheap
   * visibility into the destructive action.
   *
   * Stats are computed by the parent from `tab.entries` + `tab.insertedInstructions`
   * and passed in via props; this component is purely presentational.
   */
  import type { ExportDiff } from "../lib/types";

  interface Props {
    binFilename: string;
    diff: ExportDiff;
    onCancel: () => void;
    onConfirm: () => void;
    /** Jump to a row (by instruction index) in the editor — aborts the export
     *  and closes this dialog so the user can hand-edit before re-exporting. */
    onJump: (instructionIndex: number) => void;
  }
  let { binFilename, diff, onCancel, onConfirm, onJump }: Props = $props();

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onCancel();
    else if (e.key === "Enter") onConfirm();
  }

  function truncate(s: string, n: number): string {
    return s.length > n ? s.slice(0, n) + "…" : s;
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="backdrop" onclick={onCancel}></div>

<div class="dialog" role="dialog" aria-label="Export summary">
  <header class="dialog-header">
    <h3>Export {binFilename}?</h3>
    <button class="close-btn" onclick={onCancel} title="Cancel (Esc)">×</button>
  </header>

  <div class="summary">
    <div class="stat-grid">
      <div class="stat">
        <div class="stat-value">{diff.translatedCount.toLocaleString()}</div>
        <div class="stat-label">translated</div>
      </div>
      <div class="stat">
        <div class="stat-value untranslated">{diff.untranslatedCount.toLocaleString()}</div>
        <div class="stat-label">still empty</div>
      </div>
      <div class="stat">
        <div class="stat-value inserted">{diff.insertedCount.toLocaleString()}</div>
        <div class="stat-label">inserted</div>
      </div>
      <div class="stat">
        <div class="stat-value omitted">{diff.omittedCount.toLocaleString()}</div>
        <div class="stat-label">omitted</div>
      </div>
      <div class="stat">
        <div class="stat-value deleted">{diff.deletedCount.toLocaleString()}</div>
        <div class="stat-label">deleted</div>
      </div>
    </div>

    {#if diff.untranslatedCount > 0}
      <p class="warn">
        ⚠ {diff.untranslatedCount} editable entries have no translation (excludes rows
        marked “do not export”). The original Japanese will be written for those rows.
      </p>
    {/if}

    {#if diff.changes.length > 0}
      <h4>All changes ({diff.changes.length.toLocaleString()})</h4>
    {/if}
  </div>

  {#if diff.changes.length > 0}
    <div class="changes-scroll">
      <table class="changes">
        <thead>
          <tr><th class="col-id">#</th><th class="col-label">Label</th><th>Original → Translation</th></tr>
        </thead>
        <tbody>
          {#each diff.changes as c (c.id)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <tr class="change-row" onclick={() => onJump(c.id)} title="Go to row #{c.id} in the editor">
              <td class="change-id">{c.id}</td>
              <td class="change-label">{c.label}</td>
              <td class="change-text">
                <span class="orig">{truncate(c.original, 80)}</span>
                <span class="arrow">→</span>
                <span class="trans">{truncate(c.translated, 80)}</span>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <div class="no-changes">No translated changes — export will write source text for all editable rows.</div>
  {/if}

  <footer>
    <button class="btn cancel" onclick={onCancel}>Cancel</button>
    <button class="btn primary" onclick={onConfirm}>Export…</button>
  </footer>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 1000;
  }
  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 1001;
    width: 640px;
    max-width: 92vw;
    max-height: 80vh;
    background: #15152a;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
  }
  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 16px;
    border-bottom: 1px solid #2a2a4a;
  }
  .dialog-header h3 {
    margin: 0;
    font-size: 1rem;
    color: #c0c0e0;
  }
  .close-btn {
    background: transparent;
    border: none;
    color: #8a8aaa;
    font-size: 1.3rem;
    cursor: pointer;
  }
  .close-btn:hover { color: #fff; }

  .summary {
    padding: 16px 16px 0 16px;
    flex-shrink: 0;
  }
  .stat-grid {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 8px;
    margin-bottom: 16px;
  }
  .stat {
    background: #1a1a3e;
    border: 1px solid #2a2a4a;
    border-radius: 4px;
    padding: 10px 8px;
    text-align: center;
  }
  .stat-value {
    font-size: 1.4rem;
    font-weight: 600;
    color: #8ab4f8;
    line-height: 1;
    margin-bottom: 4px;
  }
  .stat-value.untranslated { color: #e8a838; }
  .stat-value.inserted { color: #6fd886; }
  .stat-value.omitted { color: #b070d0; }
  .stat-value.deleted { color: #e06060; }
  .stat-label {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #888;
  }

  .summary h4 {
    margin: 16px 0 0 0;
    font-size: 0.78rem;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #8aa0ff;
  }

  .changes-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 6px 16px 16px 16px;
  }
  .changes {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8rem;
  }
  .changes th {
    position: sticky;
    top: 0;
    background: #15152a;
    text-align: left;
    color: #888;
    font-size: 0.7rem;
    padding: 4px 8px;
    border-bottom: 1px solid #2a2a4a;
  }
  .changes td {
    padding: 5px 8px;
    border-bottom: 1px solid #1a1a30;
    vertical-align: top;
  }
  .change-row { cursor: pointer; }
  .change-row:hover td { background: #20204a; }
  .col-id { width: 56px; }
  .col-label { width: 120px; }
  .change-id {
    color: #6a6a8a;
    font-family: monospace;
    font-size: 0.72rem;
  }
  .change-label {
    color: #8aa0ff;
    font-family: monospace;
    font-size: 0.75rem;
  }
  .change-text { color: #c0c0e0; }
  .change-text .orig { color: #d8a86f; }
  .change-text .arrow { color: #555577; margin: 0 6px; }
  .change-text .trans { color: #6fd8a4; }

  .no-changes {
    padding: 12px 16px 16px 16px;
    color: #888;
    font-size: 0.8rem;
  }

  .warn {
    margin: 12px 0 0 0;
    padding: 8px 12px;
    background: #3a2a14;
    border: 1px solid #6a4a24;
    border-radius: 3px;
    color: #f4c87a;
    font-size: 0.8rem;
  }

  footer {
    padding: 10px 16px;
    border-top: 1px solid #2a2a4a;
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
  .btn {
    padding: 6px 16px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .btn.cancel:hover { background: #2a2a5e; }
  .btn.primary {
    background: #2a3a6e;
    border-color: #5a7aff;
    color: #fff;
  }
  .btn.primary:hover { background: #3a4a7e; }
</style>
