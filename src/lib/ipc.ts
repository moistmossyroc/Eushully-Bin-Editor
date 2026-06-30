import { invoke } from "@tauri-apps/api/core";
import type {
  CharacterEntry,
  DialogueEntry,
  GlossaryEntry,
  InsertedInstruction,
  LoadBinResult,
  ProjectFile,
  TranslationResult,
} from "./types";
import { resolveInsertionsForExport } from "./insertions";

/** Load a BIN file and return extracted dialogue entries + metadata. */
export function loadBin(tabId: string, path: string): Promise<LoadBinResult> {
  return invoke<LoadBinResult>("load_bin", { tabId, path });
}

export interface EngineInfo {
  sys_version: number;
  engine_version: number;
  engine_label: string;
}

/** Engine/build info for an already-loaded tab (project-restore path). */
export function getEngineInfo(tabId: string): Promise<EngineInfo> {
  return invoke<EngineInfo>("get_engine_info", { tabId });
}

/** Load a project JSON file. */
export function loadProject(path: string): Promise<ProjectFile> {
  return invoke<ProjectFile>("load_project", { path });
}

// --- Parente (Unity-era Eushully) .arc support ---

/** One effective scene/text file, resolved from one .arc or across a whole
 *  game folder. Carries its source archive + byte location for load/export. */
export interface ParenteGameFile {
  arc_path: string;
  /** Short label of the source archive (e.g. "Patch/ver103_x1.arc"). */
  source: string;
  name: string;
  path: string;
  offset: number;
  size: number;
  /** Number of translatable (Japanese-containing) strings in the scene. */
  string_count: number;
  /** Heuristic flag: scene contains adult content (explicit-vocab scan). */
  adult: boolean;
}

/** Editor rows + per-row speakers for one loaded Parente scene. */
export interface ParenteEntryLoad {
  name: string;
  path: string;
  entries: DialogueEntry[];
  /** [ordinal, speaker-name] pairs. */
  speakers: [number, string][];
}

/** List the translatable scene scripts inside a single Parente .arc. */
export function parenteListArc(path: string): Promise<ParenteGameFile[]> {
  return invoke<ParenteGameFile[]>("parente_list_arc", { path });
}

/** Resolve the effective (latest) dialogue scenes across a whole game folder. */
export function parenteListGame(dir: string): Promise<ParenteGameFile[]> {
  return invoke<ParenteGameFile[]>("parente_list_game", { dir });
}

/** Decode one scene (by byte location in its source arc) into editor rows. */
export function parenteLoadGameEntry(f: {
  arc_path: string;
  name: string;
  path: string;
  offset: number;
  size: number;
}): Promise<ParenteEntryLoad> {
  return invoke<ParenteEntryLoad>("parente_load_game_entry", {
    arcPath: f.arc_path,
    name: f.name,
    path: f.path,
    offset: f.offset,
    size: f.size,
  });
}

/** One translated scene to include in an exported patch. */
export interface ParentePatchFile {
  arc_path: string;
  name: string;
  path: string;
  offset: number;
  size: number;
  translations: [number, string][];
}

/** Build a single ver999-style patch archive containing only the translated
 *  scenes. Returns the number of files written. */
export function parenteExportPatch(files: ParentePatchFile[], outPath: string): Promise<number> {
  return invoke<number>("parente_export_patch", { files, outPath });
}

/** Build a FULL replacement patch: rebuild `primaryArc` (the current official
 *  patch) with all edits applied + any base-only edited scenes appended.
 *  Returns the number of scenes changed. */
export function parenteExportFull(primaryArc: string, files: ParentePatchFile[], outPath: string): Promise<number> {
  return invoke<number>("parente_export_full", { primaryArc, files, outPath });
}

/** Build a NON-DESTRUCTIVE English patch: English goes in slot 2 (Japanese
 *  preserved in slot 1), with `text`/`LangArray` patched so the game renders
 *  English where present and falls back to Japanese otherwise. Returns scenes
 *  changed. */
export function parenteExportEnglish(primaryArc: string, files: ParentePatchFile[], outPath: string): Promise<number> {
  return invoke<number>("parente_export_english", { primaryArc, files, outPath });
}

