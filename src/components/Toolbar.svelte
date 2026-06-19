<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { store } from "../lib/stores.svelte";
  import {
    doExportBin, doExportAllBins, doLoadFile, doLoadProjectFolders,
    doExportForTranslation, doImportTranslations,
    doExportAllTabsForTranslation, doImportAllTranslations, doCheckTranslationStatus,
    forgetRecentFile, clearRecentFiles, doOpenParenteGameFolder,
  } from "../lib/actions";
  import SaveExportPromptDialog from "./SaveExportPromptDialog.svelte";
  import AboutDialog from "./AboutDialog.svelte";

  let aboutOpen = $state(false);

  let promptOpen = $state(false);
  let openMenuOpen = $state(false);
  let ccMenuOpen = $state(false);
  let tabCount = $derived(store.tabs.length);

  async function handleLoad() {
    openMenuOpen = false;
    const selected = await open({
      multiple: true,
      filters: [
        { name: "Supported Files", extensions: ["BIN", "bin", "json", "arc"] },
        { name: "BIN Files", extensions: ["BIN", "bin"] },
        { name: "Project Files", extensions: ["json"] },
        { name: "Parente Archive", extensions: ["arc"] },
      ],
    });

    if (!selected) return;
    // Tauri 2 open() with multiple:true returns string[]; with false, string.
    const paths = Array.isArray(selected) ? selected : [selected];

    if (paths.length > 1) {
      store.loadingProgress = { current: 0, total: paths.length, currentFile: "" };
    }

    for (let i = 0; i < paths.length; i++) {
      const filename = paths[i].replace(/\\/g, "/").split("/").pop() || paths[i];
      if (store.loadingProgress) {
        store.loadingProgress = { current: i, total: paths.length, currentFile: filename };
      }
      await doLoadFile(paths[i]);
    }

    store.loadingProgress = null;
  }

  function handleLoadFolders() {
    openMenuOpen = false;
    doLoadProjectFolders();
  }

  async function handleOpenGameFolder() {
    openMenuOpen = false;
    const dir = await open({ directory: true, title: "Select the game's Start (or Archives) folder" });
    if (typeof dir === "string") await doOpenParenteGameFolder(dir);
  }

  function handleExport() {
    if (store.tabs.length > 1) {
      promptOpen = true;
    } else {
      doExportBin();
    }
  }

  function handleCurrentOnly() {
    promptOpen = false;
    doExportBin();
  }

  function handleAll() {
    promptOpen = false;
    doExportAllBins();
  }
</script>

