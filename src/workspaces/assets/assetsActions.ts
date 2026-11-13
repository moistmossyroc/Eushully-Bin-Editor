// User actions of the Browse & Extract workspace: open a game, filter, page
// rows in, cursor/selection, preview, extract.
import { open } from "@tauri-apps/plugin-dialog";
import { openFolderInExplorer } from "../../lib/ipc";
import { toastError, toastInfo, toastWarn } from "../../lib/toast";
import { assetsStore as state } from "./assetsStore.svelte";
import {
  cancelExtract,
  closeGame,
  listEntries,
  listEntryIndices,
  onExtractDone,
  onExtractProgress,
  openGame,
  previewEntry,
  findEntry,
  startExtract,
  startupRequest,
} from "./ipc";
import { store } from "../../lib/stores.svelte";
import type { EntryFilter } from "./types";
import { indicesInRange, pagesForRange } from "./utils";

export const PAGE_SIZE = 200;

// ---------------------------------------------------------------------------
// Opening
// ---------------------------------------------------------------------------

export async function openGameAt(path: string): Promise<void> {
  state.opening = true;
  state.openError = null;
  try {
    const result = await openGame(path);
    if (result.kind === "choose") {
      state.pickedFolder = path;
      state.gameChoices = result.roots;
      return;
    }
    if (state.game) {
      void closeGame(state.game.session);
    }
    state.gameChoices = null;
    state.resetForNewGame();
    const { kind: _kind, ...opened } = result;
    state.game = opened;
    state.preferences.lastGamePath = opened.root;
    state.savePreferences();
    await reloadListing();
  } catch (error) {
    state.openError = String(error);
  } finally {
    state.opening = false;
  }
}

/** `--browse <folder> [--show <file>]` at launch: switch here, open, preview. */
export async function openStartupFolder(): Promise<void> {
  const request = await startupRequest();
  if (!request) return;
  store.workspace = "assets";
  await openGameAt(request.folder);
  if (request.show && state.game) {
    const entryIndex = await findEntry(state.game.session, request.show);
    if (entryIndex === null) {
      toastWarn(`--show: no file named ${request.show}`);
      return;
    }
    const position = state.listing.indexOf(entryIndex);
    if (position >= 0) moveCursorTo(position);
    else void showPreview(entryIndex);
  }
}