/** Where a patch should be written for a dropped game folder, if detectable. */
export function parenteSuggestedPatchPath(dir: string): Promise<string | null> {
  return invoke<string | null>("parente_suggested_patch_path", { dir });
}

/** Save a Parente scene's translation project (keyed per scene) so progress
 *  survives closing/reopening. */
export function parenteSaveProject(key: string, content: string): Promise<void> {
  return invoke<void>("parente_save_project", { key, content });
}

/** Load a saved Parente scene project, or null if none exists. */
export function parenteLoadProject(key: string): Promise<string | null> {
  return invoke<string | null>("parente_load_project", { key });
}

/** Load the BIN file for an existing project, verifying its hash. */
export function loadBinForProject(
  tabId: string,
  binPath: string,
  expectedHash: string
): Promise<boolean> {
  return invoke<boolean>("load_bin_for_project", {
    tabId,
    binPath,
    expectedHash,
  });
}

/** Save the current project state to a JSON file. */
export function saveProject(
  tabId: string,
  path: string,
  entries: DialogueEntry[],
  glossary: GlossaryEntry[],
  characters: CharacterEntry[],
  insertedInstructions: InsertedInstruction[],
  model: string
): Promise<void> {
  return invoke("save_project", {
    tabId,
    path,
    entries,
    glossary,
    characters,
    insertedInstructions,
    model,
  });
}

/**
 * Export a modified BIN file with applied translations.
 *
 * IMPORTANT: `entries` MUST be the FULL entries array including negative-index
 * (synthetic) continuation rows. resolveInsertionsForExport derives insertion
 * order (_visualOrder) and continuation text from those rows; passing only
 * positive-index entries silently reverses inserted chains on export.
 */
export function exportBin(
  tabId: string,
  outputPath: string,
  entries: DialogueEntry[],
  insertedInstructions: InsertedInstruction[]
): Promise<void> {
  return invoke("export_bin", {
    tabId,
    outputPath,
    entries,
    insertedInstructions: resolveInsertionsForExport(insertedInstructions, entries),
  });
}

/** One BIN to pack into a .fanpatch: which tab + the name the game requests it by. */
export interface FanpatchItem {
  tabId: string;
  filename: string;
  entries: DialogueEntry[];
  insertedInstructions: InsertedInstruction[];
}

export interface FanpatchResult {
  written: number;
  /** "<NAME>: <reason>" per skipped BIN (corruption guard / duplicate). */
  skipped: string[];
  /** True if version.dll (the drop-in loader) was written next to the patch. */
  loader_written: boolean;
}

/**
 * Export a `.fanpatch` (a ZIP of translated BINs, flat + request-named) for the
 * version.dll fanpatch loader. Each item's insertions are resolved for export
 * exactly like {@link exportBin}, so ordering/continuation text stay correct.
 */
export function exportFanpatch(
  items: FanpatchItem[],
  outputPath: string,
  title: string,
  label: string,
  uiConfigPath: string
): Promise<FanpatchResult> {
  return invoke("export_fanpatch", {
    items: items.map((it) => ({
      tabId: it.tabId,
      filename: it.filename,
      entries: it.entries,
      insertedInstructions: resolveInsertionsForExport(it.insertedInstructions, it.entries),
    })),
    outputPath,
    title,
    label,
    uiConfigPath,
  });
}

/**
 * Package every file in `sourceDir` (flat) into a `.fanpatch`. `.BIN` files are
 * validated; other assets pass through. Use to zip an already-translated folder.
 */
export function packFanpatchFromDir(
  sourceDir: string,
  outputPath: string,
  title: string,
  label: string,
  uiConfigPath: string
): Promise<FanpatchResult> {
  return invoke("pack_fanpatch_from_dir", { sourceDir, outputPath, title, label, uiConfigPath });
}

/**
 * Build a `.fanpatch` from a folder of translated (English) BINs against their
 * Japanese originals in `jpDir`. `bilingual=false` = narrator-box repair only;
 * `bilingual=true` = repair + in-game mid-scene EN↔JP switching. BINs that can't
 * be paired/aligned ship unchanged (reported in `skipped`).
 */
