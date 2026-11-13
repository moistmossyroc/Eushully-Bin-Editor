// Types mirrored from src-tauri/src/assets/commands.rs (serde camelCase) and
// the eushully-assets engine (EntryFilter, SortColumn, EntryKind).

export type EntryKind = "image" | "audio" | "script" | "video" | "other";
export type EntryTier = "original" | "officialUpdate" | "mod";
export type SortColumn = "name" | "kind" | "archive" | "offset" | "size";
export type AgfFormat = "AsIs" | "Png" | "Bmp" | "Jpeg" | "Tga" | "Tiff";

export interface EntryFilter {
  search: string;
  kind: EntryKind | null;
  folder: string;
  effectiveOnly: boolean;
  modsOnly: boolean;
  sort: SortColumn;
  descending: boolean;
}

export interface GameRootInfo {
  path: string;
  kind: "age" | "parente";
}

export interface OpenedGame {
  session: number;
  root: string;
  version: string;
  entryCount: number;
  archiveCount: number;
  notEffectiveCount: number;
  modCount: number;
  kindCounts: Partial<Record<EntryKind, number>>;
}

export type OpenGameResult =
  | ({ kind: "opened" } & OpenedGame)
  | { kind: "choose"; roots: GameRootInfo[] };

export interface EntryRow {
  index: number;
  name: string;
  kind: EntryKind;
  archive: string;
  offset: number;
  size: number;
  tier: EntryTier;
  effective: boolean;
  note: string | null;
}

export interface EntryPage {
  total: number;
  offset: number;
  rows: EntryRow[];
}

export type PreviewResult =
  | { kind: "image"; width: number; height: number }
  | { kind: "text"; text: string }
  | { kind: "audio"; format: string; durationSecs: number | null; size: number }
  | { kind: "video"; description: string }
  | { kind: "hex"; text: string }
  | { kind: "error"; message: string };

export interface EntryPreview {
  row: EntryRow;
  preview: PreviewResult;
}

export interface ExtractRequest {
  indices: number[];
  outputDir: string;
  flatten: boolean;
  agfFormat: AgfFormat;
  fsbToWav: boolean;
}

export interface ExtractStarted {
  job: number;
  total: number;
  skippedShadowed: number;
}

export interface ExtractProgress {
  job: number;
  done: number;
  total: number;
  currentFile: string;
}

export interface ExtractDone {
  job: number;
  extracted: number;
  errors: string[];
  cancelled: boolean;
  outputDir: string;
}

export const KIND_LABELS: Record<EntryKind, string> = {
  image: "Images",
  audio: "Audio",
  script: "Scripts",
  video: "Video",
  other: "Other",
};

export const AGF_FORMAT_LABELS: Record<AgfFormat, string> = {
  AsIs: "As-is (AGF)",
  Png: "PNG",
  Bmp: "BMP",
  Jpeg: "JPEG (no alpha)",
  Tga: "TGA",
  Tiff: "TIFF",
};