export async function pickAndOpenGame(): Promise<void> {
  const picked = await open({
    directory: true,
    title: "Open a game folder (any level — subfolders are searched)",
    defaultPath: state.preferences.lastGamePath || undefined,
  });
  if (typeof picked === "string") {
    await openGameAt(picked);
  }
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

/** Re-run the filter; keeps the previewed entry under the cursor if still listed. */
export async function reloadListing(): Promise<void> {
  const game = state.game;
  if (!game) return;
  const generation = ++state.listingGeneration;
  const filter: EntryFilter = { ...state.filter };
  try {
    const listing = await listEntryIndices(game.session, filter);
    if (generation !== state.listingGeneration) return;
    state.rows.clear();
    state.listing = listing;
    const previewed = state.preview?.row.index;
    state.cursor = previewed === undefined ? null : listing.indexOf(previewed);
    if (state.cursor === -1) state.cursor = null;
    state.shiftAnchor = null;
  } catch (error) {
    toastError(`Listing failed: ${error}`);
  }
}

export function updateFilter(changes: Partial<EntryFilter>): void {
  state.filter = { ...state.filter, ...changes };
  void reloadListing();
}

const pendingPages = new Set<string>();

/** Make sure row details for list positions [first, last] are loaded. */
export async function ensureRows(first: number, last: number): Promise<void> {
  const game = state.game;
  if (!game || state.listing.length === 0) return;
  const generation = state.listingGeneration;
  const filter: EntryFilter = { ...state.filter };
  const lastPosition = Math.min(last, state.listing.length - 1);
  for (const page of pagesForRange(first, lastPosition, PAGE_SIZE)) {
    const firstIndexOnPage = state.listing[page * PAGE_SIZE];
    if (firstIndexOnPage === undefined || state.rows.has(firstIndexOnPage)) continue;
    const key = `${generation}:${page}`;
    if (pendingPages.has(key)) continue;
    pendingPages.add(key);
    try {
      const result = await listEntries(game.session, filter, page * PAGE_SIZE, PAGE_SIZE);
      if (generation !== state.listingGeneration) return;
      for (const row of result.rows) state.rows.set(row.index, row);
    } catch (error) {
      toastError(`Loading rows failed: ${error}`);
    } finally {
      pendingPages.delete(key);
    }
  }
}

// ---------------------------------------------------------------------------
// Cursor, selection, preview
// ---------------------------------------------------------------------------

export async function showPreview(entryIndex: number): Promise<void> {
  const game = state.game;
  if (!game || state.preview?.row.index === entryIndex) return;
  const generation = ++state.previewGeneration;
  state.previewLoading = true;
  try {
    const preview = await previewEntry(game.session, entryIndex);
    if (generation === state.previewGeneration) state.preview = preview;
  } catch (error) {
    if (generation === state.previewGeneration) {
      state.preview = null;
      toastError(`Preview failed: ${error}`);
    }
  } finally {
    if (generation === state.previewGeneration) state.previewLoading = false;
  }
}

export function moveCursorTo(position: number): void {
  if (state.listing.length === 0) return;
  const clamped = Math.max(0, Math.min(position, state.listing.length - 1));
  state.cursor = clamped;
  void showPreview(state.listing[clamped]);
}

/** Click semantics (same as the AGE Extractor): plain = cursor + preview,
 *  Ctrl = toggle selection, Shift = range select from the anchor. */
export function clickRow(position: number, modifiers: { ctrl: boolean; shift: boolean }): void {
  const entryIndex = state.listing[position];
  if (entryIndex === undefined) return;
  if (modifiers.shift) {
    const anchor = state.shiftAnchor ?? state.cursor ?? position;
    for (const index of indicesInRange(state.listing, anchor, position)) state.selected.add(index);
  } else if (modifiers.ctrl) {
    toggleSelected(entryIndex);
    state.shiftAnchor = position;
  } else {
    state.shiftAnchor = position;
  }
  moveCursorTo(position);
}

export function toggleSelected(entryIndex: number): void {
  if (state.selected.has(entryIndex)) state.selected.delete(entryIndex);
  else state.selected.add(entryIndex);
}

export function selectAllShown(): void {
  for (const index of state.listing) state.selected.add(index);
}

export function clearSelection(): void {
  state.selected.clear();
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

let extractionListenersReady: Promise<void> | null = null;

function ensureExtractionListeners(): Promise<void> {
  extractionListenersReady ??= (async () => {
    await onExtractProgress((progress) => {
      if (state.extraction?.job !== progress.job) return;
      state.extraction = { ...state.extraction, done: progress.done, currentFile: progress.currentFile };
    });
    await onExtractDone((done) => {
      if (state.extraction?.job !== done.job) return;
      state.extraction = null;
      state.lastExtraction = done;
      if (done.cancelled) toastWarn("Extraction cancelled.");
      else if (done.errors.length > 0) toastWarn(`Extracted ${done.extracted} files; ${done.errors.length} failed.`);
      else toastInfo(`Extracted ${done.extracted} files.`);
    });
  })();
  return extractionListenersReady;
}

export async function pickOutputFolder(): Promise<boolean> {
  const picked = await open({ directory: true, title: "Extract to…", defaultPath: state.preferences.outputDir || undefined });
  if (typeof picked !== "string") return false;
  state.preferences.outputDir = picked;
  state.savePreferences();
  return true;
}

/** Extract the selection, or everything shown when nothing is selected. */
export async function extractChosen(): Promise<void> {
  const game = state.game;
  if (!game || state.extraction) return;
  const indices = state.selected.size > 0 ? [...state.selected] : [...state.listing];
  if (indices.length === 0) return;
  if (!state.preferences.outputDir && !(await pickOutputFolder())) return;
  await ensureExtractionListeners();
  const preferences = state.preferences;
  try {
    const started = await startExtract(game.session, {
      indices,
      outputDir: preferences.outputDir,
      flatten: preferences.flatten,
      agfFormat: preferences.agfFormat,
      fsbToWav: preferences.fsbToWav,
    });
    state.lastExtraction = null;
    state.extraction = { job: started.job, total: started.total, done: 0, currentFile: "" };
    if (started.skippedShadowed > 0) {
      toastInfo(`${started.skippedShadowed} shadowed copies skipped (the game's copy of each file is extracted).`);
    }
  } catch (error) {
    toastError(`Extraction failed to start: ${error}`);
  }
}

export function cancelRunningExtraction(): void {
  if (state.extraction) void cancelExtract(state.extraction.job);
}

export function openLastOutputFolder(): void {
  const folder = state.lastExtraction?.outputDir ?? state.preferences.outputDir;
  if (folder) void openFolderInExplorer(folder);
}