export function packDualLanguageFanpatch(
  enDir: string,
  jpDir: string,
  agfDir: string,
  bilingual: boolean,
  outputPath: string,
  title: string,
  label: string,
  uiConfigPath: string
): Promise<FanpatchResult> {
  return invoke("pack_dual_language_fanpatch", {
    enDir,
    jpDir,
    agfDir,
    bilingual,
    outputPath,
    title,
    label,
    uiConfigPath,
  });
}

/** Set the Claude API key. */
export function setApiKey(key: string): Promise<void> {
  return invoke("set_api_key", { key });
}

/** Get the stored API key. */
export function getApiKey(): Promise<string | null> {
  return invoke<string | null>("get_api_key");
}

/** Set the API provider configuration. */
export function setProviderConfig(provider: string, endpoint: string | null): Promise<void> {
  return invoke("set_provider_config", { provider, endpoint });
}

/** Get the current API provider configuration. */
export function getProviderConfig(): Promise<{ provider: string; endpoint: string | null }> {
  return invoke<{ provider: string; endpoint: string | null }>("get_provider_config");
}

/** Fetch the model ID list from an OpenAI-compatible provider's /v1/models.
 *  Returns ids on success, throws (rejection) on any error so the caller can
 *  fall back to the manual-entry placeholder. */
export function fetchOpenAiCompatModels(
  endpoint: string,
  apiKey: string,
): Promise<string[]> {
  return invoke<string[]>("fetch_openai_compat_models", { endpoint, apiKey });
}

/** Translation options passed to Claude. */
export interface TranslationIpcOptions {
  blockCharCap: number;
  lineCharCap: number;
  enforceCharLimits: boolean;
  crossBinContext: string | null;
  minimalNotes: boolean;
  fastMode: boolean;
  blockMode: boolean;
  contextBlocks: number;
  scopedGlossary: boolean;
}

/** Translate a single dialogue entry. */
export function translateSingleEntry(
  entry: DialogueEntry,
  contextEntries: DialogueEntry[],
  glossary: GlossaryEntry[],
  model: string,
  options: TranslationIpcOptions
): Promise<TranslationResult> {
  return invoke<TranslationResult>("translate_single_entry", {
    entry,
    contextEntries,
    glossary,
    model,
    blockCharCap: options.blockCharCap ?? 0,
    lineCharCap: options.lineCharCap ?? 0,
    enforceCharLimits: options.enforceCharLimits ?? true,
    crossBinContext: options.crossBinContext,
    minimalNotes: options.minimalNotes ?? false,
    fastMode: options.fastMode ?? false,
    blockMode: options.blockMode ?? true,
    contextBlocks: options.contextBlocks ?? 5,
    scopedGlossary: options.scopedGlossary ?? false,
  });
}

/** Result of a proofread call */
export interface ProofreadResult {
  translations: string[];
  notes: string;
}

/** Proofread an entire dialog block in one call. */
export function proofreadBlock(
  entries: DialogueEntry[],
  glossary: GlossaryEntry[],
  model: string,
  lineCharCap: number,
  blockCharCap: number,
  enforceCharLimits: boolean,
): Promise<ProofreadResult> {
  return invoke<ProofreadResult>("proofread_block", {
    entries,
    glossary,
    model,
    lineCharCap,
    blockCharCap,
    enforceCharLimits,
  });
}

/** Optimize a single translated entry to fit within character limits. */
export function optimizeEntry(
  entry: DialogueEntry,
  contextEntries: DialogueEntry[],
  glossary: GlossaryEntry[],
  model: string,
  lineCharCap: number,
  blockCharCap: number,
  currentLineLength: number,
  currentBlockLength: number,
  minimalNotes: boolean = false,
): Promise<TranslationResult> {
  return invoke<TranslationResult>("optimize_entry", {
    entry,
    contextEntries,
    glossary,
    model,
    lineCharCap,
    blockCharCap,
    currentLineLength,
    currentBlockLength,
    minimalNotes,
  });
}

