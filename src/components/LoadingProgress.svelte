<script lang="ts">
  import { store } from "../lib/stores.svelte";
</script>

{#if store.loadingProgress}
  <div class="loading-overlay">
    <div class="loading-dialog">
      <div class="loading-header">Loading Files</div>
      <div class="loading-body">
        <div class="progress-text">
          {store.loadingProgress.current} / {store.loadingProgress.total}
        </div>
        <div class="progress-bar-track">
          <div
            class="progress-bar-fill"
            style="width: {(store.loadingProgress.current / store.loadingProgress.total) * 100}%"
          ></div>
        </div>
        <div class="current-file" title={store.loadingProgress.currentFile}>
          {store.loadingProgress.currentFile}
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .loading-overlay {
    position: fixed;
    inset: 0;
    z-index: 200;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .loading-dialog {
    width: 360px;
    background: #1a1a2e;
    border: 1px solid #3a3a5a;
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    overflow: hidden;
  }

  .loading-header {
    padding: 10px 16px;
    font-size: 0.9rem;
    font-weight: 600;
    color: #e0e0e0;
    border-bottom: 1px solid #2a2a4a;
  }

  .loading-body {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .progress-text {
    font-size: 0.85rem;
    color: #8ab4f8;
    text-align: center;
    font-family: "Consolas", "Courier New", monospace;
  }

  .progress-bar-track {
    height: 6px;
    background: #2a2a4a;
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-bar-fill {
    height: 100%;
    background: #6c9bd2;
    border-radius: 3px;
    transition: width 0.2s ease;
  }

  .current-file {
    font-size: 0.8rem;
    color: #888;
    text-align: center;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
