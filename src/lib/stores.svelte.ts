import type {
  CharacterEntry,
  ClaudeModel,
  DialogueEntry,
  GlossaryEntry,
  GlossaryReviewItem,
  InsertedInstruction,
  TranslationProgress,
} from "./types";

/**
 * Per-tab state. Each open BIN file gets its own TabState instance.
 */
export class TabState {
  /** Unique tab ID (used to key Rust-side state) */
  id = $state<string>("");

  /** All dialogue entries from the loaded BIN/project */
  entries = $state<DialogueEntry[]>([]);

  /** Instructions to be inserted during export */
  insertedInstructions = $state<InsertedInstruction[]>([]);

  /** Character/speaker name mappings (pointer -> name) */
  characters = $state<CharacterEntry[]>([]);

  /** Per-row speaker references keyed by instruction index */
  rowSpeakers = $state<Record<string, { source_type: number; source_raw_data: number; literal_name: string | null }>>({});

  /** Tab-local glossary (saved per-project) */
  glossary = $state<GlossaryEntry[]>([]);

  /** Path to the loaded BIN file */
  binPath = $state<string | null>(null);

  /** Path to the loaded project file */
  projectPath = $state<string | null>(null);

  /** Source BIN filename (for display) */
  sourceBinFilename = $state<string>("");

  /** Engine version number (3, 4, or 5) */
  sysVersion = $state(4);

  /** File format this tab represents. "bin" = classic AGE binary BIN; "parente"
   *  = Unity-era Lua scene script extracted from an .arc. Gates format-specific
   *  features (auto-split/bridge/BIN export are bin-only). */
  format = $state<"bin" | "parente">("bin");

  /** Parente only: source archive + byte location of this scene, used to
   *  re-read the original on export (patch is built from the source bytes). */
  parenteArcPath = $state<string | null>(null);
  parenteName = $state<string>("");
  parentePath = $state<string>("");
  parenteOffset = $state<number>(0);
  parenteSize = $state<number>(0);

  /** SHA-256 hash of the loaded BIN */
  sourceBinHash = $state<string>("");

  /** Whether there are unsaved changes */
  isDirty = $state(false);

  /** Currently selected row index in the entries array */
  selectedRowIndex = $state<number | null>(null);

  /** Editor table scroll position (px), remembered per tab so switching tabs
   *  restores where the user was rather than sharing one global scroll offset. */
  scrollTop = $state(0);

  /** Selected Claude model for this tab (Anthropic provider).
   *  Kept separately from openAiCompatModel so toggling providers in Settings
   *  preserves each one's choice. */
  anthropicModel = $state("claude-sonnet-4-6-20250627");

  /** Selected model name for this tab when using an OpenAI-compatible provider
   *  (Mistral, OpenAI, OpenRouter, Ollama, etc.). Empty until the user sets one
   *  via Settings → Apply. */
  openAiCompatModel = $state("");

  /** The model identifier that should be sent on translate calls, based on the
   *  currently-selected provider in the app store. */
  get activeModel(): string {
    return store.apiProvider === "openai-compatible" ? this.openAiCompatModel : this.anthropicModel;
  }

  /** Setter used by Settings dialog's Apply button — writes to the field
   *  matching the current provider. */
  setActiveModel(value: string) {
    if (store.apiProvider === "openai-compatible") {
      this.openAiCompatModel = value;
    } else {
      this.anthropicModel = value;
    }
  }

  /** Backwards-compat: keep `claudeModel` as a getter/setter alias for the
   *  active model so older call sites that haven't been updated still work.
   *  New code should use `activeModel` / `setActiveModel` or the explicit
   *  per-provider fields. */
  get claudeModel(): string {
    return this.activeModel;
  }
  set claudeModel(value: string) {
    this.setActiveModel(value);
  }

  /** Translation session progress */
  translationProgress = $state<TranslationProgress>({
    status: "Idle",
    current_index: 0,
    total_entries: 0,
    total_input_tokens: 0,
    total_output_tokens: 0,
    last_error: null,
  });

  /** Sticky display of the most recent translation error. Unlike
   *  `translationProgress.last_error` (which is cleared by every progress
   *  emit), this stays visible until the user dismisses it or starts a new
   *  translation. Used to surface API failures (auth, rate-limit exhaustion,
   *  invalid model name, etc.) so the user knows WHY translations stopped. */
  stickyTranslationError = $state<string | null>(null);

  /** Bridge template quality for auto-split: "clean", "degraded", "unavailable", or null (not yet fetched) */
  bridgeTemplateQuality = $state<"clean" | "degraded" | "unavailable" | null>(null);