/** Start batch translation for a tab. */
export function startTranslation(
  tabId: string,
  entries: DialogueEntry[],
  glossary: GlossaryEntry[],
  model: string,
  options: TranslationIpcOptions
): Promise<void> {
  return invoke("start_translation", {
    tabId,
    entries,
    glossary,
    model,
    blockCharCap: options.blockCharCap ?? 0,
    lineCharCap: options.lineCharCap ?? 0,
    enforceCharLimits: options.enforceCharLimits ?? true,
    crossBinContext: options.crossBinContext,
    minimalNotes: options.minimalNotes ?? false,
    fastMode: options.fastMode ?? false,
    blockMode: options.blockMode ?? true,
    contextBlocks: options.contextBlocks ?? 5,
    scopedGlossary: options.scopedGlossary ?? false,
  });
}

/** Pause ongoing translation for a tab. */
export function pauseTranslation(tabId: string): Promise<void> {
  return invoke("pause_translation", { tabId });
}

/** Resume paused translation for a tab. */
export function resumeTranslation(tabId: string): Promise<void> {
  return invoke("resume_translation", { tabId });
}

/** Stop ongoing translation for a tab. */
export function stopTranslation(tabId: string): Promise<void> {
  return invoke("stop_translation", { tabId });
}

/** Extract all referenced speaker pointers from the loaded BIN. */
export function getReferencedSpeakers(tabId: string): Promise<CharacterEntry[]> {
  return invoke<CharacterEntry[]>("get_referenced_speakers", { tabId });
}

/** Scan a BIN file for character definitions (set-string pairs). */
export function scanBinForCharacters(
  path: string,
): Promise<CharacterEntry[]> {
  return invoke<CharacterEntry[]>("scan_bin_for_characters", { path });
}

/** Speaker reference for a single row */
export interface SpeakerRef {
  source_type: number;
  source_raw_data: number;
  literal_name: string | null;
}

/** Get speaker references for each dialogue row from the loaded BIN. */
export function getRowSpeakers(tabId: string): Promise<Record<string, SpeakerRef>> {
  return invoke<Record<string, SpeakerRef>>("get_row_speakers", { tabId });
}

/** Instruction argument from the BIN file */
export interface BinArgument {
  arg_type: number;
  raw_data: number;
  decoded_string?: string;
}

/** A single parsed instruction from the BIN file */
export interface BinInstruction {
  opcode: number;
  label: string;
  argument_count: number;
  arguments: BinArgument[];
  offset: number;
}

/** Get all instructions from the loaded BIN file. */
export function getAllInstructions(tabId: string): Promise<BinInstruction[]> {
  return invoke<BinInstruction[]>("get_all_instructions", { tabId });
}

/** Compute SHA-256 hash of a file. */
export function computeFileHash(path: string): Promise<string> {
  return invoke<string>("compute_file_hash", { path });
}

/** Close a tab and free its Rust-side state. */
export function closeTab(tabId: string): Promise<void> {
  return invoke("close_tab", { tabId });
}

/** Export a glossary to a JSON file. */
export function exportGlossary(entries: GlossaryEntry[], path: string): Promise<void> {
  return invoke("export_glossary", { entries, path });
}

/** Import a glossary from a JSON file. */
export function importGlossary(path: string): Promise<GlossaryEntry[]> {
  return invoke<GlossaryEntry[]>("import_glossary", { path });
}

/** Scan sibling BIN files for cross-file narrative context. */
export function scanCrossBinContext(binPath: string, maxChars: number = 30000): Promise<string> {
  return invoke<string>("scan_cross_bin_context", { binPath, maxChars });
}

/** Fetch available Claude models from the Anthropic API. */
export function fetchAvailableModels(): Promise<{ id: string; label: string; created_at: string }[]> {
  return invoke<{ id: string; label: string; created_at: string }[]>("fetch_available_models");
}

/** Compute a split plan for an overflowing entry. */
export function autoSplitEntry(
  tabId: string,
  translatedText: string,
  entryIndex: number,
  lineCharCap: number,
  blockCharCap: number,
  maxLinesPerBox: number,
): Promise<import("./types").SplitPlan> {
  return invoke<import("./types").SplitPlan>("auto_split_entry", {
    tabId,
    translatedText,
    entryIndex,
    lineCharCap,
    blockCharCap,
    maxLinesPerBox,
  });
}

export interface SplitRequest {
  translated_text: string;
  entry_index: number;
}

