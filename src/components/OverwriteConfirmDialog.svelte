<script lang="ts">
  interface Props {
    userCount: number;
    claudeCount: number;
    emptyCount: number;
    onConfirm: (mode: "skip" | "overwrite-claude" | "overwrite-all") => void;
    onCancel: () => void;
  }

  let { userCount, claudeCount, emptyCount, onConfirm, onCancel }: Props = $props();

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onCancel();
  }

  let backdropMouseDown = false;
  function handleBackdropMouseDown(e: MouseEvent) {
    backdropMouseDown = (e.target as HTMLElement).classList.contains("dialog-backdrop");
  }
  function handleBackdropMouseUp(e: MouseEvent) {
    if (backdropMouseDown && (e.target as HTMLElement).classList.contains("dialog-backdrop")) {
      onCancel();
    }
    backdropMouseDown = false;
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="dialog-backdrop" onmousedown={handleBackdropMouseDown} onmouseup={handleBackdropMouseUp}>
  <div class="dialog">
    <div class="dialog-header">
      <h2>Existing Translations Found</h2>
      <button class="close-btn" onclick={onCancel}>&times;</button>
    </div>

    <div class="dialog-body">
      <p class="summary">Some entries already have translations:</p>

      <div class="counts">
        {#if userCount > 0}
          <div class="count-row user">
            <span class="count-badge">{userCount}</span>
            <span>translated by you</span>
          </div>
        {/if}
        {#if claudeCount > 0}
          <div class="count-row claude">
            <span class="count-badge">{claudeCount}</span>
            <span>translated by Claude</span>
          </div>
        {/if}
        <div class="count-row empty">
          <span class="count-badge">{emptyCount}</span>
          <span>untranslated</span>
        </div>
      </div>

      <p class="prompt-text">How should Claude handle existing translations?</p>

      <div class="actions">
        <button class="action-btn primary" onclick={() => onConfirm("skip")}>
          <span class="btn-label">Skip existing</span>
          <span class="btn-hint">Only translate {emptyCount} empty entries</span>
        </button>

        {#if claudeCount > 0}
          <button class="action-btn caution" onclick={() => onConfirm("overwrite-claude")}>
            <span class="btn-label">Overwrite Claude only</span>
            <span class="btn-hint">Re-translate {claudeCount} Claude entries + {emptyCount} empty</span>
          </button>
        {/if}

        <button class="action-btn danger" onclick={() => onConfirm("overwrite-all")}>
          <span class="btn-label">Overwrite all</span>
          <span class="btn-hint">Re-translate all {userCount + claudeCount + emptyCount} entries</span>
        </button>

        <button class="action-btn cancel" onclick={onCancel}>
          Cancel
        </button>
      </div>
    </div>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    /* Above the modal layer (status/overflow at 1000); below the app-quit
       confirm (3000) so that always stays topmost. */
    z-index: 2000;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .dialog {
    width: 440px;
    background: #1a1a2e;
    border: 1px solid #3a3a5a;
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    overflow: hidden;
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid #2a2a4a;
  }

  .dialog-header h2 {
    margin: 0;
    font-size: 1rem;
    color: #e0e0e0;
  }

  .close-btn {
    background: none;
    border: none;
    color: #888;
    font-size: 1.4rem;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }

  .close-btn:hover {
    color: #e0e0e0;
  }

  .dialog-body {
    padding: 16px;
  }

  .summary {
    margin: 0 0 12px;
    color: #c0c0e0;
    font-size: 0.9rem;
  }

  .counts {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 16px;
    padding: 10px 12px;
    background: #16213e;
    border-radius: 6px;
  }

  .count-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.85rem;
    color: #999;
  }

  .count-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 0.8rem;
    font-weight: 600;
  }

  .count-row.user .count-badge {
    background: #1a3a2e;
    color: #6fd886;
  }

  .count-row.claude .count-badge {
    background: #1a2a3e;
    color: #8ab4f8;
  }

  .count-row.empty .count-badge {
    background: #2a2a3e;
    color: #888;
  }

  .prompt-text {
    margin: 0 0 12px;
    color: #999;
    font-size: 0.85rem;
  }

  .actions {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .action-btn {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: 8px 12px;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    background: #16213e;
    cursor: pointer;
    text-align: left;
    transition: all 0.15s;
  }

  .action-btn:hover {
    border-color: #5a5a8a;
  }

  .btn-label {
    font-size: 0.85rem;
    font-weight: 600;
  }

  .btn-hint {
    font-size: 0.75rem;
    color: #666;
    margin-top: 2px;
  }

  .action-btn.primary {
    border-color: #2a5a3e;
  }

  .action-btn.primary .btn-label {
    color: #6fd886;
  }

  .action-btn.primary:hover {
    background: #1a3a2e;
    border-color: #4a7a5e;
  }

  .action-btn.caution .btn-label {
    color: #e0d060;
  }

  .action-btn.caution {
    border-color: #4a4a2a;
  }

  .action-btn.caution:hover {
    background: #2a2a1a;
    border-color: #6a6a3a;
  }

  .action-btn.danger .btn-label {
    color: #e06060;
  }

  .action-btn.danger {
    border-color: #4a2a2a;
  }

  .action-btn.danger:hover {
    background: #2a1a1a;
    border-color: #6a3a3a;
  }

  .action-btn.cancel {
    align-items: center;
    color: #888;
    font-size: 0.85rem;
    border-color: #2a2a4a;
  }

  .action-btn.cancel:hover {
    color: #c0c0e0;
  }
</style>