  /** Incremented to signal all context menus to close */
  contextMenuGeneration = $state(0);

  /** Close all open context menus */
  closeAllContextMenus() {
    this.contextMenuGeneration++;
  }

  /** Reset all state for reloading */
  resetState() {
    this.entries = [];
    this.glossary = [];
    this.characters = [];
    this.insertedInstructions = [];
    this.rowSpeakers = {};
    this.binPath = null;
    this.projectPath = null;
    this.sourceBinFilename = "";
    this.sysVersion = 4;
    this.sourceBinHash = "";
    this.isDirty = false;
    this.selectedRowIndex = null;
    this.scrollTop = 0;
    this.bridgeTemplateQuality = null;
    this.translationProgress = {
      status: "Idle",
      current_index: 0,
      total_entries: 0,
      total_input_tokens: 0,
      total_output_tokens: 0,
      last_error: null,
    };
  }

  constructor(id: string) {
    this.id = id;
  }
}

/**
 * Global application state using Svelte 5 runes.
 * Holds the tab array and shared state that spans all tabs.
 */
class AppStore {
  /** All open tabs */
  tabs = $state<TabState[]>([]);

  /** Index of the currently active tab (-1 = no tabs) */
  activeTabIndex = $state<number>(-1);

  /** Global glossary shared across all tabs */
  globalGlossary = $state<GlossaryEntry[]>([]);

  /** Whether an API key is configured */
  apiKeyConfigured = $state(false);

  /** Which API provider to use: "anthropic" or "openai-compatible" */
  apiProvider = $state<"anthropic" | "openai-compatible">("anthropic");

  /** Custom endpoint URL for OpenAI-compatible providers */
  customEndpoint = $state("");

  /** Last-used Anthropic model — used as the default when a new tab is created
   *  so the user doesn't have to re-pick after opening another BIN. */
  lastAnthropicModel = $state("claude-sonnet-4-6-20250627");

  /** Recently opened file paths (BINs or project JSONs). Most-recent first,
   *  capped at RECENT_FILES_MAX. Persisted in settings.json so the menu
   *  survives restarts. */
  recentFiles = $state<string[]>([]);

  /** When enabled, on app launch the editor reopens whatever tabs were open
   *  at the last close. Default off to keep startup fast for casual use. */
  autoRestoreSession = $state(false);

  /** Snapshot of open tab paths captured at app close (or whenever tabs
   *  change). Used by autoRestoreSession on next launch. */
  lastSessionPaths = $state<string[]>([]);

  /** When on, closing a tab always prompts for confirmation. When off, clean
   *  tabs close immediately; dirty (unsaved) tabs still always confirm so edits
   *  can't be lost silently. */
  confirmTabClose = $state(true);

  /** Last-used OpenAI-compatible model — same role for non-Anthropic providers.
   *  Empty until the user types one in Settings. */
  lastOpenAiCompatModel = $state("");

  /** Cache of fetched model lists from OpenAI-compatible /v1/models, keyed by
   *  normalized endpoint URL. Lets us show suggestions immediately when the
   *  user toggles back to a provider they've already loaded once this session. */
  openAiCompatModelCache = $state<Record<string, string[]>>({});

  /** Fetch state for the active OpenAI-compatible endpoint — drives the
   *  state-aware placeholder text in the Settings dialog. */
  openAiCompatFetchState = $state<"idle" | "loading" | "loaded" | "failed">("idle");
  openAiCompatFetchError = $state<string>("");

  /** Available Claude models — populated by `fetchAvailableModels()` once an
   *  Anthropic API key is configured. Empty by default so users without a key
   *  see the "Configure API Key" placeholder instead of stale/unsupported model
   *  options that won't work when selected. */
  availableModels = $state<ClaudeModel[]>([]);

  /** Soft character cap for translated text per text block. 0 = disabled. */
  translatedCharCap = $state(0);

  /** Soft character cap for individual translated text cells. 0 = disabled. */
  translatedLineCharCap = $state(0);

  /** Maximum display lines that fit in a single text box for this game.
   *  Used by auto-split to decide when to bridge into a new box.
   *  Default 3 (covers most Eushully VNs); override per-project as needed. */
  maxLinesPerBox = $state(3);

  /** Whether Claude should enforce character limits during translation. When off, Claude translates freely and flags lines that exceed limits. */
  enforceCharLimits = $state(true);

  /** When true, skip detailed notes and glossary_updates in LLM responses to reduce token usage. */
  minimalNotes = $state(false);

