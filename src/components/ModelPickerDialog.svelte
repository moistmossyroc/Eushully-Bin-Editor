<script lang="ts">
  import { store } from "../lib/stores.svelte";

  export interface ExportOptions {
    model: string;
    /** When true, generate a parallel launcher (translate-parallel.bat/.sh)
     *  alongside the sequential one. Only meaningful for multi-tab exports. */
    parallel: boolean;
    /** Max concurrent claude sessions when parallel mode runs. */
    parallelLimit: number;
  }

  interface Props {
    defaultModel: string;
    title: string;
    /** When true, show the parallel option (multi-tab export). */
    showParallel: boolean;
    onConfirm: (opts: ExportOptions) => void;
    onCancel: () => void;
  }

  let { defaultModel, title, showParallel, onConfirm, onCancel }: Props = $props();
  let selected = $state(defaultModel);
  let parallel = $state(false);
  let parallelLimit = $state(4);

  function handleConfirm() {
    if (!selected) return;
    onConfirm({ model: selected, parallel, parallelLimit });
  }
</script>

<div class="dialog-backdrop" onclick={onCancel} onkeydown={(e) => e.key === "Escape" && onCancel()} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <h2>{title}</h2>
    <p class="subtitle">The chosen model is written into the export's launcher scripts (translate.bat / translate-all.bat) via <code>--model</code>.</p>

    <label class="field">
      <span>Claude model</span>
      <select bind:value={selected}>
        {#each store.availableModels as model}
          <option value={model.id}>{model.label} <span class="opt-id">— {model.id}</span></option>
        {/each}
      </select>
    </label>

    <p class="hint">Default is your active tab's current model. Change here to override for this export only.</p>

    {#if showParallel}
      <label class="field checkbox-field">
        <input type="checkbox" bind:checked={parallel} />
        <span>Run subfolders in parallel (headless)</span>
      </label>
      <p class="hint">
        When enabled, generates <code>translate-parallel.bat</code>/<code>.sh</code> that spawns multiple Claude sessions concurrently (via <code>claude -p</code>). Faster on machines with cores to spare, but uses more memory + API tokens at once. The single-session <code>translate-all.bat</code> is still generated for sequential use.
      </p>
      {#if parallel}
        <label class="field">
          <span>Max concurrent sessions</span>
          <input
            type="number"
            min="1"
            max="16"
            bind:value={parallelLimit}
          />
        </label>
      {/if}
    {/if}

    <div class="actions">
      <button class="btn cancel" onclick={onCancel}>Cancel</button>
      <button class="btn confirm" onclick={handleConfirm} disabled={!selected}>Continue</button>
    </div>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed; inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex; align-items: center; justify-content: center;
    z-index: 1000;
  }
  .dialog {
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    padding: 20px 24px;
    min-width: 460px;
    max-width: 560px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  h2 {
    margin: 0 0 4px 0;
    color: #8ab4f8;
    font-size: 1.05rem;
  }
  .subtitle {
    margin: 0 0 14px 0;
    color: #a0a0c0;
    font-size: 0.82rem;
    line-height: 1.4;
  }
  .subtitle code {
    background: #16213e;
    padding: 1px 5px;
    border-radius: 3px;
    color: #c0d0f0;
    font-size: 0.78rem;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 10px;
  }
  .field span {
    color: #c0c0e0;
    font-size: 0.85rem;
    font-weight: 600;
  }
  select {
    padding: 6px 8px;
    background: #16213e;
    color: #d0d0e0;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    font-size: 0.85rem;
    font-family: inherit;
  }
  select:focus, input[type="number"]:focus {
    outline: none;
    border-color: #6c9bd2;
  }
  input[type="number"] {
    padding: 6px 8px;
    background: #16213e;
    color: #d0d0e0;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    font-size: 0.85rem;
    font-family: inherit;
    width: 100px;
  }
  .checkbox-field {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  .checkbox-field input[type="checkbox"] {
    accent-color: #8ab4f8;
    cursor: pointer;
  }
  .hint code {
    background: #16213e;
    padding: 1px 5px;
    border-radius: 3px;
    color: #c0d0f0;
    font-size: 0.78rem;
  }
  .opt-id {
    color: #888;
    font-family: monospace;
    font-size: 0.78rem;
  }
  .hint {
    margin: 0 0 14px 0;
    color: #888;
    font-size: 0.78rem;
    line-height: 1.4;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .btn {
    padding: 7px 16px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .btn:hover:not(:disabled) { background: #2a2a5e; }
  .btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .btn.confirm {
    background: #2a3a6e;
    border-color: #6c9bd2;
    color: #8ab4f8;
    font-weight: 600;
  }
  .btn.confirm:hover:not(:disabled) { background: #3a4a8e; }
</style>
