<script lang="ts">
  // Preview of the entry under the cursor. Images load from the
  // eushully-asset protocol (decoded + PNG-encoded by the backend).
  import { assetsStore as assets } from "./assetsStore.svelte";
  import { previewImageUrl } from "./ipc";
  import { formatSize, tierLabel } from "./utils";

  /** Fit the image to the pane, or show it at 1:1 (click toggles). */
  let actualSize = $state(false);
  let wrapText = $state(true);

  $effect(() => {
    void assets.preview?.row.index;
    actualSize = false;
  });

  async function copyText(text: string) {
    await navigator.clipboard.writeText(text);
  }
</script>

<section class="preview">
  {#if !assets.game}
    <p class="hint">Open a game to browse its files.</p>
  {:else if !assets.preview}
    <p class="hint">{assets.previewLoading ? "Loading…" : "Click a file to preview it. ↑/↓ browse, Space selects."}</p>
  {:else}
    {@const { row, preview } = assets.preview}
    <header>
      <div class="name" class:mod={row.tier === "mod"} class:update={row.tier === "officialUpdate"}>{row.name}</div>
      <div class="meta">
        {row.kind} · {formatSize(row.size)} · {row.archive}
        {#if row.tier !== "original"}<span class="tier {row.tier}">{tierLabel(row.tier)}</span>{/if}
      </div>
      {#if row.note}<div class="note">⚠ {row.note}</div>{/if}
    </header>

    <div class="body" class:busy={assets.previewLoading}>
      {#if preview.kind === "image"}
        <button
          class="image-frame checkerboard"
          class:actual={actualSize}
          onclick={() => (actualSize = !actualSize)}
          title={actualSize ? "Click: fit to pane" : "Click: actual size"}
        >
          <img src={previewImageUrl(assets.game.session, row.index)} alt={row.name} />
        </button>
        <div class="footer">{preview.width} × {preview.height}{actualSize ? " · 100%" : " · fit"}</div>
      {:else if preview.kind === "text"}
        <div class="text-tools">
          <label><input type="checkbox" bind:checked={wrapText} /> Wrap</label>
          <button class="small" onclick={() => copyText(preview.text)}>Copy</button>
        </div>
        <pre class:wrap={wrapText}>{preview.text}</pre>
      {:else if preview.kind === "audio"}
        <p class="info">
          Audio ({preview.format}, {formatSize(preview.size)}{preview.durationSecs ? `, ${preview.durationSecs.toFixed(1)} s` : ""}).
          In-app playback arrives in a later suite phase; extract it to listen.
        </p>
      {:else if preview.kind === "video"}
        <p class="info">{preview.description}<br />In-app playback arrives in a later suite phase; extract it to watch.</p>
      {:else if preview.kind === "hex"}
        <pre class="hex">{preview.text}</pre>
      {:else}
        <p class="error">⚠ {preview.message}</p>
      {/if}
    </div>
  {/if}
</section>

<style>
  .preview {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
    font-size: 0.85rem;
  }
  .hint {
    color: #7a7a9a;
    padding: 16px;
  }
  header {
    padding: 10px 12px;
    border-bottom: 1px solid #2a2a4a;
  }
  .name {
    font-weight: 600;
    word-break: break-all;
  }
  .name.mod {
    color: #f07a90;
  }
  .name.update {
    color: #78aae6;
  }
  .meta {
    color: #8a8aa8;
    margin-top: 2px;
    font-size: 0.8rem;
  }
  .tier {
    margin-left: 6px;
    padding: 1px 6px;
    border-radius: 999px;
    font-size: 0.72rem;
    border: 1px solid currentColor;
  }
  .tier.mod {
    color: #f07a90;
  }
  .tier.officialUpdate {
    color: #78aae6;
  }
  .note {
    color: #e0b04a;
    margin-top: 4px;
    font-size: 0.8rem;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 8px;
    gap: 6px;
  }
  .body.busy {
    opacity: 0.6;
  }
  .image-frame {
    flex: 1;
    min-height: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    border: 1px solid #2a2a4a;
    border-radius: 4px;
    padding: 0;
    cursor: zoom-in;
  }
  .image-frame img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    image-rendering: auto;
  }
  .image-frame.actual {
    overflow: auto;
    align-items: flex-start;
    justify-content: flex-start;
    cursor: zoom-out;
  }
  .image-frame.actual img {
    max-width: none;
    max-height: none;
  }
  .checkerboard {
    background-color: #2a252b;
    background-image:
      linear-gradient(45deg, #3a343b 25%, transparent 25%),
      linear-gradient(-45deg, #3a343b 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, #3a343b 75%),
      linear-gradient(-45deg, transparent 75%, #3a343b 75%);
    background-size: 16px 16px;
    background-position: 0 0, 0 8px, 8px -8px, -8px 0;
  }
  .footer {
    color: #8a8aa8;
    font-size: 0.78rem;
  }
  .text-tools {
    display: flex;
    gap: 10px;
    align-items: center;
    color: #c0c0e0;
  }
  pre {
    flex: 1;
    min-height: 0;
    margin: 0;
    overflow: auto;
    padding: 8px;
    background: #12122a;
    border: 1px solid #2a2a4a;
    border-radius: 4px;
    font-family: "Cascadia Mono", Consolas, "MS Gothic", monospace;
    font-size: 0.8rem;
    white-space: pre;
  }
  pre.wrap {
    white-space: pre-wrap;
    word-break: break-word;
  }
  .info {
    color: #b0b0d0;
  }
  .error {
    color: #ff8a80;
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
</style>
