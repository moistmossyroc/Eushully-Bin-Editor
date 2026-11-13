<script lang="ts">
  // Output folder, options, the labelled Extract button, progress + result.
  import { assetsStore as assets } from "./assetsStore.svelte";
  import { cancelRunningExtraction, extractChosen, openLastOutputFolder, pickOutputFolder } from "./assetsActions";
  import { AGF_FORMAT_LABELS, type AgfFormat } from "./types";
  import { extractButtonLabel } from "./utils";

  const AGF_FORMATS = Object.keys(AGF_FORMAT_LABELS) as AgfFormat[];

  let isParente = $derived(assets.game?.version.startsWith("Parente") ?? false);
  let scopeCount = $derived(assets.selected.size > 0 ? assets.selected.size : assets.listing.length);
  let scope = $derived<"selected" | "shown">(assets.selected.size > 0 ? "selected" : "shown");
  let showErrors = $state(false);

  function savePreferences() {
    assets.savePreferences();
  }
</script>

{#if assets.game}
  <div class="extract-bar">
    <button class="small" onclick={pickOutputFolder} title="Choose the output folder">Output…</button>
    <span class="output" title={assets.preferences.outputDir}>{assets.preferences.outputDir || "(asked on first extract)"}</span>
    <label title="Write every file straight into the output folder (name clashes get _2, _3…)">
      <input type="checkbox" bind:checked={assets.preferences.flatten} onchange={savePreferences} /> Flatten
    </label>
    <label title="What AGF images are written as">
      AGF →
      <select bind:value={assets.preferences.agfFormat} onchange={savePreferences}>
        {#each AGF_FORMATS as format (format)}
          <option value={format}>{AGF_FORMAT_LABELS[format]}</option>
        {/each}
      </select>
    </label>
    {#if isParente}
      <label title="Convert FMOD FSB5 audio to WAV (needs vgmstream-cli next to the app or on PATH)">
        <input type="checkbox" bind:checked={assets.preferences.fsbToWav} onchange={savePreferences} /> FSB → WAV
      </label>
    {/if}
    <span class="spacer"></span>

    {#if assets.extraction}
      {@const progress = assets.extraction}
      <div class="progress" title={progress.currentFile}>
        <div class="bar" style:width="{(100 * progress.done) / Math.max(1, progress.total)}%"></div>
        <span>{progress.done.toLocaleString()} / {progress.total.toLocaleString()}</span>
      </div>
      <button class="small" onclick={cancelRunningExtraction}>Cancel</button>
    {:else}
      {#if assets.lastExtraction && !assets.lastExtraction.cancelled}
        <button class="small" onclick={openLastOutputFolder}>Open folder</button>
        {#if assets.lastExtraction.errors.length > 0}
          <button class="small warn" onclick={() => (showErrors = !showErrors)}>
            {assets.lastExtraction.errors.length} failed
          </button>
        {/if}
      {/if}
      <button class="primary" disabled={scopeCount === 0} onclick={extractChosen} title="Writes the copy the game uses; shadowed duplicates of chosen files are skipped">
        {extractButtonLabel(scopeCount, scope)}
      </button>
    {/if}
  </div>
  {#if showErrors && assets.lastExtraction && assets.lastExtraction.errors.length > 0}
    <div class="errors">
      <div class="errors-head">
        <span>Failed files</span>
        <button class="small" onclick={() => navigator.clipboard.writeText(assets.lastExtraction?.errors.join("\n") ?? "")}>Copy all</button>
        <button class="small" onclick={() => (showErrors = false)}>Close</button>
      </div>
      <pre>{assets.lastExtraction.errors.join("\n")}</pre>
    </div>
  {/if}
{/if}

<style>
  .extract-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-top: 1px solid #2a2a4a;
    background: #16213e;
    font-size: 0.82rem;
    color: #c0c0e0;
  }
  .output {
    /* Meiryo draws "\\" as "¥"; paths need a Latin UI font. */
    font-family: "Segoe UI", system-ui, sans-serif;
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #9a9ab8;
  }
  label {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  select {
    background: #12122a;
    color: #e0e0e0;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    padding: 2px 4px;
  }
  .spacer {
    flex: 1;
  }
  .small {
    padding: 3px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.78rem;
  }
  .small.warn {
    color: #e0b04a;
    border-color: #6a5a2a;
  }
  .primary {
    padding: 6px 14px;
    border: 1px solid #6c9bd2;
    border-radius: 4px;
    background: #2a3a6e;
    color: #fff;
    cursor: pointer;
    font-weight: 600;
  }
  .primary:hover:not(:disabled) {
    background: #3a4a8e;
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .progress {
    position: relative;
    width: 220px;
    height: 20px;
    background: #12122a;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    overflow: hidden;
  }
  .progress .bar {
    position: absolute;
    inset: 0 auto 0 0;
    background: #2a4a7e;
  }
  .progress span {
    position: relative;
    display: block;
    text-align: center;
    line-height: 18px;
    font-size: 0.75rem;
  }
  .errors {
    border-top: 1px solid #2a2a4a;
    background: #12122a;
    max-height: 180px;
    display: flex;
    flex-direction: column;
  }
  .errors-head {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 4px 10px;
    color: #e0b04a;
    font-size: 0.8rem;
  }
  .errors pre {
    margin: 0;
    padding: 4px 10px 8px;
    overflow: auto;
    font-size: 0.75rem;
    color: #d0a0a0;
  }
</style>