  /** When true, skip the previous-dialogue context window for faster, cheaper translations. Also disables cross-bin scan. */
  fastMode = $state(false);

  /** When true, skip including glossary entries in translation prompts to reduce token usage. */
  skipGlossary = $state(false);

  /** When true, scan the entry's JP text against glossary `japanese` fields
   *  and only send matching entries to the LLM. Reduces tokens vs sending the
   *  full glossary every call, while keeping name/term consistency. Ignored
   *  when `skipGlossary` is true. */
  scopedGlossary = $state(false);

  /** When true, group dialog rows into blocks (bounded by wait-for-input) and translate each block in one LLM call. */
  blockMode = $state(true);

  /** Number of previous dialog blocks to include as context. 0 = none. Forced to 0 in fast mode. */
  contextBlocks = $state(5);

  /** Whether Claude should scan sibling BIN files for narrative context before translating. */
  crossBinContext = $state(false);

  /** When true, translations that overflow character caps are automatically
   *  split into multiple dialog boxes using the BIN's bridge template. */
  autoSplitEnabled = $state(false);

  /** When true, importing translations (single or "import all") automatically
   *  runs the bulk auto-split pass on each affected tab, so overflowing rows
   *  are wrapped/box-split without manual review. Default off — opt-in per game
   *  (notably auto-wrap SYS4 titles like Sankai where boxes carry long lines). */
  autoSplitOnImport = $state(false);

  /** Last folder a Parente patch was exported to. Used as the save-dialog
   *  default so it doesn't open inside the game's admin-protected Archives
   *  folder (which the user usually can't write to without elevation). */
  lastParenteExportDir = $state<string | null>(null);

  /** Whether the characters panel is visible */
  charactersPanelOpen = $state(false);

  /** Whether the glossary panel is visible */
  glossaryPanelOpen = $state(false);

  /** Whether the global cross-tab search panel is visible */
  globalSearchPanelOpen = $state(false);

  /** Whether the cross-tab stats dashboard panel is visible */
  statsPanelOpen = $state(false);

  /** User-adjustable side panel widths (px). Persisted in settings.json.
   *  Each panel has a left-edge drag handle (ResizeHandle.svelte) that mutates
   *  the appropriate field. Defaults match the prior hard-coded widths. */
  panelWidths = $state<{ search: number; glossary: number; characters: number; stats: number }>({
    search: 420,
    glossary: 320,
    characters: 360,
    stats: 320,
  });

  /** Pending scroll-to-row request. When set, the active tab's EditorTable
   *  scrolls the row into view and briefly highlights it, then clears this
   *  signal. Used by the global search panel's click-to-jump behavior. */
  pendingScrollRequest = $state<{ tabId: string; rowIndex: number; nonce: number } | null>(null);

  /** Pending "scroll editor to top" request — set by double-clicking the
   *  active tab in the TabBar. EditorTable resets the active tab's scroll to 0
   *  when this fires for the active tab. Nonce makes repeat double-clicks fire. */
  scrollToTopRequest = $state<{ tabId: string; nonce: number } | null>(null);

  /** Whether the omitted-cells dialog is open (lists omit_export rows + rescan). */
  omittedCellsDialogOpen = $state(false);

  /** Persisted Export Review view state — scope/filter + scroll offset — so the
   *  dialog resumes exactly where the user left off when reopened. */
  exportReview = $state<{
    scope: "all" | "tab";
    filter: "needs" | "omitted" | "included" | "all";
    scrollTop: number;
  }>({ scope: "all", filter: "needs", scrollTop: 0 });

  /** Pending Parente scene picker. Set after listing a single .arc or a whole
   *  game folder; the dialog lets the user choose which scenes to load as tabs.
   *  `gameDir` is set in game-folder mode so export can target the Patch folder. */
  parenteArcRequest = $state<{
    title: string;
    files: import("./ipc").ParenteGameFile[];
    gameDir?: string;
  } | null>(null);

  /** Pending "jump to instruction in BIN Viewer" request — set by EditorRow's
   *  context menu, consumed by BinViewerDialog after it loads/becomes visible.
   *  Nonce ensures repeat clicks on the same instruction re-fire the jump. */
  pendingBinViewerJump = $state<{ tabId: string; instructionIndex: number; nonce: number } | null>(null);

