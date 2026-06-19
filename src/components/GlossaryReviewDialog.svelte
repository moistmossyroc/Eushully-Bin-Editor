<script lang="ts">
  import { store } from "../lib/stores.svelte";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  function acceptItem(index: number) {
    const item = store.glossaryReviewQueue[index];
    if (!item) return;

    // Find the tab this glossary update belongs to
    const tab = store.tabs.find((t) => t.id === item.tabId);
    if (tab) {
      const existingIdx = tab.glossary.findIndex(
        (g) => g.japanese === item.existing.japanese
      );
      if (existingIdx !== -1) {
        tab.glossary[existingIdx] = { ...item.proposed };
      } else {
        tab.glossary.push({ ...item.proposed });
      }
      tab.isDirty = true;
    }

    store.glossaryReviewQueue = store.glossaryReviewQueue.filter((_, i) => i !== index);
    if (store.glossaryReviewQueue.length === 0) onClose();
  }

  function dismissItem(index: number) {
    store.glossaryReviewQueue = store.glossaryReviewQueue.filter((_, i) => i !== index);
    if (store.glossaryReviewQueue.length === 0) onClose();
  }

  function acceptAll() {
    for (const item of store.glossaryReviewQueue) {
      const tab = store.tabs.find((t) => t.id === item.tabId);
      if (tab) {
        const existingIdx = tab.glossary.findIndex(
          (g) => g.japanese === item.existing.japanese
        );
        if (existingIdx !== -1) {
          tab.glossary[existingIdx] = { ...item.proposed };
        } else {
          tab.glossary.push({ ...item.proposed });
        }
        tab.isDirty = true;
      }
    }
    store.glossaryReviewQueue = [];
    onClose();
  }

  function dismissAll() {
    store.glossaryReviewQueue = [];
    onClose();
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
      <h2>Glossary Review</h2>
      <span class="pending-count">{store.glossaryReviewQueue.length} pending</span>
      <button class="close-btn" onclick={onClose}>&times;</button>
    </div>

    <div class="dialog-body">
      {#each store.glossaryReviewQueue as item, index (index)}
        <div class="review-item">
          <div class="term-header">
            <span class="jp-term">{item.existing.japanese}</span>
            <span class="cat-badge">{item.proposed.category}</span>
          </div>
          <div class="comparison">
            <div class="current">
              <span class="label">Current</span>
              <span class="value">{item.existing.english}</span>
              {#if item.existing.notes}
                <span class="notes">{item.existing.notes}</span>
              {/if}
            </div>
            <span class="arrow">&rarr;</span>
            <div class="proposed">
              <span class="label">Proposed</span>
              <span class="value">{item.proposed.english}</span>
              {#if item.proposed.notes}
                <span class="notes">{item.proposed.notes}</span>
              {/if}
            </div>
          </div>
          <div class="item-actions">
            <button class="accept-btn" onclick={() => acceptItem(index)}>Allow</button>
            <button class="dismiss-btn" onclick={() => dismissItem(index)}>Deny</button>
          </div>
        </div>
      {/each}

      {#if store.glossaryReviewQueue.length === 0}
        <div class="empty-state">No pending glossary reviews</div>
      {/if}
    </div>

    {#if store.glossaryReviewQueue.length > 1}
      <div class="dialog-footer">
        <button class="bulk-btn accept" onclick={acceptAll}>Allow All</button>
        <button class="bulk-btn dismiss" onclick={dismissAll}>Deny All</button>
      </div>
    {/if}
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 150;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .dialog {
    width: 520px;
    max-height: 70vh;
    background: #1a1a2e;
    border: 1px solid #3a3a5a;
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .dialog-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px;
    border-bottom: 1px solid #2a2a4a;
  }

  .dialog-header h2 {
    margin: 0;
    font-size: 1rem;
    color: #e0e0e0;
  }

  .pending-count {
    font-size: 0.75rem;
    color: #f0a040;
    margin-right: auto;
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
    flex: 1;
    overflow-y: auto;
    padding: 8px 16px;
  }

  .review-item {
    padding: 10px;
    border: 1px solid #2a2a4a;
    border-radius: 6px;
    margin-bottom: 8px;
    background: #16213e;
  }

  .term-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }

  .jp-term {
    font-size: 0.9rem;
    color: #c0c0e0;
    font-weight: 600;
  }

  .cat-badge {
    padding: 1px 5px;
    border-radius: 3px;
    background: #2a2a4a;
    color: #888;
    font-size: 0.65rem;
  }

  .comparison {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin-bottom: 8px;
  }

  .current, .proposed {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .comparison .label {
    font-size: 0.65rem;
    color: #666;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .current .value {
    color: #888;
    font-size: 0.85rem;
  }

  .proposed .value {
    color: #8ab4f8;
    font-size: 0.85rem;
  }

  .comparison .arrow {
    color: #444;
    margin-top: 14px;
    flex-shrink: 0;
  }

  .notes {
    font-size: 0.7rem;
    color: #555;
    font-style: italic;
  }

  .item-actions {
    display: flex;
    gap: 6px;
    justify-content: flex-end;
  }

  .accept-btn {
    padding: 3px 12px;
    border: 1px solid #2a5a3e;
    border-radius: 3px;
    background: #1a3a2e;
    color: #6fd886;
    font-size: 0.8rem;
    cursor: pointer;
  }

  .accept-btn:hover {
    background: #2a4a3e;
  }

  .dismiss-btn {
    padding: 3px 12px;
    border: 1px solid #5a2a2a;
    border-radius: 3px;
    background: #3a1a1a;
    color: #e06060;
    font-size: 0.8rem;
    cursor: pointer;
  }

  .dismiss-btn:hover {
    background: #4a2a2a;
  }

  .empty-state {
    padding: 20px;
    text-align: center;
    color: #555;
    font-size: 0.85rem;
  }

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 10px 16px;
    border-top: 1px solid #2a2a4a;
  }

  .bulk-btn {
    padding: 5px 14px;
    border-radius: 4px;
    font-size: 0.8rem;
    cursor: pointer;
  }

  .bulk-btn.accept {
    border: 1px solid #2a5a3e;
    background: #1a3a2e;
    color: #6fd886;
  }

  .bulk-btn.accept:hover {
    background: #2a4a3e;
  }

  .bulk-btn.dismiss {
    border: 1px solid #5a2a2a;
    background: #3a1a1a;
    color: #e06060;
  }

  .bulk-btn.dismiss:hover {
    background: #4a2a2a;
  }
</style>
