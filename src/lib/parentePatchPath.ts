/**
 * Parente patch file naming. The game loads EVERY `*.arc` under `Archives`
 * (recursively) and ranks them by the header timestamp, not the name — so the
 * patch can carry a human-readable, versioned name players can check.
 * (Mirrors `DEFAULT_PATCH_FILE_NAME` in src-tauri/src/parente/mod.rs.)
 */
export const DEFAULT_PARENTE_PATCH_FILE_NAME = "moist English TL v1.0.arc";

/** Split an absolute path into its folder (forward slashes) and file name. */
export function splitExportPath(path: string): { folder: string; fileName: string } {
  const normalized = path.replace(/\\/g, "/");
  const slash = normalized.lastIndexOf("/");
  return slash < 0
    ? { folder: "", fileName: normalized }
    : { folder: normalized.slice(0, slash), fileName: normalized.slice(slash + 1) };
}

/**
 * Install instructions for a freshly built (slim) patch. The patch carries only
 * the files it changes, so the official ver10x_x1.arc patches MUST stay
 * installed — they keep serving everything else.
 */
export function parentePatchInstallHint(outName: string, destructive: boolean): string {
  const install =
    `Install: put ${outName} in Start\\Archives\\Patch next to the official ver10x_x1.arc patches ` +
    `(keep them; remove any older TL patch).`;
  return destructive
    ? `${install} Dialogue plays in English as-is.`
    : `${install} Then use "Set game language: English" (the in-game language button crashes).`;
}

/** Save-dialog default: last export folder + last patch name (or the default). */
export function parentePatchSaveDefault(lastFolder: string | null, lastFileName: string | null): string {
  const fileName = lastFileName?.toLowerCase().endsWith(".arc") ? lastFileName : DEFAULT_PARENTE_PATCH_FILE_NAME;
  return lastFolder ? `${lastFolder}/${fileName}` : fileName;
}
