/**
 * Centralized settings.json persistence. `save_settings` on the Rust side is a
 * full overwrite (not a merge), so anyone writing to settings.json must include
 * every persisted field. This helper snapshots the current store and writes the
 * whole blob in one call.
 *
 * Callers: SettingsDialog (on field change), ResizeHandle drag-end (panel widths).
 */
import { store } from "./stores.svelte";
import { saveSettings } from "./ipc";
import { toastWarn } from "./toast";

export async function persistAllSettings(): Promise<void> {
  try {
    await saveSettings({
      translatedCharCap: store.translatedCharCap,
      translatedLineCharCap: store.translatedLineCharCap,
      maxLinesPerBox: store.maxLinesPerBox,
      bottomLineCharCap: store.bottomLineCharCap,
      binViewerMode: store.binViewerMode,
      enforceCharLimits: store.enforceCharLimits,
      crossBinContext: store.crossBinContext,
      minimalNotes: store.minimalNotes,
      fastMode: store.fastMode,
      skipGlossary: store.skipGlossary,
      scopedGlossary: store.scopedGlossary,
      blockMode: store.blockMode,
      contextBlocks: store.contextBlocks,
      autoSplitEnabled: store.autoSplitEnabled,
      autoSplitOnImport: store.autoSplitOnImport,
      boxBridgeMode: store.boxBridgeMode,
      keepSpeakerPlateOnContinuation: store.keepSpeakerPlateOnContinuation,
      lastParenteExportDir: store.lastParenteExportDir,
      lastSysGameDir: store.lastSysGameDir,
      gameAccuratePreview: store.gameAccuratePreview,
      previewSlotByPrefix: store.previewSlotByPrefix,
      boxProfiles: store.boxProfiles,
      lastParenteExportName: store.lastParenteExportName,
      spScenePrefixes: store.spScenePrefixes,
      workspaceFolders: store.workspaceFolders,
      parenteProjectDirs: store.parenteProjectDirs,
      fanpatchTitle: store.fanpatchTitle,
      fanpatchLabel: store.fanpatchLabel,
      fanpatchUiConfigPath: store.fanpatchUiConfigPath,
      fanpatchResizableWindow: store.fanpatchResizableWindow,
      fanpatchDevMenu: store.fanpatchDevMenu,
      fanpatchCrispHd: store.fanpatchCrispHd,
      fanpatchWordWrap: store.fanpatchWordWrap,
      apiProvider: store.apiProvider,
      customEndpoint: store.customEndpoint,
      lastAnthropicModel: store.activeTab?.anthropicModel ?? store.lastAnthropicModel,
      lastOpenAiCompatModel: store.activeTab?.openAiCompatModel ?? store.lastOpenAiCompatModel,
      panelWidths: store.panelWidths,
      recentFiles: store.recentFiles,
      autoRestoreSession: store.autoRestoreSession,
      lastSessionPaths: store.lastSessionPaths,
      confirmTabClose: store.confirmTabClose,
      globalGlossary: store.globalGlossary,
    });
  } catch (err) {
    // Surface but don't crash — settings.json write failures are rare
    // (permissions, disk full) but the user should know.
    toastWarn(`Failed to save settings: ${err}`);
  }
}

/** Debounced variant — for drag events firing per-frame. */
let pendingTimer: number | null = null;
export function persistAllSettingsDebounced(delayMs = 250): void {
  if (pendingTimer !== null) clearTimeout(pendingTimer);
  pendingTimer = setTimeout(() => {
    pendingTimer = null;
    void persistAllSettings();
  }, delayMs) as unknown as number;
}
