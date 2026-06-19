<script lang="ts">
  import { doExportBin, doExportAllBins } from "../lib/actions";
  import { store } from "../lib/stores.svelte";

  interface Props {
    onClose: () => void;
    onConfirmClose: () => void;
  }

  let { onClose, onConfirmClose }: Props = $props();
  let saving = $state(false);

  let dirtyCount = $derived(store.tabs.filter((t) => t.isDirty).length);
  let hasMultipleDirty = $derived(dirtyCount > 1);

  async function handleExport() {
    saving = true;
    const exported = await doExportBin();
    saving = false;
    if (exported) onConfirmClose();
  }

  async function handleExportAll() {
    saving = true;
    const exportedCount = await doExportAllBins();
    saving = false;
    if (exportedCount === dirtyCount) {
      onConfirmClose();
    } else {
      store.statusMessage = `Exported ${exportedCount} of ${dirtyCount} files`;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
  }

  let backdropMouseDown = false;
  function handleBackdropMouseDown(e: MouseEvent) {
    backdropMouseDown = (e.target as HTMLElement).classList.contains("dialog-backdrop");
  }
  function handleBackdropMouseUp(e: MouseEvent) {
    if (backdropMouseDown && (e.target as HTMLElement).classList.contains("dialog-backdrop")) {
      onClose();
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
      <h2>Are you sure?</h2>
      <button class="close-btn" onclick={onClose}>&times;</button>
    </div>

    <div class="dialog-body">
      {#if dirtyCount > 0}
        <p>
          You have unsaved changes in {dirtyCount} {dirtyCount === 1 ? "tab" : "tabs"}. Would you like to export before closing?
        </p>
        <p class="auto-save-note">Projects are auto-saved.</p>
      {:else}
        <p>Are you sure you want to exit?</p>
      {/if}
    </div>

    {#if dirtyCount > 0}
      <div class="dialog-actions">
        <div class="action-group">
          {#if hasMultipleDirty}
            <button class="export-btn" onclick={handleExportAll} disabled={saving}>
              Export All BINs
            </button>
          {/if}
          <button class="export-btn" onclick={handleExport} disabled={saving}>
            Export BIN
          </button>
        </div>
      </div>
    {/if}

    <div class="dialog-footer">
      <button class="cancel-btn" onclick={onClose} disabled={saving}>Cancel</button>
      <button class="close-confirm-btn" onclick={onConfirmClose} disabled={saving}>
        {dirtyCount > 0 ? "Close Without Exporting" : "Exit"}
      </button>
    </div>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    /* App-quit confirmation must outrank every other modal (status/overflow
       dialogs sit at 1000) so it's never trapped behind an open popup. */
    z-index: 3000;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .dialog {
    width: 480px;
    background: #1a1a2e;
    border: 1px solid #3a3a5a;
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    overflow: hidden;
    display: flex;
    flex-direction: column;
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

  .dialog-body p {
    margin: 0;
    font-size: 0.9rem;
    color: #c0c0e0;
  }

  .auto-save-note {
    margin-top: 6px !important;
    font-size: 0.8rem !important;
    color: #6fd886 !important;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    padding: 12px 16px 0;
  }

  .action-group {
    display: flex;
    gap: 8px;
  }

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 8px;
    padding: 12px 16px;
    border-top: 1px solid #2a2a4a;
    margin-top: 12px;
  }

  .cancel-btn {
    padding: 6px 14px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #888;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .cancel-btn:hover:not(:disabled) {
    color: #c0c0e0;
    border-color: #5a5a8a;
  }

  .export-btn {
    padding: 6px 14px;
    border: 1px solid #2a4a5a;
    border-radius: 4px;
    background: #1a2a3e;
    color: #6cb4d8;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .export-btn:hover:not(:disabled) {
    background: #2a3a4e;
  }

  .close-confirm-btn {
    padding: 6px 14px;
    border: 1px solid #5a2a2a;
    border-radius: 4px;
    background: #3a1a1a;
    color: #f06060;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .close-confirm-btn:hover:not(:disabled) {
    background: #4a2a2a;
  }

  .dialog-footer button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
