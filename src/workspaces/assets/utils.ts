// Pure helpers for the Browse & Extract workspace.
import type { EntryFilter, EntryTier, SortColumn } from "./types";

export const DEFAULT_FILTER: EntryFilter = {
  search: "",
  kind: null,
  folder: "",
  effectiveOnly: false,
  modsOnly: false,
  sort: "name",
  descending: false,
};

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

export function formatOffset(offset: number): string {
  return `0x${offset.toString(16).toUpperCase().padStart(8, "0")}`;
}

export function tierLabel(tier: EntryTier): string {
  switch (tier) {
    case "original": return "Original";
    case "officialUpdate": return "Official update";
    case "mod": return "Mod";
  }
}

/** Clicking a sorted column flips direction; another column sorts ascending. */
export function nextSort(filter: EntryFilter, column: SortColumn): Pick<EntryFilter, "sort" | "descending"> {
  return filter.sort === column ? { sort: column, descending: !filter.descending } : { sort: column, descending: false };
}

/** Entry indices between two list positions (inclusive, either order). */
export function indicesInRange(listing: readonly number[], fromPosition: number, toPosition: number): number[] {
  const low = Math.max(0, Math.min(fromPosition, toPosition));
  const high = Math.min(listing.length - 1, Math.max(fromPosition, toPosition));
  return listing.slice(low, high + 1);
}

/** Page numbers needed to show list positions [first, last]. */
export function pagesForRange(first: number, last: number, pageSize: number): number[] {
  const pages: number[] = [];
  for (let page = Math.floor(Math.max(0, first) / pageSize); page <= Math.floor(Math.max(0, last) / pageSize); page++) {
    pages.push(page);
  }
  return pages;
}

/** `root` shown relative to the folder it was found in (`root` if not below it). */
export function pathRelativeTo(root: string, folder: string): string {
  const normalize = (path: string) => path.replace(/[\\/]+$/, "").toLowerCase();
  const prefix = normalize(folder);
  if (!prefix || !root.toLowerCase().startsWith(prefix)) return root;
  return root.slice(prefix.length).replace(/^[\\/]+/, "") || root;
}

/** "Extract 12 selected (3 shadowed skipped)" style label. */
export function extractButtonLabel(count: number, scope: "selected" | "shown"): string {
  return `Extract ${count.toLocaleString()} ${scope}`;
}