<div class="toolbar">
  <div class="toolbar-group">
    <div class="open-menu-wrapper">
      <button class="toolbar-btn" onclick={() => (openMenuOpen = !openMenuOpen)}>
        Open
      </button>
      {#if openMenuOpen}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div class="menu-backdrop" onclick={() => (openMenuOpen = false)}></div>
        <div class="open-menu open-menu-wide">
          <button class="menu-item" onclick={handleLoad}>Open Files…</button>
          <button class="menu-item" onclick={handleLoadFolders}>Open Project Folders…</button>
          <button class="menu-item" onclick={handleOpenGameFolder}>Open Game Folder… (Parente)</button>
          {#if store.recentFiles.length > 0}
            <div class="menu-sep"></div>
            <div class="menu-section-label">Recent</div>
            {#each store.recentFiles as path}
              {@const fname = path.replace(/\\/g, "/").split("/").pop() || path}
              <div class="recent-row">
                <button
                  class="menu-item recent-item"
                  title={path}
                  onclick={async () => {
                    openMenuOpen = false;
                    try { await doLoadFile(path); }
                    catch (e) { store.statusMessage = `Failed to open: ${e}`; }
                  }}
                >{fname}</button>
                <button
                  class="recent-forget"
                  title="Remove from Recent"
                  onclick={(e) => { e.stopPropagation(); forgetRecentFile(path); }}
                >×</button>
              </div>
            {/each}
            <div class="menu-sep"></div>
            <button class="menu-item menu-item-muted" onclick={() => { openMenuOpen = false; clearRecentFiles(); }}>
              Clear Recent
            </button>
          {/if}
        </div>
      {/if}
    </div>
    <button
      class="toolbar-btn"
      onclick={handleExport}
      disabled={!store.activeTab}
    >
      Export BIN
    </button>
    <div class="open-menu-wrapper">
      <button
        class="toolbar-btn"
        onclick={() => (ccMenuOpen = !ccMenuOpen)}
        disabled={!store.activeTab}
        title="Export/import translation JSON for offline translation with Claude Code"
      >
        CC Translate
      </button>
      {#if ccMenuOpen}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div class="menu-backdrop" onclick={() => (ccMenuOpen = false)}></div>
        <div class="open-menu">
          <button class="menu-item" onclick={() => { ccMenuOpen = false; doExportForTranslation(); }}>
            Export current tab
          </button>
          <button class="menu-item" onclick={() => { ccMenuOpen = false; doImportTranslations(); }}>
            Import to current tab
          </button>
          <div class="menu-sep"></div>
          <button class="menu-item" onclick={() => { ccMenuOpen = false; doExportAllTabsForTranslation(); }}>
            Export all open tabs
          </button>
          <button class="menu-item" onclick={() => { ccMenuOpen = false; doImportAllTranslations(); }}>
            Import all (match by hash)
          </button>
          <div class="menu-sep"></div>
          <button class="menu-item" onclick={() => { ccMenuOpen = false; doCheckTranslationStatus(); }}>
            Check translation progress
          </button>
        </div>
      {/if}
    </div>
  </div>

  <div class="toolbar-group">
    <button
      class="toolbar-btn"
      class:active={store.charactersPanelOpen}
      onclick={() => (store.charactersPanelOpen = !store.charactersPanelOpen)}
      disabled={!store.activeTab}
    >
      Characters
    </button>
    <button
      class="toolbar-btn"
      class:active={store.glossaryPanelOpen}
      onclick={() => (store.glossaryPanelOpen = !store.glossaryPanelOpen)}
      disabled={!store.activeTab}
    >
      Glossary
    </button>
    <button
      class="toolbar-btn"
      class:active={store.globalSearchPanelOpen}
      onclick={() => (store.globalSearchPanelOpen = !store.globalSearchPanelOpen)}
      disabled={store.tabs.length === 0}
      title="Search across all open tabs (Ctrl+Shift+F)"
    >
      Find
    </button>
    <button
      class="toolbar-btn"
      class:active={store.statsPanelOpen}
      onclick={() => (store.statsPanelOpen = !store.statsPanelOpen)}
      disabled={store.tabs.length === 0}
      title="Cross-tab translation progress dashboard"
    >
      Stats
    </button>
    <button
      class="toolbar-btn"
      onclick={() => (store.binViewerOpen = true)}
      disabled={!store.activeTab}
    >
      BIN Viewer
    </button>
    <button
      class="toolbar-btn"
      onclick={() => (store.settingsDialogOpen = true)}
    >
      Settings
    </button>
    <button
      class="toolbar-btn"
      onclick={() => (aboutOpen = true)}
      title="App version + project info"
    >
      About
    </button>
  </div>

  {#if store.activeTab?.sourceBinFilename}
    <div class="toolbar-info">
      <span class="filename">{store.activeTab.sourceBinFilename}</span>
      {#if store.activeTab.format === "parente"}
        <span class="format-badge parente-badge">Parente</span>
      {:else}
        <span class="format-badge">SYS{store.activeTab.sysVersion}</span>
      {/if}
    </div>
  {/if}
</div>

{#if promptOpen}
  <SaveExportPromptDialog
    mode="export"
    count={tabCount}
    onCurrentOnly={handleCurrentOnly}
    onAll={handleAll}
    onClose={() => (promptOpen = false)}
  />
{/if}

{#if aboutOpen}
  <AboutDialog onClose={() => (aboutOpen = false)} />
{/if}

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 8px 12px;
    background: #16213e;
    border-bottom: 1px solid #2a2a4a;
  }

  .toolbar-group {
    display: flex;
    gap: 4px;
  }

  .open-menu-wrapper {
    position: relative;
  }

  .menu-backdrop {
    position: fixed;
    inset: 0;
    z-index: 99;
  }

  .open-menu {
    position: absolute;
    top: 100%;
    left: 0;
    margin-top: 4px;
    z-index: 100;
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
    min-width: 180px;
    overflow: hidden;
  }

  .menu-item {
    display: block;
    width: 100%;
    padding: 8px 14px;
    border: none;
    background: none;
    color: #c0c0e0;
    font-size: 0.85rem;
    text-align: left;
    cursor: pointer;
    transition: background 0.1s;
  }

  .menu-item:hover {
    background: #2a2a5e;
  }

  .menu-item + .menu-item {
    border-top: 1px solid #2a2a4a;
  }

  .open-menu-wide {
    min-width: 280px;
    max-height: 400px;
    overflow-y: auto;
  }
  .menu-section-label {
    padding: 6px 14px 2px 14px;
    font-size: 0.65rem;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #6a6a8a;
    background: #14142a;
  }
  .recent-row {
    display: flex;
    align-items: stretch;
    border-top: 1px solid #2a2a4a;
  }
  .recent-row .recent-item {
    flex: 1;
    border-top: none !important;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding-right: 4px;
  }
  .recent-forget {
    width: 26px;
    background: transparent;
    border: none;
    color: #555;
    cursor: pointer;
    font-size: 1rem;
    padding: 0;
    flex-shrink: 0;
  }
  .recent-forget:hover {
    color: #e06060;
    background: rgba(224, 96, 96, 0.15);
  }
  .menu-item-muted {
    color: #888;
    font-size: 0.78rem;
    font-style: italic;
  }

  .menu-sep {
    height: 1px;
    background: #2a2a4a;
    margin: 4px 0;
  }

  .toolbar-btn {
    padding: 6px 14px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.85rem;
    transition: all 0.15s;
  }

  .toolbar-btn:hover:not(:disabled) {
    background: #2a2a5e;
    border-color: #5a5a8a;
  }

  .toolbar-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .toolbar-btn.active {
    background: #2a3a6e;
    border-color: #6c9bd2;
  }

  .toolbar-info {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .filename {
    color: #6c9bd2;
    font-size: 0.85rem;
  }

  .format-badge {
    padding: 2px 6px;
    border-radius: 3px;
    background: #2a3a5e;
    color: #8ab4f8;
    font-size: 0.75rem;
    font-weight: 600;
  }
  /* Parente (Unity Lua) tabs use a distinct color so it's obvious this isn't
   * the traditional AGE binary SYS engine. */
  .format-badge.parente-badge {
    background: #3a2a5e;
    color: #c8a0f0;
  }

</style>