export interface SplitOutcome {
  entry_index: number;
  plan: import("./types").SplitPlan | null;
  error: string | null;
}

/** Compute split plans for many overflowing entries in a single round-trip. */
export function autoSplitEntriesBatch(
  tabId: string,
  items: SplitRequest[],
  lineCharCap: number,
): Promise<SplitOutcome[]> {
  return invoke<SplitOutcome[]>("auto_split_entries_batch", { tabId, items, lineCharCap });
}

/** Get the bridge template quality for a tab's loaded BIN. */
export function getBridgeTemplateStatus(tabId: string): Promise<string> {
  return invoke<string>("get_bridge_template_status", { tabId });
}

/** Fetch a cloned bridge instruction sequence for use in frontend-side
 *  group splits that need to cross a textbox boundary. */
export function getBridgeClone(tabId: string): Promise<import("./types").SerializedInstruction[]> {
  return invoke<import("./types").SerializedInstruction[]>("get_bridge_clone", { tabId });
}

/** A term to send to the Claude CLI for glossary translation. */
export interface GlossaryCliEntry { term: string; examples: string[]; suggestion: string }
/** A translated term read back from the CLI's glossary-out.json. */
export interface GlossaryCliResult { term: string; translation: string; notes: string }

/** Export selected glossary terms to a self-contained `glossary-cli/` folder
 *  (glossary.json + PROMPT.md + interactive `claude` launcher) and optionally
 *  launch it. Returns the created folder path. */
export function exportGlossaryCli(outDir: string, entries: GlossaryCliEntry[], launch: boolean): Promise<string> {
  return invoke<string>("export_glossary_cli", { outDir, entries, launch });
}

/** Poll the CLI's glossary-out.json in `folder`. Returns null while missing /
 *  not-yet-valid (half-written), or the parsed results when ready. */
export function readGlossaryCliResults(folder: string): Promise<GlossaryCliResult[] | null> {
  return invoke<GlossaryCliResult[] | null>("read_glossary_cli_results", { folder });
}

/** Scan a set of BIN files for `set-string [pointer] = "name"` writes matching
 *  the requested pointers. Returns a flat map. Used to resolve unknown character
 *  names by direct lookup across CVINIT/CIINIT/CNINIT and the scene BIN. */
export function lookupCharacterNamesForPointers(
  binPaths: string[],
  pointers: number[]
): Promise<Record<string, string>> {
  return invoke<Record<string, string>>("lookup_character_names_for_pointers", {
    binPaths,
    pointers,
  });
}

/** A single candidate name from an init BIN, surfaced in the manual name-picker. */
export interface InitNameCandidate {
  source_file: string;
  address: number;
  name: string;
}

/** List every `set-string` name defined in the given init BIN paths.
 *  Used by the manual character-name picker when the auto-resolver can't
 *  resolve a scene-local lookup-array ID. */
export function listInitBinNames(binPaths: string[]): Promise<InitNameCandidate[]> {
  return invoke<InitNameCandidate[]>("list_init_bin_names", { binPaths });
}

/** Open a folder in the OS-native file explorer. */
export function openFolderInExplorer(folderPath: string): Promise<void> {
  return invoke<void>("open_folder_in_explorer", { folderPath });
}

/** An AGE script BIN found by scanning a game folder, classified for the picker. */
export interface SysBinFile {
  path: string;
  filename: string;
  /** Filename with the `$N$` update-layer prefix stripped. */
  base_name: string;
  /** Eushully update layer ($1$/$2$…); 0 = unprefixed. */
  layer: number;
  sys_version: number;
  /** Picker bucket: "names" (character/item name tables) | "sc" (story scenes)
   *  | "sp" (special scenes) | "ui" (menu/system text) | "other" (everything
   *  else). */
  category: string;
  /** Count of translatable strings (0 ⇒ data BIN; the picker hides these). */
  string_count: number;
  /** Dialogue is mostly Latin → likely already-translated / not original. */
  likely_translated: boolean;
  size: number;
  /** Present only for entries packed in a .alf archive (extract on open). */
  alf_archive_index?: number;
  alf_offset?: number;
  alf_length?: number;
}

