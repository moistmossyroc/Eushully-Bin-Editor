<script lang="ts">
  // Browse & Extract workspace (Eushully suite, merge phase 1): open any game
  // folder level (AGE SYS3-5 / Parente; subfolders searched), browse with
  // filters, preview, extract. Backend: src-tauri/src/assets (eushully-assets engine).
  import { assetsStore as assets } from "./assetsStore.svelte";
  import { openGameAt, pickAndOpenGame } from "./assetsActions";
  import AssetsFilterBar from "./AssetsFilterBar.svelte";
  import AssetsEntryTable from "./AssetsEntryTable.svelte";
  import AssetsPreviewPane from "./AssetsPreviewPane.svelte";
  import AssetsExtractBar from "./AssetsExtractBar.svelte";
  import { pathRelativeTo } from "./utils";
</script>

<div class="workspace">
  <div class="game-bar">
    <button class="open" onclick={pickAndOpenGame} disabled={assets.opening}>
      {assets.opening ? "Opening…" : "Open game folder…"}
    </button>
    {#if assets.game}
      <span class="root" title={assets.game.root}>{assets.game.root}</span>
      <span class="summary">
        {assets.game.version} · {assets.game.entryCount.toLocaleString()} files · {assets.game.archiveCount} archives
        {#if assets.game.notEffectiveCount > 0}· {assets.game.notEffectiveCount.toLocaleString()} not loaded by the game{/if}
      </span>
    {:else if assets.preferences.lastGamePath && !assets.opening}
      <button class="recent" onclick={() => openGameAt(assets.preferences.lastGamePath)} title="Reopen the last game">
        ↺ {assets.preferences.lastGamePath}
      </button>
    {/if}
  </div>

  {#if assets.openError}
    <div class="banner error">
      ⚠ {assets.openError}
      <button class="dismiss" onclick={() => (assets.openError = null)}>×</button>
    </div>
  {/if}

  {#if assets.gameChoices}
    <div class="chooser">
      <h3>Several games found — pick one</h3>
      {#each assets.gameChoices as root (root.path)}
        <button class="choice" onclick={() => openGameAt(root.path)}>
          <span class="kind">{root.kind === "age" ? "AGE" : "Parente"}</span>
          <span>{pathRelativeTo(root.path, assets.pickedFolder)}</span>
        </button>
      {/each}
      <button class="cancel" onclick={() => (assets.gameChoices = null)}>Cancel</button>
    </div>
  {:else if assets.game}
    <div class="main">
      <div class="list-column">
        <AssetsFilterBar />
        <AssetsEntryTable />
        <AssetsExtractBar />
      </div>
      <div class="preview-column">
        <AssetsPreviewPane />
      </div>
    </div>
  {:else}
    <div class="empty">
      <p>Open a game folder to browse and extract its files.</p>
      <p class="hint">
        Any level works: the game itself, its install folder, or a folder holding several games.
        AGE titles (SYS3/SYS4/SYS5INI.BIN, .fanpatch mods) and Parente (.arc) are supported.
      </p>
    </div>
  {/if}
</div>

<style>
  .workspace {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    color: #e0e0e0;
  }
  .game-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    border-bottom: 1px solid #2a2a4a;
    font-size: 0.85rem;
  }
  .open {
    padding: 6px 14px;
    border: 1px solid #6c9bd2;
    border-radius: 4px;
    background: #2a3a6e;
    color: #fff;
    cursor: pointer;
  }
  .open:disabled {
    opacity: 0.6;
  }
  .root {
    /* Meiryo draws "\\" as "¥"; paths need a Latin UI font. */
    font-family: "Segoe UI", system-ui, sans-serif;
    font-weight: 600;
    max-width: 40%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  .summary {
    color: #8a8aa8;
  }
  .recent {
    font-family: "Segoe UI", system-ui, sans-serif;
    border: none;
    background: none;
    color: #8ab4e8;
    cursor: pointer;
    font-size: 0.82rem;
  }
  .banner.error {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    padding: 8px 12px;
    background: #3a1a22;
    color: #ffb4b4;
    font-size: 0.82rem;
    white-space: pre-wrap;
  }
  .dismiss {
    margin-left: auto;
    border: none;
    background: none;
    color: inherit;
    cursor: pointer;
    font-size: 1rem;
  }
  .main {
    display: grid;
    grid-template-columns: minmax(0, 1.4fr) minmax(320px, 1fr);
    flex: 1;
    min-height: 0;
  }
  .list-column {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid #2a2a4a;
  }
  .preview-column {
    min-height: 0;
  }
  .chooser {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px 24px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 900px;
  }
  .chooser h3 {
    margin: 0 0 8px;
    font-size: 1rem;
  }
  .choice {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 8px 12px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #e0e0e0;
    text-align: left;
    cursor: pointer;
  }
  .choice:hover {
    background: #2a2a5e;
  }
  .choice .kind {
    min-width: 60px;
    color: #8ab4e8;
    font-size: 0.78rem;
  }
  .cancel {
    align-self: flex-start;
    margin-top: 6px;
    padding: 4px 12px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: none;
    color: #c0c0e0;
    cursor: pointer;
  }
  .empty {
    padding: 48px;
    text-align: center;
    color: #b0b0d0;
  }
  .empty .hint {
    color: #7a7a9a;
    font-size: 0.85rem;
    max-width: 620px;
    margin: 8px auto;
  }
</style>
