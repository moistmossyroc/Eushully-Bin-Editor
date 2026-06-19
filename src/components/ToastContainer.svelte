<script lang="ts">
  /**
   * Bottom-right toast stack. Reads from `store.toasts` (newest first).
   * Click any toast to dismiss. Color-coded by kind. Sits above everything
   * via high z-index so a backdrop-clad dialog can't hide a critical error.
   */
  import { store } from "../lib/stores.svelte";
  import { dismissToast } from "../lib/toast";
</script>

{#if store.toasts.length > 0}
  <div class="toast-stack" role="status" aria-live="polite">
    {#each store.toasts as t (t.id)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="toast toast-{t.kind}"
        onclick={() => dismissToast(t.id)}
        title="Click to dismiss"
      >
        <span class="toast-icon">
          {t.kind === "error" ? "✕" : t.kind === "warn" ? "⚠" : "ℹ"}
        </span>
        <span class="toast-msg">{t.message}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-stack {
    position: fixed;
    right: 16px;
    bottom: 32px;
    z-index: 2000;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 420px;
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 9px 13px;
    border-radius: 4px;
    font-size: 0.82rem;
    line-height: 1.4;
    cursor: pointer;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
    border: 1px solid;
    animation: slideIn 0.18s ease-out;
  }
  @keyframes slideIn {
    from { transform: translateX(20px); opacity: 0; }
    to   { transform: translateX(0); opacity: 1; }
  }
  .toast-icon {
    flex-shrink: 0;
    font-weight: 700;
    line-height: 1.4;
  }
  .toast-msg {
    flex: 1;
    word-break: break-word;
  }
  .toast-error {
    background: #3a1a1a;
    border-color: #a04040;
    color: #f0c0c0;
  }
  .toast-warn {
    background: #3a2a14;
    border-color: #a07040;
    color: #f4d8a0;
  }
  .toast-info {
    background: #1a2a4a;
    border-color: #4070a0;
    color: #c0d8f0;
  }
  .toast:hover { filter: brightness(1.15); }
</style>