/** Scan a folder for AGE script .BIN files (SYS3/4/5) — loose files plus the
 *  base BINs packed in `.alf` archives (loose overrides archived). `spPrefixes`
 *  are the filename prefixes that mark special scenes (empty → defaults to SP). */
export function scanBinFolder(folderPath: string, spPrefixes: string[]): Promise<SysBinFile[]> {
  return invoke<SysBinFile[]>("scan_bin_folder", { folderPath, spPrefixes });
}

/** One packed entry to extract. */
export interface AlfRef {
  archive_index: number;
  offset: number;
  length: number;
  name: string;
}

/** Extract packed .alf entries from a game folder into dest as real .BIN files. */
export function extractAlfToFolder(gameDir: string, destFolder: string, entries: AlfRef[]): Promise<string[]> {
  return invoke<string[]>("extract_alf_to_folder", { gameDir, destFolder, entries });
}

/** Whether a path is a directory (backend check — not fs-plugin-scoped). */
export function pathIsDir(path: string): Promise<boolean> {
  return invoke<boolean>("path_is_dir", { path });
}

/** Probe whether a folder can be written to (detects admin-locked game folders). */
export function isFolderWritable(folderPath: string): Promise<boolean> {
  return invoke<boolean>("is_folder_writable", { folderPath });
}

/** Copy files into a destination folder (created if needed); returns new paths. */
export function copyFilesToFolder(sources: string[], destFolder: string): Promise<string[]> {
  return invoke<string[]>("copy_files_to_folder", { sources, destFolder });
}

export interface ProjectBackupInfo {
  path: string;
  depth: number;
  modified_ms: number;
  size_bytes: number;
  entry_count?: number | null;
}

export interface BackupDiffRow {
  instruction_index: number;
  label: string;
  original_text: string;
  current_translated: string;
  backup_translated: string;
  omit_changed: boolean;
}

export interface BackupDiff {
  total_current: number;
  total_backup: number;
  translated_diff_count: number;
  omit_diff_count: number;
  rows: BackupDiffRow[];
}

export function listProjectBackups(projectPath: string): Promise<ProjectBackupInfo[]> {
  return invoke<ProjectBackupInfo[]>("list_project_backups", { projectPath });
}

export function diffProjectBackup(currentPath: string, backupPath: string): Promise<BackupDiff> {
  return invoke<BackupDiff>("diff_project_backup", { currentPath, backupPath });
}

export function restoreProjectBackup(currentPath: string, backupPath: string): Promise<void> {
  return invoke<void>("restore_project_backup", { currentPath, backupPath });
}

/** Read a prior translation.json in `folderPath` and return its
 *  `original → translated` map for re-export delta mode. Empty map on
 *  missing/invalid file. */
export function readPriorTranslationMap(folderPath: string): Promise<Record<string, string>> {
  return invoke<Record<string, string>>("read_prior_translation_map", { folderPath });
}

/** Read a project-folder-level glossary from `<binDir>/glossary.json`. */
export function loadFolderGlossary(binPath: string): Promise<import("./types").GlossaryEntry[]> {
  return invoke<import("./types").GlossaryEntry[]>("load_folder_glossary", { binPath });
}

/** Write a project-folder-level glossary to `<binDir>/glossary.json`. */
export function saveFolderGlossary(binPath: string, entries: import("./types").GlossaryEntry[]): Promise<void> {
  return invoke<void>("save_folder_glossary", { binPath, entries });
}

/** Re-launch a single translation subfolder's translate.bat/.sh in a new terminal. */
export function relaunchTranslationFolder(folderPath: string): Promise<void> {
  return invoke<void>("relaunch_translation_folder", { folderPath });
}

/** Build a `character_id → name` map for a CHINIT-style init BIN by detecting
 *  its consecutive-register character table. Keys are decimal stringified IDs
 *  (serde quirk for integer-keyed maps). */
export function extractChinitIdNameMap(binPath: string): Promise<Record<string, string>> {
  return invoke<Record<string, string>>("extract_chinit_id_name_map", { binPath });
}

/** Per-folder translation progress entry from scan_translation_project_status. */
export interface TranslationFolderStatus {
  folder: string;
  source_bin: string;
  source_bin_sha256: string;
  total: number;
  translated: number;
  error?: string;
}

