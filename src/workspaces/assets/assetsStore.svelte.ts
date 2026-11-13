// State of the Browse & Extract workspace (Svelte 5 runes). Actions that talk
// to the backend live in assetsActions.ts; this file is state + persistence.
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import type {
  AgfFormat,
  EntryFilter,
  EntryPreview,
  EntryRow,
  ExtractDone,
  GameRootInfo,
  OpenedGame,
} from "./types";
import { DEFAULT_FILTER } from "./utils";

const PREFERENCES_KEY = "assetsWorkspace.preferences.v1";

/** Remembered between sessions (localStorage; separate from the editor's settings file). */
interface AssetsPreferences {
  lastGamePath: string;
  outputDir: string;
  flatten: boolean;
  agfFormat: AgfFormat;
  fsbToWav: boolean;
}

const DEFAULT_PREFERENCES: AssetsPreferences = {
  lastGamePath: "",
  outputDir: "",
  flatten: false,
  agfFormat: "Png",
  fsbToWav: false,
};

function loadPreferences(): AssetsPreferences {
  try {
    const raw = localStorage.getItem(PREFERENCES_KEY);
    return raw ? { ...DEFAULT_PREFERENCES, ...(JSON.parse(raw) as Partial<AssetsPreferences>) } : { ...DEFAULT_PREFERENCES };
  } catch {
    return { ...DEFAULT_PREFERENCES };
  }
}

export interface ExtractionStatus {
  job: number;
  total: number;
  done: number;
  currentFile: string;
}

class AssetsStore {
  game = $state<OpenedGame | null>(null);
  /** Several games found below the picked folder. */
  gameChoices = $state<GameRootInfo[] | null>(null);
  /** The folder the user picked when `gameChoices` was produced. */
  pickedFolder = $state("");
  opening = $state(false);
  openError = $state<string | null>(null);

  filter = $state<EntryFilter>({ ...DEFAULT_FILTER });
  /** Entry indices in list order (the current filter's result). */
  listing = $state<number[]>([]);
  /** Row details by entry index, fetched page by page as the table scrolls. */
  rows = new SvelteMap<number, EntryRow>();
  /** Bumped on every listing change so late page responses are dropped. */
  listingGeneration = 0;

  /** List position of the keyboard cursor / previewed row. */
  cursor = $state<number | null>(null);
  shiftAnchor = $state<number | null>(null);
  selected = new SvelteSet<number>();

  preview = $state<EntryPreview | null>(null);
  previewLoading = $state(false);
  previewGeneration = 0;

  preferences = $state<AssetsPreferences>(loadPreferences());
  extraction = $state<ExtractionStatus | null>(null);
  lastExtraction = $state<ExtractDone | null>(null);

  savePreferences(): void {
    try {
      localStorage.setItem(PREFERENCES_KEY, JSON.stringify(this.preferences));
    } catch {
      // Storage full/blocked: preferences just won't persist.
    }
  }

  resetForNewGame(): void {
    this.filter = { ...DEFAULT_FILTER };
    this.listing = [];
    this.rows.clear();
    this.listingGeneration += 1;
    this.cursor = null;
    this.shiftAnchor = null;
    this.selected.clear();
    this.preview = null;
    this.previewGeneration += 1;
    this.lastExtraction = null;
  }
}

export const assetsStore = new AssetsStore();
