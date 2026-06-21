<script lang="ts">
  /**
   * Themed confirmation dialog. Driven by `store.confirmRequest` (set via
   * confirmDialog() in lib/confirm.ts). Resolves the pending promise true on
   * confirm, false on cancel/escape/backdrop.
   */
  import { store } from "../lib/stores.svelte";

  function resolve(value: boolean) {
    const req = store.confirmRequest;
    store.confirmRequest = null;
    req?.resolve(value);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (!store.confirmRequest) return;
    if (e.key === "Escape") { e.preventDefault(); resolve(false); }
    else if (e.key === "Enter") { e.preventDefault(); resolve(true); }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if store.confirmRequest}
  {@const req = store.confirmRequest}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={() => resolve(false)}></div>

  <div class="dialog" role="dialog" aria-label={req.title}>
    <header><h3>{req.title}</h3></header>
    <div class="body">{req.message}</div>
    <footer>
      <button class="btn cancel" onclick={() => resolve(false)}>{req.cancelLabel}</button>
      <button class="btn confirm" class:danger={req.danger} onclick={() => resolve(true)}>{req.confirmLabel}</button>
    </footer>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    /* Above the modal layer (1000) and 2000-tier dialogs, below app-quit (3000). */
    z-index: 2600;
  }
  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 2601;
    width: 420px;
    max-width: 92vw;
    background: #15152a;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
  }
  header {
    padding: 12px 16px;
    border-bottom: 1px solid #2a2a4a;
  }
  header h3 {
    margin: 0;
    font-size: 1rem;
    color: #8ab4f8;
  }
  .body {
    padding: 16px;
    color: #c0c0e0;
    font-size: 0.88rem;
    line-height: 1.5;
    white-space: pre-line; /* honor \n / \n\n in messages */
  }
  footer {
    padding: 12px 16px;
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
  .btn.confirm {
    background: #2a3a6e;
    border-color: #5a7aff;
    color: #fff;
    font-weight: 600;
  }
  .btn.confirm:hover { background: #3a4a7e; }
  .btn.confirm.danger {
    background: #5a2020;
    border-color: #e06060;
    color: #ffd8d8;
  }
  .btn.confirm.danger:hover { background: #7a2a2a; }
</style>
