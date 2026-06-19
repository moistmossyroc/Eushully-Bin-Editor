<script lang="ts">
  /**
   * Launch-time prompt offering to reopen the BINs that were open when the app
   * was last closed. Shown only when "Auto-restore last session" is enabled and
   * the prior session had open tabs. The user explicitly confirms — we don't
   * silently reload, so a stale/huge session never surprises them on startup.
   */
  interface Props {
    paths: string[];
    onRestore: () => void;
    onDismiss: () => void;
  }
  let { paths, onRestore, onDismiss }: Props = $props();

  /** Display just the filename; full path in the title attribute. */
  function basename(p: string): string {
    return p.replace(/\\/g, "/").split("/").pop() || p;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onDismiss();
    else if (e.key === "Enter") onRestore();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onclick={onDismiss}></div>

<div class="dialog" role="dialog" aria-label="Restore previous session">
  <header>
    <h3>Restore previous session?</h3>
  </header>

  <div class="body">
    <p class="lead">
      {paths.length} tab{paths.length === 1 ? "" : "s"} {paths.length === 1 ? "was" : "were"} open when you last closed the editor.
    </p>
    <ul class="path-list">
      {#each paths as p (p)}
        <li title={p}>{basename(p)}</li>
      {/each}
    </ul>
  </div>

  <footer>
    <button class="btn" onclick={onDismiss}>Start fresh</button>
    <button class="btn primary" onclick={onRestore}>Restore {paths.length} tab{paths.length === 1 ? "" : "s"}</button>
  </footer>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 2000;
  }
  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 2001;
    width: 460px;
    max-width: 92vw;
    max-height: 80vh;
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
    padding: 14px 16px;
    overflow-y: auto;
  }
  .lead {
    margin: 0 0 10px 0;
    color: #c0c0e0;
    font-size: 0.85rem;
  }
  .path-list {
    margin: 0;
    padding: 0;
    list-style: none;
    border: 1px solid #2a2a4a;
    border-radius: 4px;
    background: #1a1a3e;
    max-height: 240px;
    overflow-y: auto;
  }
  .path-list li {
    padding: 5px 10px;
    font-family: monospace;
    font-size: 0.8rem;
    color: #c0d0f0;
    border-bottom: 1px solid #20204a;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .path-list li:last-child { border-bottom: none; }
  footer {
    padding: 12px 16px;
    border-top: 1px solid #2a2a4a;
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
  .btn {
    padding: 6px 14px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .btn:hover { background: #2a2a5e; }
  .btn.primary {
    background: #2a3a6e;
    border-color: #5a7aff;
    color: #fff;
    font-weight: 600;
  }
  .btn.primary:hover { background: #3a4a7e; }
</style>
