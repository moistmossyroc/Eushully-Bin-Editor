<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Toolbar from "./components/Toolbar.svelte";
  import TabBar from "./components/TabBar.svelte";
  import EditorTable from "./components/EditorTable.svelte";
  import TranslationControls from "./components/TranslationControls.svelte";
  import CharactersPanel from "./components/CharactersPanel.svelte";
  import GlossaryPanel from "./components/GlossaryPanel.svelte";
  import GlobalSearchPanel from "./components/GlobalSearchPanel.svelte";
  import StatsPanel from "./components/StatsPanel.svelte";
  import SettingsDialog from "./components/SettingsDialog.svelte";
  import InsertDialog from "./components/InsertDialog.svelte";
  import CloseConfirmDialog from "./components/CloseConfirmDialog.svelte";
  import GlossaryReviewDialog from "./components/GlossaryReviewDialog.svelte";
  import ImportModeDialog from "./components/ImportModeDialog.svelte";
  import OverflowReviewDialog from "./components/OverflowReviewDialog.svelte";
  import GlossaryConsistencyDialog from "./components/GlossaryConsistencyDialog.svelte";
  import OmittedCellsDialog from "./components/OmittedCellsDialog.svelte";
  import ParenteFilePickerDialog from "./components/ParenteFilePickerDialog.svelte";
  import TranslationStatusDialog from "./components/TranslationStatusDialog.svelte";
  import ModelPickerDialog from "./components/ModelPickerDialog.svelte";
  import DialogPreview from "./components/DialogPreview.svelte";
  import LoadingProgress from "./components/LoadingProgress.svelte";
  import BinViewerDialog from "./components/BinViewerDialog.svelte";
  import KeyboardShortcutsDialog from "./components/KeyboardShortcutsDialog.svelte";
  import ExportDiffDialog from "./components/ExportDiffDialog.svelte";
  import RestoreSessionDialog from "./components/RestoreSessionDialog.svelte";
  import ConfirmDialog from "./components/ConfirmDialog.svelte";
  import ToastContainer from "./components/ToastContainer.svelte";
  import AutoSplitBadge from "./components/AutoSplitBadge.svelte";
  import { store } from "./lib/stores.svelte";
  import { doLoadFile, doAutoSaveAll, doRefreshTranslationStatus, doImportSingleFolder, doOpenParenteGameFolder } from "./lib/actions";
  import { loadSettings, fetchAvailableModels, getProviderConfig, setProviderConfig, getApiKey } from "./lib/ipc";
  import { detectMissingCJKFonts, isFontWarningDismissed, dismissFontWarning } from "./lib/fontCheck";

  // Auto-save when the user selects a different row
  $effect(() => {
    // Track selectedRowIndex to trigger on row change
    const _rowIndex = store.activeTab?.selectedRowIndex;
    // Track activeTabIndex to save when switching tabs
    const _tabIndex = store.activeTabIndex;

    doAutoSaveAll();
  });

  let insertDialogOpen = $state(false);
  let insertAfterIndex = $state<number | null>(null);
  let closeConfirmOpen = $state(false);
  let dragOver = $state(false);
  /** List of recommended CJK fonts the system is missing. Populated on boot.
   *  When non-empty (and the user hasn't dismissed) we show a warning banner. */
  let missingFonts = $state<string[]>([]);
  let fontWarningDismissed = $state(false);
  let shortcutsDialogOpen = $state(false);
  /** Paths from the prior session awaiting the user's restore decision. When
   *  non-null, RestoreSessionDialog is shown. */
  let pendingRestorePaths = $state<string[] | null>(null);
  /** Gate for the session-snapshot effect below. Stays false through boot +
   *  the restore decision so the effect can't clobber the saved session list
   *  before it's been consumed. Flipped true once the session is settled. */
  let booted = $state(false);

  const LOADABLE_EXTENSIONS = [".bin", ".json", ".arc"];

  /** Snapshot the open tabs' paths whenever the tab set changes so an unclean
   *  shutdown (crash, OS kill) still leaves a session to restore from. The
   *  in-memory list mirrors stores.svelte's `lastSessionPaths`; the debounced
   *  settings save flushes to disk shortly after. */
  $effect(() => {
    // Don't snapshot until boot + restore decision are settled — otherwise this
    // runs during startup with `store.tabs` still empty and wipes the saved
    // session list before auto-restore can consume it.
    if (!booted) return;
    // Track tab IDs to detect adds/removes; touching `length` would miss
    // identity changes from quick close+open of the same path.
    const _ids = store.tabs.map((t) => t.id);
    void _ids;
    // Snapshot the BIN path, not the project path. Every opened BIN gets an
    // *intended* projectPath immediately, but the .json only exists on disk once
    // the tab is edited/saved. Recording projectPath would make untouched tabs
    // point at a nonexistent .json and silently drop on restore. Loading the
    // .bin instead auto-restores its project when one exists (see doLoadBin), so
    // both edited and untouched tabs come back. Parente tabs have neither path
    // and are (still) not session-restorable.
    const paths = store.tabs
      .map((t) => t.binPath || t.projectPath)
      .filter((p): p is string => !!p);
    // Avoid spurious writes when nothing actually changed.
    const cur = store.lastSessionPaths;
    if (paths.length === cur.length && paths.every((p, i) => p === cur[i])) return;
    store.lastSessionPaths = paths;
    void import("./lib/settings").then((m) => m.persistAllSettingsDebounced());
  });

  function handleInsertInstruction(afterIndex: number) {
    insertAfterIndex = afterIndex;
    insertDialogOpen = true;
  }

  onMount(() => {
    // Cleanup registrations populated during the async boot below. We can't
    // make the onMount callback itself async because Svelte expects either a
    // sync cleanup-returning function or a Promise<never> — and an async
    // function's resolved cleanup never reaches Svelte. Track listeners in a
    // closure-shared list, return a sync function that drains them.
    const cleanups: (() => void)[] = [];
    void (async () => {
    // Font detection — warn (once) if neither preferred CJK font is installed.
    // Defer one frame so canvas measurement runs after the page's font CSS has
    // had a chance to load.
    fontWarningDismissed = isFontWarningDismissed();
    requestAnimationFrame(() => {
      missingFonts = detectMissingCJKFonts();
    });

    // Restore persisted settings
    let restorePaths: string[] = [];
    try {
      const settings = await loadSettings();
      if (typeof settings.translatedCharCap === "number") store.translatedCharCap = settings.translatedCharCap;
      if (typeof settings.translatedLineCharCap === "number") store.translatedLineCharCap = settings.translatedLineCharCap;
      if (typeof settings.maxLinesPerBox === "number" && settings.maxLinesPerBox > 0) store.maxLinesPerBox = settings.maxLinesPerBox;
      if (settings.binViewerMode === "scroll" || settings.binViewerMode === "paged") store.binViewerMode = settings.binViewerMode;
      if (typeof settings.enforceCharLimits === "boolean") store.enforceCharLimits = settings.enforceCharLimits;
      if (typeof settings.crossBinContext === "boolean") store.crossBinContext = settings.crossBinContext;
      if (typeof settings.minimalNotes === "boolean") store.minimalNotes = settings.minimalNotes;
      if (typeof settings.fastMode === "boolean") store.fastMode = settings.fastMode;
      if (typeof settings.skipGlossary === "boolean") store.skipGlossary = settings.skipGlossary;
      if (typeof settings.scopedGlossary === "boolean") store.scopedGlossary = settings.scopedGlossary;
      if (typeof settings.blockMode === "boolean") store.blockMode = settings.blockMode;
      if (typeof settings.contextBlocks === "number") store.contextBlocks = settings.contextBlocks;
      if (typeof settings.autoSplitEnabled === "boolean") store.autoSplitEnabled = settings.autoSplitEnabled;
      if (typeof settings.autoSplitOnImport === "boolean") store.autoSplitOnImport = settings.autoSplitOnImport;
      if (typeof settings.lastParenteExportDir === "string") store.lastParenteExportDir = settings.lastParenteExportDir;
      // Provider config — persisted in settings.json so it survives app restarts
      // (Rust state is in-memory only). We still push it to Rust below so
      // translation commands see the right provider on this session.
      if (settings.apiProvider === "anthropic" || settings.apiProvider === "openai-compatible") {
        store.apiProvider = settings.apiProvider;
      }
      if (typeof settings.customEndpoint === "string") store.customEndpoint = settings.customEndpoint;
      if (typeof settings.lastAnthropicModel === "string" && settings.lastAnthropicModel) {
        store.lastAnthropicModel = settings.lastAnthropicModel;
      }
      if (typeof settings.lastOpenAiCompatModel === "string") {
        store.lastOpenAiCompatModel = settings.lastOpenAiCompatModel;
      }
      if (settings.panelWidths && typeof settings.panelWidths === "object") {
        const pw = settings.panelWidths as Record<string, unknown>;
        if (typeof pw.search === "number" && pw.search > 0) store.panelWidths.search = pw.search;
        if (typeof pw.glossary === "number" && pw.glossary > 0) store.panelWidths.glossary = pw.glossary;
        if (typeof pw.characters === "number" && pw.characters > 0) store.panelWidths.characters = pw.characters;
      }
      if (Array.isArray(settings.recentFiles)) {
        store.recentFiles = settings.recentFiles.filter((p): p is string => typeof p === "string");
      }
      if (typeof settings.autoRestoreSession === "boolean") {
        store.autoRestoreSession = settings.autoRestoreSession;
      }
      if (typeof settings.confirmTabClose === "boolean") {
        store.confirmTabClose = settings.confirmTabClose;
      }
      if (Array.isArray(settings.lastSessionPaths)) {
        store.lastSessionPaths = settings.lastSessionPaths.filter((p): p is string => typeof p === "string");
        // Capture into a local NOW — immune to the snapshot effect, which would
        // otherwise reset store.lastSessionPaths to [] during the awaits below.
        restorePaths = [...store.lastSessionPaths];
      }
      if (Array.isArray(settings.globalGlossary)) {
        // Sanitize: keep only well-formed entries, coerce missing fields so the
        // panel never renders `undefined`. Global glossary is the only one that
        // lives in settings.json (project glossaries ride their tab/JSON file).
        store.globalGlossary = settings.globalGlossary
          .filter((g): g is Record<string, unknown> => !!g && typeof g === "object")
          .map((g) => ({
            japanese: typeof g.japanese === "string" ? g.japanese : "",
            english: typeof g.english === "string" ? g.english : "",
            category: typeof g.category === "string" ? g.category : "general",
            notes: typeof g.notes === "string" ? g.notes : "",
          }))
          .filter((g) => g.japanese.trim() !== "");
      }
    } catch {
      // First run or corrupted settings — use defaults
    }

    // Push provider config to Rust state so translate commands see the right
    // provider. Fallback to Rust state if settings.json had nothing yet.
    try {
      if (store.apiProvider === "openai-compatible" || store.customEndpoint) {
        await setProviderConfig(
          store.apiProvider,
          store.apiProvider === "openai-compatible" ? (store.customEndpoint || null) : null,
        );
      } else {
        const config = await getProviderConfig();
        if (config.provider === "anthropic" || config.provider === "openai-compatible") {
          store.apiProvider = config.provider;
        }
        store.customEndpoint = config.endpoint || "";
      }
    } catch {
      // Defaults
    }

    // Check if API key exists — for OpenAI-compatible, empty key is OK
    try {
      const key = await getApiKey();
      store.apiKeyConfigured = (key != null && key.length > 0) || store.apiProvider === "openai-compatible";
    } catch {
      // Defaults
    }

    // Offer to restore the prior session's tabs if the user opted in. We use
    // the locally-captured `restorePaths` (not store.lastSessionPaths, which the
    // snapshot effect may have reset during the awaits above). Showing the
    // prompt keeps `booted` false so the saved list isn't clobbered until the
    // user decides; both decision handlers flip `booted` true.
    if (store.autoRestoreSession && restorePaths.length > 0) {
      pendingRestorePaths = restorePaths;
    } else {
      booted = true;
    }

    // Auto-fetch latest models if using Anthropic and key is available (non-blocking)
    if (store.apiKeyConfigured && store.apiProvider === "anthropic") {
      fetchAvailableModels()
        .then((models) => {
          if (models.length > 0) store.availableModels = models;
        })
        .catch(() => { /* Offline or invalid key — keep defaults */ });
    }

    const appWindow = getCurrentWindow();
    const unlistenClose = appWindow.onCloseRequested(async (event) => {
      event.preventDefault();
      closeConfirmOpen = true;
    });

    const unlistenDrop = appWindow.onDragDropEvent(async (event) => {
      // Only show the file-drop overlay for actual file drags. Tauri's
      // event also fires for intra-app text/element drags (highlighted text
      // moved between cells), but those carry no paths. Without this guard
      // the overlay flashes on every internal drag and obscures the UI.
      const hasFilePaths =
        ("paths" in event.payload) &&
        Array.isArray(event.payload.paths) &&
        event.payload.paths.length > 0;

      if (event.payload.type === "enter" || event.payload.type === "over") {
        if (hasFilePaths) dragOver = true;
      } else if (event.payload.type === "leave") {
        dragOver = false;
      } else if (event.payload.type === "drop") {
        dragOver = false;
        const paths = event.payload.paths;
        const files = paths.filter((p) =>
          LOADABLE_EXTENSIONS.some((ext) => p.toLowerCase().endsWith(ext))
        );
        const others = paths.filter((p) => !files.includes(p));
        if (files.length > 1) {
          store.loadingProgress = { current: 0, total: files.length, currentFile: "" };
        }
        for (let i = 0; i < files.length; i++) {
          const filename = files[i].replace(/\\/g, "/").split("/").pop() || files[i];
          if (store.loadingProgress) {
            store.loadingProgress = { current: i, total: files.length, currentFile: filename };
          }
          await doLoadFile(files[i]);
        }
        store.loadingProgress = null;
        // A dropped folder → Parente game-folder mode. Detect "likely folder" by
        // a basename with no file extension; the backend validates it's actually
        // a game install (finds the Archives dir) and reports if not.
        const likelyDirs = others.filter((p) => {
          const base = p.replace(/\\/g, "/").split("/").pop() || "";
          return !base.includes(".");
        });
        for (const p of likelyDirs) {
          await doOpenParenteGameFolder(p);
        }
        if (files.length === 0 && likelyDirs.length === 0 && paths.length > 0) {
          store.statusMessage = "Unsupported drop — drop a .BIN, .json, .arc, or a game folder";
        }
      }
    });

    unlistenClose.then((fn) => cleanups.push(fn));
    unlistenDrop.then((fn) => cleanups.push(fn));
    })();
    return () => { cleanups.forEach((fn) => fn()); };
  });

  async function handleConfirmClose() {
    closeConfirmOpen = false;
    // Persist any pending edits (Parente projects + BIN projects) before exit.
    try { await doAutoSaveAll(); } catch { /* best-effort */ }
    await getCurrentWindow().destroy();
  }

  /** User chose to restore the prior session. Load each path sequentially;
   *  missing/moved files are skipped silently. Enables session snapshotting
   *  afterward so the (possibly reduced) set is persisted. */
  async function handleRestoreSession() {
    const paths = pendingRestorePaths ?? [];
    pendingRestorePaths = null;
    store.statusMessage = `Restoring ${paths.length} tab${paths.length === 1 ? "" : "s"} from last session…`;
    if (paths.length > 1) {
      store.loadingProgress = { current: 0, total: paths.length, currentFile: "" };
    }
    for (let i = 0; i < paths.length; i++) {
      const p = paths[i];
      if (store.loadingProgress) {
        const filename = p.replace(/\\/g, "/").split("/").pop() || p;
        store.loadingProgress = { current: i, total: paths.length, currentFile: filename };
      }
      try { await doLoadFile(p); } catch { /* skip missing/broken */ }
    }
    store.loadingProgress = null;
    store.statusMessage = `Restored ${store.tabs.length} tab${store.tabs.length === 1 ? "" : "s"}`;
    booted = true;
  }

  /** User declined — start fresh. Enabling snapshotting with no tabs open lets
   *  the effect persist an empty session list. */
  function handleDismissRestore() {
    pendingRestorePaths = null;
    booted = true;
  }

  function handleKeydown(e: KeyboardEvent) {
    // Ctrl+Shift+F (Cmd+Shift+F on mac) → toggle global search panel
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && (e.key === "F" || e.key === "f")) {
      e.preventDefault();
      if (store.tabs.length > 0) {
        store.globalSearchPanelOpen = !store.globalSearchPanelOpen;
      }
      return;
    }
    // `?` → open shortcuts dialog. Suppress if focus is inside a text input
    // (so users can actually type the character).
    if (e.key === "?" && !e.ctrlKey && !e.metaKey && !e.altKey) {
      const target = e.target as HTMLElement | null;
      const tag = target?.tagName;
      const isText = tag === "INPUT" || tag === "TEXTAREA" || target?.isContentEditable;
      if (!isText) {
        e.preventDefault();
        shortcutsDialogOpen = true;
      }
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<main class="app">
  {#if missingFonts.length > 0 && !fontWarningDismissed}
    <div class="font-warning" role="alert">
      <span class="warn-icon">⚠</span>
      <span class="warn-text">
        Recommended CJK fonts missing ({missingFonts.join(", ")}).
        Some Japanese punctuation may not render correctly. Install <strong>Meiryo</strong>
        (preinstalled on Windows Vista+) for full coverage.
      </span>
      <button
        class="warn-dismiss"
        onclick={() => { dismissFontWarning(); fontWarningDismissed = true; }}
        title="Don't show again"
      >×</button>
    </div>
  {/if}
  <Toolbar />
  <div class="tab-bar-row">
    <TabBar />
    <AutoSplitBadge />
  </div>

  {#if store.activeTab}
    <TranslationControls />
    <div class="editor-area" class:with-side-panel={store.glossaryPanelOpen || store.charactersPanelOpen || store.globalSearchPanelOpen || store.statsPanelOpen}>
      <div class="editor-main">
        <EditorTable onInsertInstruction={handleInsertInstruction} />
        <DialogPreview />
      </div>
      {#if store.charactersPanelOpen}
        <CharactersPanel />
      {/if}
      {#if store.glossaryPanelOpen}
        <GlossaryPanel />
      {/if}
      <GlobalSearchPanel />
      <StatsPanel />
    </div>
  {:else}
    <div class="empty-state">
      <p>Load a .BIN file to start a new translation project,</p>
      <p>or load a .json project file to resume.</p>
      <p class="drop-hint">You can also drag and drop files here</p>
    </div>
  {/if}

  {#if dragOver}
    <div class="drag-overlay">
      <div class="drag-overlay-content">
        Drop .BIN or .json files to open
      </div>
    </div>
  {/if}

  <footer class="status-bar">
    <span>{store.statusMessage}</span>
    <div class="status-right">
      {#if store.lastSaveTime}
        <span class="save-indicator">Saved {store.lastSaveTime}</span>
      {/if}
      {#if store.activeTab}
        <span class="entry-count">
          {store.activeTab.entries.filter((e) => !e.is_context_row && !e.deleted).length} dialogue entries
        </span>
      {/if}
    </div>
  </footer>

  {#if store.settingsDialogOpen}
    <SettingsDialog onClose={() => (store.settingsDialogOpen = false)} />
  {/if}

  {#if insertDialogOpen && insertAfterIndex !== null}
    <InsertDialog
      afterIndex={insertAfterIndex}
      onClose={() => (insertDialogOpen = false)}
    />
  {/if}

  {#if store.glossaryReviewDialogOpen && store.glossaryReviewQueue.length > 0}
    <GlossaryReviewDialog
      onClose={() => (store.glossaryReviewDialogOpen = false)}
    />
  {/if}

  {#if store.importModeRequest}
    <ImportModeDialog
      onConfirm={(mode) => {
        const req = store.importModeRequest;
        store.importModeRequest = null;
        req?.resolve(mode);
      }}
      onCancel={() => {
        const req = store.importModeRequest;
        store.importModeRequest = null;
        req?.resolve(null);
      }}
    />
  {/if}

  {#if store.overflowReviewDialogOpen && store.activeTab}
    <OverflowReviewDialog onClose={() => (store.overflowReviewDialogOpen = false)} />
  {/if}

  {#if store.glossaryConsistencyOpen && store.activeTab}
    <GlossaryConsistencyDialog onClose={() => (store.glossaryConsistencyOpen = false)} />
  {/if}

  {#if store.bulkUndo}
    <div class="bulk-undo-pill">
      <span class="bu-label">{store.bulkUndo.label} applied</span>
      <button
        class="bu-undo"
        onclick={() => { const l = store.restoreBulkUndo(); store.statusMessage = l ? `Undid: ${l}` : ""; }}
      >Undo</button>
      <button class="bu-dismiss" title="Dismiss" onclick={() => store.clearBulkUndo()}>×</button>
    </div>
  {/if}

  {#if store.omittedCellsDialogOpen && store.activeTab}
    <OmittedCellsDialog onClose={() => (store.omittedCellsDialogOpen = false)} />
  {/if}

  {#if store.parenteArcRequest}
    <ParenteFilePickerDialog
      title={store.parenteArcRequest.title}
      files={store.parenteArcRequest.files}
      onClose={() => (store.parenteArcRequest = null)}
    />
  {/if}

  {#if store.translationStatusReport}
    <TranslationStatusDialog
      parentDir={store.translationStatusReport.parentDir}
      folders={store.translationStatusReport.folders}
      onClose={() => (store.translationStatusReport = null)}
      onRefresh={() => doRefreshTranslationStatus()}
      onImport={(folder) => doImportSingleFolder(store.translationStatusReport!.parentDir, folder)}
    />
  {/if}

  {#if store.modelPickerRequest}
    <ModelPickerDialog
      defaultModel={store.modelPickerRequest.defaultModel}
      title={store.modelPickerRequest.title}
      showParallel={store.modelPickerRequest.showParallel}
      onConfirm={(opts) => {
        const req = store.modelPickerRequest;
        store.modelPickerRequest = null;
        req?.resolve(opts);
      }}
      onCancel={() => {
        const req = store.modelPickerRequest;
        store.modelPickerRequest = null;
        req?.resolve(null);
      }}
    />
  {/if}

  {#if store.binViewerOpen && store.activeTab}
    <BinViewerDialog onClose={() => (store.binViewerOpen = false)} />
  {/if}

  {#if shortcutsDialogOpen}
    <KeyboardShortcutsDialog onClose={() => (shortcutsDialogOpen = false)} />
  {/if}

  <ToastContainer />

  {#if pendingRestorePaths}
    <RestoreSessionDialog
      paths={pendingRestorePaths}
      onRestore={handleRestoreSession}
      onDismiss={handleDismissRestore}
    />
  {/if}

  {#if store.pendingExportDiff}
    <ExportDiffDialog
      binFilename={store.pendingExportDiff.binFilename}
      diff={store.pendingExportDiff.diff}
      onCancel={store.pendingExportDiff.onCancel}
      onConfirm={store.pendingExportDiff.onConfirm}
      onJump={(instr) => {
        const d = store.pendingExportDiff;
        if (!d) return;
        const ti = store.tabs.findIndex((t) => t.id === d.tabId);
        d.onCancel(); // abort the export and close the dialog
        if (ti < 0) return;
        store.switchTab(ti);
        const tab = store.tabs[ti];
        const idx = tab.entries.findIndex((e) => e.instruction_index === instr);
        if (idx < 0) return;
        tab.selectedRowIndex = idx;
        store.pendingScrollRequest = { tabId: tab.id, rowIndex: idx, nonce: Date.now() };
        store.statusMessage = `Jumped to row #${instr} — re-export when ready`;
      }}
    />
  {/if}

  <ConfirmDialog />

  <LoadingProgress />

  {#if closeConfirmOpen}
    <CloseConfirmDialog
      onClose={() => (closeConfirmOpen = false)}
      onConfirmClose={handleConfirmClose}
    />
  {/if}
</main>

<style>
  /* Floating one-click undo for bulk operations (split/paginate/replace). */
  .bulk-undo-pill {
    position: fixed;
    bottom: 18px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 1100;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px 8px 14px;
    background: #20203a;
    border: 1px solid #4a4a7a;
    border-radius: 999px;
    box-shadow: 0 6px 24px rgba(0, 0, 0, 0.5);
    font-size: 0.82rem;
    color: #d0d0e8;
  }
  .bulk-undo-pill .bu-undo {
    padding: 4px 14px;
    border: 1px solid #6c9bd2;
    border-radius: 999px;
    background: #2a3a6e;
    color: #cfe0ff;
    cursor: pointer;
    font-size: 0.8rem;
  }
  .bulk-undo-pill .bu-undo:hover { background: #3a4a8e; }
  .bulk-undo-pill .bu-dismiss {
    background: none;
    border: none;
    color: #9a9ac0;
    font-size: 1.1rem;
    line-height: 1;
    cursor: pointer;
    padding: 0 4px;
  }
  .bulk-undo-pill .bu-dismiss:hover { color: #fff; }

  /* Unified scrollbar styling across the entire app. Defined here once with
   * :global so every scrollable element matches the dark theme — replaces
   * the per-component duplicates that had drifted slightly out of sync. */
  :global(*::-webkit-scrollbar) {
    width: 10px;
    height: 10px;
  }
  :global(*::-webkit-scrollbar-track) {
    background: #14142a;
  }
  :global(*::-webkit-scrollbar-thumb) {
    background: #3a3a5a;
    border-radius: 5px;
    border: 2px solid #14142a;
  }
  :global(*::-webkit-scrollbar-thumb:hover) {
    background: #5a5a7a;
  }
  :global(*::-webkit-scrollbar-corner) {
    background: #14142a;
  }

  :global(body) {
    margin: 0;
    padding: 0;
    /* Meiryo first — it has the broadest CJK + standard-punctuation coverage
     * (renders U+2014 em dash, U+2026 ellipsis, etc. that Segoe UI doesn't
     * include in its default install). Preinstalled on every Windows Vista+
     * machine. Yu Gothic UI (Windows 8.1+) and Segoe UI are graceful fallbacks. */
    font-family: "Meiryo", "Yu Gothic UI", "Segoe UI",
      system-ui, -apple-system, sans-serif;
    background: #1a1a2e;
    color: #e0e0e0;
    overflow: hidden;
    height: 100vh;
  }

  :global(*) {
    box-sizing: border-box;
  }

  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }

  .font-warning {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px;
    background: #3a2a14;
    border-bottom: 1px solid #6a4a24;
    color: #f4c87a;
    font-size: 0.82rem;
  }
  .warn-icon {
    flex-shrink: 0;
    font-size: 1.1rem;
  }
  .warn-text {
    flex: 1;
    line-height: 1.4;
  }
  .warn-text strong {
    color: #ffd99a;
    font-weight: 600;
  }
  .warn-dismiss {
    flex-shrink: 0;
    background: transparent;
    border: 1px solid #6a4a24;
    border-radius: 3px;
    color: #f4c87a;
    width: 24px;
    height: 24px;
    cursor: pointer;
    font-size: 1rem;
    line-height: 1;
    padding: 0;
  }
  .warn-dismiss:hover {
    background: #4a3a1a;
    color: #fff;
  }

  .tab-bar-row {
    display: flex;
    align-items: stretch;
    background: #12122a;
    border-bottom: 1px solid #2a2a4a;
  }

  .tab-bar-row :global(.auto-split-badge) {
    align-self: center;
    margin-right: 8px;
    flex-shrink: 0;
  }

  .editor-area {
    flex: 1;
    display: flex;
    overflow: hidden;
    min-height: 0;
  }

  .editor-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-height: 0;
    min-width: 0;
  }

  .editor-area.with-side-panel {
    gap: 0;
  }

  .empty-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 1.1rem;
    gap: 0.25rem;
  }

  .empty-state p {
    margin: 0;
  }

  .drop-hint {
    margin-top: 8px !important;
    font-size: 0.85rem;
    color: #555;
  }

  .drag-overlay {
    position: fixed;
    inset: 0;
    z-index: 300;
    background: rgba(26, 26, 46, 0.85);
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
  }

  .drag-overlay-content {
    padding: 32px 48px;
    border: 2px dashed #6c9bd2;
    border-radius: 12px;
    color: #8ab4f8;
    font-size: 1.2rem;
    background: rgba(108, 155, 210, 0.08);
  }

  .status-bar {
    display: flex;
    justify-content: space-between;
    padding: 4px 12px;
    background: #16213e;
    border-top: 1px solid #2a2a4a;
    font-size: 0.8rem;
    color: #888;
  }

  .status-right {
    display: flex;
    gap: 16px;
  }

  .save-indicator {
    color: #6fd886;
  }

  .entry-count {
    color: #6c9bd2;
  }
</style>