/** Scan a parent folder for translation.json subfolders. Returns per-folder progress. */
export function scanTranslationProjectStatus(parentDir: string): Promise<TranslationFolderStatus[]> {
  return invoke<TranslationFolderStatus[]>("scan_translation_project_status", { parentDir });
}

/** Write translate-all.bat / translate-all.sh (always) + translate-parallel.* (if parallel)
 *  at a parent folder. */
export function writeMultiTranslateLaunchers(
  parentDir: string,
  subfolderNames: string[],
  claudeModel: string | null,
  parallel: boolean,
  parallelLimit: number,
): Promise<void> {
  return invoke<void>("write_multi_translate_launchers", {
    parentDir, subfolderNames, claudeModel, parallel, parallelLimit,
  });
}

/** Load persisted editor settings. */
export function loadSettings(): Promise<Record<string, unknown>> {
  return invoke<Record<string, unknown>>("load_settings");
}

/** Save editor settings. */
export function saveSettings(settings: Record<string, unknown>): Promise<void> {
  return invoke<void>("save_settings", { settings });
}

/** Check if a file exists at the given path. */
export function fileExists(path: string): Promise<boolean> {
  return invoke<boolean>("file_exists", { path });
}

/** Ensure a directory exists, creating it and parents if needed. */
export function ensureDirectory(path: string): Promise<void> {
  return invoke<void>("ensure_directory", { path });
}

/** A row in the offline translation export. Matches Rust TranslationRow.
 *  - group_member_ids: multi-entry line group anchor (concatenated JP)
 *  - block_id: dialogue block grouping (bounded by wait-for-input)
 *  - duplicate_of: this row is a dup of the canonical row at the given id;
 *    only translate the canonical and let import distribute. */
export interface ExternalDuplicateRef {
  /** Subfolder basename containing the canonical row. */
  folder: string;
  /** `id` of the canonical row inside that folder's translation.json. */
  id: number;
}

export interface TranslationRow {
  id: number;
  label: string;
  speaker?: string;
  original: string;
  translated: string;
  notes: string;
  group_member_ids?: number[];
  block_id?: number;
  duplicate_of?: number;
  context_before?: string[];
  /** Cross-folder duplicate marker — set by multi-tab export when the same
   *  Japanese appears in multiple sibling folders. Points back at the
   *  canonical row; `translate.py` skips these (no Claude tokens spent). */
  external_duplicate_of?: ExternalDuplicateRef;
}

/** Result of importing translations back from an edited JSON file. */
export interface ImportMergeResult {
  entries: DialogueEntry[];
  applied: number;
  unknown_ids: number;
  unchanged: number;
  skipped_existing: number;
  hash_mismatch: boolean;
  source_bin: string;
}

/** Export the current tab's dialogue as translation.json + PROMPT.md for offline Claude Code translation. */
/** Split capability for a tab's BIN — sizes the CC-translate budget per file. */
export interface SplitCapability {
  can_add_boxes: boolean;
  can_break_lines: boolean;
}

export function getSplitCapability(tabId: string): Promise<SplitCapability> {
  return invoke<SplitCapability>("get_split_capability", { tabId });
}

export function exportForTranslation(
  outputDir: string,
  sourceBinFilename: string,
  sourceBinSha256: string,
  wrapWidth: number,
  maxLinesPerBox: number,
  canAddBoxes: boolean,
  canBreakLines: boolean,
  glossary: GlossaryEntry[],
  entries: TranslationRow[],
  claudeModel: string | null
): Promise<string> {
  return invoke<string>("export_for_translation", {
    outputDir,
    sourceBinFilename,
    sourceBinSha256,
    wrapWidth,
    maxLinesPerBox,
    canAddBoxes,
    canBreakLines,
    glossary,
    entries,
    claudeModel,
  });
}

/** Import an edited translation.json and merge into existing entries by id. */
export function importTranslations(
  jsonPath: string,
  currentBinSha256: string,
  mode: "fill_missing" | "overwrite",
  entries: DialogueEntry[]
): Promise<ImportMergeResult> {
  return invoke<ImportMergeResult>("import_translations", {
    jsonPath,
    currentBinSha256,
    mode,
    entries,
  });
}
