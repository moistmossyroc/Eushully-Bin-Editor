<script lang="ts">
  import { store } from "../lib/stores.svelte";
</script>

{#if store.loadingProgress}
  {@const lp = store.loadingProgress}
  <div class="loading-overlay">
    <div class="loading-dialog">
      <div class="loading-header">{lp.total > 0 ? "Loading Files" : "Working…"}</div>
      <div class="loading-body">
        {#if lp.total > 0}
          <div class="progress-text">{lp.current} / {lp.total}</div>
          <div class="progress-bar-track">
            <div class="progress-bar-fill" style="width: {(lp.current / lp.total) * 100}%"></div>
          </div>
        {:else}
          <!-- Indeterminate: a single Rust call with no per-item progress. -->
          <div class="progress-bar-track">
            <div class="progress-bar-indeterminate"></div>
          </div>
        {/if}
        <div class="current-file" title={lp.currentFile}>{lp.currentFile}</div>
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

  .progress-bar-track { position: relative; }
  .progress-bar-indeterminate {
    position: absolute;
    top: 0;
    height: 100%;
    width: 35%;
    background: #6c9bd2;
    border-radius: 3px;
    animation: indeterminate 1.1s ease-in-out infinite;
  }
  @keyframes indeterminate {
    0% { left: -35%; }
    100% { left: 100%; }
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
