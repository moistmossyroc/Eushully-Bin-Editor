<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { store } from "../lib/stores.svelte";
  import {
    doExportBin, doExportAllBins, doLoadFile, doLoadProjectFolders,
    doExportForTranslation, doImportTranslations,
    doExportAllTabsForTranslation, doImportAllTranslations, doCheckTranslationStatus,
    forgetRecentFile, clearRecentFiles, doOpenParenteGameFolder, doOpenSysGameFolder,
    doExportFanpatchFromTabs, doPackFanpatchFromFolder, doPackDualLanguageFanpatch,
    doSaveProject,
  } from "../lib/actions";
  import SaveExportPromptDialog from "./SaveExportPromptDialog.svelte";
  import AboutDialog from "./AboutDialog.svelte";

  let aboutOpen = $state(false);

  /** Open the Help Center at a specific section (contextual `?` links). */
  function openHelp(section: string) {
    store.helpSection = section;
    store.helpOpen = true;
  }

  let promptOpen = $state(false);
  let openMenuOpen = $state(false);
  let ccMenuOpen = $state(false);
  let exportMenuOpen = $state(false);
  let tabCount = $derived(store.tabs.length);

  // --- Export popover model ---------------------------------------------------
  // Context-aware enablement. `hasBinTabs` mirrors doExportFanpatchFromTabs's own
  // guard (≥1 non-Parente BIN tab with entries). The folder-based fanpatch
  // builders pick their inputs at runtime, so they're always available.
  let hasActiveTab = $derived(!!store.activeTab);
  let hasBinTabs = $derived(
    store.tabs.some((t) => t.format !== "parente" && t.entries.length > 0),
  );

  interface ExportOption {
    label: string;
    desc: string;
    disabled: boolean;
    /** Shown (inline + as title) when disabled, explaining why. */
    reason?: string;
    run: () => void;
  }
  interface ExportGroup {
    title: string;
    items: ExportOption[];
  }

  let exportGroups: ExportGroup[] = $derived([
    {
      title: "Game build",
      items: [
        {
          label: "Export BIN…",
          desc: "Translated .BIN to drop into the game's data folder.",
          disabled: !hasActiveTab,
          reason: "Open a tab first",
          run: handleExport,
        },
        {
          label: "Export open tabs as .fanpatch",
          desc: "Single zip the loader serves; overrides base scenes. Built from your open BIN tabs.",
          disabled: !hasBinTabs,
          reason: "No BIN tabs open",
          run: doExportFanpatchFromTabs,
        },
        {
          label: "Build .fanpatch from folder…",
          desc: "Zip an existing folder of translated BINs into a loader .fanpatch.",
          disabled: false,
          run: doPackFanpatchFromFolder,
        },
        {
          label: "Repair speaker boxes → .fanpatch…",
          desc: "Fix narrator/speaker boxes from EN + original JP folders; outputs a .fanpatch.",
          disabled: false,
          run: () => doPackDualLanguageFanpatch(false),
        },
        {
          label: "Bilingual (EN↔JP switch) → .fanpatch…",
          desc: "Adds in-game mid-scene EN↔JP toggle; needs EN + JP BIN folders.",
          disabled: false,
          run: () => doPackDualLanguageFanpatch(true),
        },
      ],
    },
    {
      title: "Translation workflow",
      items: [
        {
          label: "Save project (.json)…",
          desc: "Save translations, glossary & characters as a reloadable project file.",
          disabled: !hasActiveTab,
          reason: "Open a tab first",
          run: doSaveProject,
        },
      ],
    },
  ]);

  /** Close the popover, then fire the option's existing action. */
  function runExport(fn: () => void) {
    exportMenuOpen = false;
    fn();
  }

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

  async function handleOpenSysFolder() {
    openMenuOpen = false;
    const dir = await open({ directory: true, title: "Select the game's BIN folder (SYS3/4/5)" });
    if (typeof dir === "string") await doOpenSysGameFolder(dir);
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

<svelte:window
  onkeydown={(e) => {
    if (e.key === "Escape") { exportMenuOpen = false; openMenuOpen = false; ccMenuOpen = false; }
  }}
/>

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
          <button class="menu-item" onclick={handleOpenSysFolder}>Open Game Folder… (SYS3/4/5)</button>
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
    <div class="open-menu-wrapper">
      <button
        class="toolbar-btn"
        aria-haspopup="menu"
        aria-expanded={exportMenuOpen}
        onclick={() => (exportMenuOpen = !exportMenuOpen)}
      >
        Export
      </button>
      {#if exportMenuOpen}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div class="menu-backdrop" onclick={() => (exportMenuOpen = false)}></div>
        <div class="open-menu export-menu" role="menu" aria-label="Export options">
          {#each exportGroups as group, gi}
            {#if gi > 0}<div class="menu-sep"></div>{/if}
            <div class="menu-section-label">{group.title}</div>
            {#each group.items as item}
              <button
                class="export-item"
                role="menuitem"
                disabled={item.disabled}
                title={item.disabled && item.reason ? item.reason : item.desc}
                onclick={() => runExport(item.run)}
              >
                <span class="export-label">{item.label}</span>
                <span class="export-desc"
                  >{item.desc}{#if item.disabled && item.reason}<span class="export-reason"> — {item.reason}</span>{/if}</span
                >
              </button>
            {/each}
          {/each}
          <div class="menu-hint">Dual-language builders ask for translated + original BIN folders and bundle the version.dll loader; drop the output into the game folder.
            <button class="menu-help-link" onclick={() => { exportMenuOpen = false; openHelp("bilingual"); }}>Learn more →</button>
          </div>
        </div>
      {/if}
    </div>
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
    <button
      class="toolbar-btn"
      onclick={() => openHelp("getting-started")}
      title="Help Center (F1)"
    >
      ? Help
    </button>
  </div>

  {#if store.activeTab?.sourceBinFilename}
    <div class="toolbar-info">
      <span class="filename">{store.activeTab.sourceBinFilename}</span>
      {#if store.activeTab.format === "parente"}
        <span class="format-badge parente-badge">Parente</span>
      {:else}
        <button
          class="format-badge format-badge-btn"
          title="Detected Eushully game + AGE engine build — click for help"
          onclick={() => openHelp("loading")}
        >
          {store.activeTab.engineLabel || `SYS${store.activeTab.sysVersion}`}
        </button>
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

  /* Export popover: wider than the plain menus so each option's one-line
   * description fits on a single row under its label. */
  .export-menu {
    min-width: 320px;
    max-width: 360px;
    max-height: 460px;
    overflow-y: auto;
    padding-bottom: 2px;
  }
  .export-item {
    display: block;
    width: 100%;
    padding: 7px 14px;
    border: none;
    background: none;
    text-align: left;
    cursor: pointer;
    transition: background 0.1s;
  }
  .export-item:hover:not(:disabled) {
    background: #2a2a5e;
  }
  .export-item:disabled {
    cursor: not-allowed;
  }
  .export-label {
    display: block;
    color: #c0c0e0;
    font-size: 0.85rem;
  }
  .export-item:disabled .export-label {
    color: #6a6a8a;
  }
  .export-desc {
    display: block;
    margin-top: 2px;
    color: #777;
    font-size: 0.72rem;
    line-height: 1.3;
  }
  .export-reason {
    color: #c08a8a;
    font-style: italic;
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

  .menu-hint {
    padding: 6px 12px 2px;
    color: #777;
    font-size: 0.72rem;
    line-height: 1.3;
  }
  .menu-help-link {
    display: inline;
    background: none;
    border: none;
    padding: 0;
    margin-left: 4px;
    color: #8ab4f8;
    font-size: 0.72rem;
    cursor: pointer;
  }
  .menu-help-link:hover {
    text-decoration: underline;
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
  .format-badge-btn {
    border: none;
    cursor: pointer;
    font-family: inherit;
  }
  .format-badge-btn:hover {
    background: #34487a;
  }
  /* Parente (Unity Lua) tabs use a distinct color so it's obvious this isn't
   * the traditional AGE binary SYS engine. */
  .format-badge.parente-badge {
    background: #3a2a5e;
    color: #c8a0f0;
  }

</style>