  /** Toast notifications — surfaced for previously-silent error paths so
   *  failures (font load, settings save, sibling-folder lookups) become
   *  visible without breaking the foreground task. Newest first. */
  toasts = $state<Array<{
    id: number;
    kind: "error" | "warn" | "info";
    message: string;
    /** Auto-dismiss after this many ms; 0 = sticky until clicked. */
    timeoutMs: number;
  }>>([]);

  /** Pending export diff dialog. Set by doExportBin before opening the save
   *  dialog so the user sees what's about to change. Cleared on cancel or
   *  after the actual export starts. */
  pendingExportDiff = $state<{
    tabId: string;
    binFilename: string;
    diff: import("./types").ExportDiff;
    onConfirm: () => void;
    onCancel: () => void;
  } | null>(null);

  /** Generic themed confirmation request. Set via `confirmDialog()` in
   *  lib/confirm.ts; rendered by ConfirmDialog. Replaces OS-native `ask()`
   *  popups so confirmations match the app theme. */
  confirmRequest = $state<{
    title: string;
    message: string;
    confirmLabel: string;
    cancelLabel: string;
    danger: boolean;
    resolve: (confirmed: boolean) => void;
  } | null>(null);

  /** Whether the settings dialog is open */
  settingsDialogOpen = $state(false);

  /** Whether the BIN viewer dialog is open */
  binViewerOpen = $state(false);

  /** BIN viewer display mode: "scroll" for single continuous list, "paged" for paginated */
  binViewerMode = $state<"scroll" | "paged">("scroll");

  /** Status bar message */
  statusMessage = $state("Ready");

  /** Timestamp of last auto-save, null if never saved */
  lastSaveTime = $state<string | null>(null);

  /** Pending glossary review items from Claude suggestions */
  glossaryReviewQueue = $state<GlossaryReviewItem[]>([]);

  /** Whether the glossary review dialog is open */
  glossaryReviewDialogOpen = $state(false);

  /** Loading progress for batch file operations */
  loadingProgress = $state<{ current: number; total: number; currentFile: string } | null>(null);

  /** Pending import-mode dialog. When set, App.svelte renders the dialog and
   *  the resolver is called when the user picks a mode or cancels. */
  importModeRequest = $state<{
    resolve: (mode: "fill_missing" | "overwrite" | null) => void;
  } | null>(null);

  /** Whether the overflow review dialog is open */
  overflowReviewDialogOpen = $state(false);

  /** Whether the glossary-consistency review dialog is open */
  glossaryConsistencyOpen = $state(false);

  /** Whether the glossary auto-apply review dialog is open */
  glossaryAutoApplyOpen = $state(false);

  /** One-level undo for bulk operations (replace-all, split/paginate across
   *  tabs). Snapshots the affected tabs before the op; a floating Undo pill
   *  restores it. Replaced by the next bulk op or cleared on use. */
  bulkUndo = $state<{
    label: string;
    snapshots: { tabId: string; entries: DialogueEntry[]; insertedInstructions: InsertedInstruction[]; isDirty: boolean }[];
  } | null>(null);

  /** Translation project status snapshot to render in the status dialog.
   *  Set by doCheckTranslationStatus; App.svelte mounts the dialog when present. */
  translationStatusReport = $state<{
    parentDir: string;
    folders: import("./ipc").TranslationFolderStatus[];
  } | null>(null);

  /** Pending model-picker dialog request. The CC export actions await this
   *  to let the user choose which Claude model goes into the launcher scripts,
   *  plus optional parallel-batch settings. */
  modelPickerRequest = $state<{
    defaultModel: string;
    title: string;
    showParallel: boolean;
    resolve: (opts: import("../components/ModelPickerDialog.svelte").ExportOptions | null) => void;
  } | null>(null);

  /** The currently active tab, or undefined if no tabs */
  get activeTab(): TabState | undefined {
    if (this.activeTabIndex >= 0 && this.activeTabIndex < this.tabs.length) {
      return this.tabs[this.activeTabIndex];
    }
    return undefined;
  }

  /**
   * Merged glossary: global + active tab-local.
   * Tab-local entries win on japanese key conflict.
   */
  get mergedGlossary(): GlossaryEntry[] {
    const tab = this.activeTab;
    if (!tab) return [...this.globalGlossary];

    const merged = [...this.globalGlossary];
    for (const local of tab.glossary) {
      const existingIdx = merged.findIndex((g) => g.japanese === local.japanese);
      if (existingIdx !== -1) {
        // Tab-local overrides global
        merged[existingIdx] = local;
      } else {
        merged.push(local);
      }
    }
    return merged;
  }

