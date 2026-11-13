// Typed wrappers for the `assets_*` commands and events (src-tauri/src/assets).
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  EntryFilter,
  EntryPage,
  EntryPreview,
  ExtractDone,
  ExtractProgress,
  ExtractRequest,
  ExtractStarted,
  OpenGameResult,
} from "./types";

const PROTOCOL = "eushully-asset";
const EXTRACT_PROGRESS_EVENT = "assets://extract-progress";
const EXTRACT_DONE_EVENT = "assets://extract-done";

/** `--browse <folder> [--show <file>]` given at launch, if any. */
export function startupRequest(): Promise<{ folder: string; show: string | null } | null> {
  return invoke<{ folder: string; show: string | null } | null>("assets_startup_request");
}

/** Entry index for a file name (the copy the game loads wins). */
export function findEntry(session: number, name: string): Promise<number | null> {
  return invoke<number | null>("assets_find_entry", { session, name });
}

/** Open a game folder (any level; subfolders are searched). */
export function openGame(path: string): Promise<OpenGameResult> {
  return invoke<OpenGameResult>("assets_open_game", { path });
}

export function closeGame(session: number): Promise<void> {
  return invoke<void>("assets_close_game", { session });
}

/** One page of the filtered, sorted list. */
export function listEntries(session: number, filter: EntryFilter, offset: number, limit: number): Promise<EntryPage> {
  return invoke<EntryPage>("assets_list", { session, filter, offset, limit });
}

/** All entry indices matching the filter, in list order. */
export function listEntryIndices(session: number, filter: EntryFilter): Promise<number[]> {
  return invoke<number[]>("assets_list_indices", { session, filter });
}

export function previewEntry(session: number, index: number): Promise<EntryPreview> {
  return invoke<EntryPreview>("assets_preview", { session, index });
}

/** URL of an entry's decoded preview image (served by the eushully-asset protocol). */
export function previewImageUrl(session: number, index: number): string {
  return convertFileSrc(`${session}/${index}/image`, PROTOCOL);
}

export function startExtract(session: number, request: ExtractRequest): Promise<ExtractStarted> {
  return invoke<ExtractStarted>("assets_extract", { session, request });
}

export function cancelExtract(job: number): Promise<void> {
  return invoke<void>("assets_cancel_extract", { job });
}

export function onExtractProgress(handler: (progress: ExtractProgress) => void): Promise<UnlistenFn> {
  return listen<ExtractProgress>(EXTRACT_PROGRESS_EVENT, (event) => handler(event.payload));
}

export function onExtractDone(handler: (done: ExtractDone) => void): Promise<UnlistenFn> {
  return listen<ExtractDone>(EXTRACT_DONE_EVENT, (event) => handler(event.payload));
}
