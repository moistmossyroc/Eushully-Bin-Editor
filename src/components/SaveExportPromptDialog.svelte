<script lang="ts">
  type ActionMode = "save" | "export";

  interface Props {
    mode: ActionMode;
    count: number;
    onCurrentOnly: () => void;
    onAll: () => void;
    onClose: () => void;
  }

  let { mode, count, onCurrentOnly, onAll, onClose }: Props = $props();

  let label = $derived(mode === "save" ? "Save Project" : "Export BIN");

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
      <h2>{label}</h2>
      <button class="close-btn" onclick={onClose}>&times;</button>
    </div>

    <div class="dialog-body">
      <p>You have {count} tabs open. Would you like to {mode === "save" ? "save" : "export"} all of them or just the current tab?</p>
    </div>

    <div class="dialog-footer">
      <button class="cancel-btn" onclick={onClose}>Cancel</button>
      <button class="current-btn" onclick={onCurrentOnly}>
        Current Tab Only
      </button>
      <button class="all-btn" onclick={onAll}>
        All Tabs ({count})
      </button>
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
    width: 460px;
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

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 16px;
    border-top: 1px solid #2a2a4a;
  }

  .cancel-btn {
    padding: 6px 14px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #888;
    font-size: 0.85rem;
    cursor: pointer;
    margin-right: auto;
  }

  .cancel-btn:hover {
    color: #c0c0e0;
    border-color: #5a5a8a;
  }

  .current-btn {
    padding: 6px 14px;
    border: 1px solid #2a4a5a;
    border-radius: 4px;
    background: #1a2a3e;
    color: #6cb4d8;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .current-btn:hover {
    background: #2a3a4e;
  }

  .all-btn {
    padding: 6px 14px;
    border: 1px solid #2a5a3e;
    border-radius: 4px;
    background: #1a3a2e;
    color: #6fd886;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .all-btn:hover {
    background: #2a4a3e;
  }
</style>