  /** Glossary to send to the LLM — empty if skipGlossary is on. */
  get promptGlossary(): GlossaryEntry[] {
    return this.skipGlossary ? [] : this.mergedGlossary;
  }

  /** Glossary scoped to a specific JP text (entry or concatenated block).
   *  - skipGlossary on → []
   *  - scopedGlossary on → only entries whose `japanese` is a substring of `text`
   *  - both off → full merged glossary
   *  Keeps per-call tokens minimal while preserving name/term consistency. */
  promptGlossaryFor(text: string): GlossaryEntry[] {
    if (this.skipGlossary) return [];
    if (!this.scopedGlossary) return this.mergedGlossary;
    return this.mergedGlossary.filter(g => g.japanese && text.includes(g.japanese));
  }

  /** Create a new tab and make it active. Returns the new TabState. */
  createTab(): TabState {
    const id = crypto.randomUUID();
    const tab = new TabState(id);
    // Seed per-provider model defaults so the user's last choice carries over
    // to new tabs / new sessions instead of resetting to the hardcoded default.
    if (this.lastAnthropicModel) tab.anthropicModel = this.lastAnthropicModel;
    if (this.lastOpenAiCompatModel) tab.openAiCompatModel = this.lastOpenAiCompatModel;
    this.tabs = [...this.tabs, tab];
    this.activeTabIndex = this.tabs.length - 1;
    return tab;
  }

  /** Close a tab by index. Adjusts activeTabIndex accordingly. */
  closeTab(index: number) {
    if (index < 0 || index >= this.tabs.length) return;

    this.tabs = this.tabs.filter((_, i) => i !== index);

    if (this.tabs.length === 0) {
      this.activeTabIndex = -1;
    } else if (this.activeTabIndex >= this.tabs.length) {
      this.activeTabIndex = this.tabs.length - 1;
    } else if (this.activeTabIndex > index) {
      this.activeTabIndex--;
    }
    // If activeTabIndex === index and there are still tabs, it now points to the next tab (or last)
  }

  /** Switch to a different tab by index. */
  switchTab(index: number) {
    if (index >= 0 && index < this.tabs.length) {
      this.activeTabIndex = index;
    }
  }

  /** Snapshot the given tabs' editable state before a bulk operation, enabling
   *  one-level undo via `restoreBulkUndo`. Call this at the START of any bulk op
   *  that mutates many rows (replace-all, split/paginate). */
  captureBulkUndo(label: string, tabs: TabState[]) {
    this.bulkUndo = {
      label,
      snapshots: tabs.map((t) => ({
        tabId: t.id,
        entries: $state.snapshot(t.entries) as DialogueEntry[],
        insertedInstructions: $state.snapshot(t.insertedInstructions) as InsertedInstruction[],
        isDirty: t.isDirty,
      })),
    };
  }

  /** Restore the last bulk snapshot. Returns the op label (for a toast), or null. */
  restoreBulkUndo(): string | null {
    if (!this.bulkUndo) return null;
    const { label, snapshots } = this.bulkUndo;
    for (const s of snapshots) {
      const tab = this.tabs.find((t) => t.id === s.tabId);
      if (!tab) continue;
      tab.entries = structuredClone(s.entries);
      tab.insertedInstructions = structuredClone(s.insertedInstructions);
      tab.isDirty = s.isDirty;
    }
    this.bulkUndo = null;
    return label;
  }

  clearBulkUndo() {
    this.bulkUndo = null;
  }

  /** Move a tab from one index to another, preserving the active tab's
   *  identity (so the active highlight follows the dragged tab even when
   *  the active index shifts due to the reorder). Used by drag-and-drop
   *  reordering in TabBar. */
  reorderTab(fromIndex: number, toIndex: number) {
    if (fromIndex === toIndex) return;
    if (fromIndex < 0 || fromIndex >= this.tabs.length) return;
    if (toIndex < 0 || toIndex > this.tabs.length) return;
    const activeId = this.tabs[this.activeTabIndex]?.id;
    const moved = this.tabs[fromIndex];
    // Splice out, then insert. Adjust target index when removal shifts indices.
    const next = this.tabs.slice();
    next.splice(fromIndex, 1);
    const adjustedTo = toIndex > fromIndex ? toIndex - 1 : toIndex;
    next.splice(adjustedTo, 0, moved);
    this.tabs = next;
    // Restore active highlight to the same tab object.
    if (activeId) {
      const newIdx = this.tabs.findIndex((t) => t.id === activeId);
      if (newIdx >= 0) this.activeTabIndex = newIdx;
    }
  }
}

export const store = new AppStore();
