import { open, save } from "@tauri-apps/plugin-dialog";
import { readDir, exists as fsExists } from "@tauri-apps/plugin-fs";
import { confirmDialog } from "./confirm";
import type { TabState } from "./stores.svelte";
import {
  loadBin,
  getEngineInfo,
  loadProject,
  loadBinForProject,
  computeFileHash,
  getReferencedSpeakers,
  getRowSpeakers,
  saveProject,
  exportBin,
  exportFanpatch,
  packFanpatchFromDir,
  packDualLanguageFanpatch,
  ensureDirectory,
  fileExists,
  scanBinForCharacters,
  getBridgeTemplateStatus,
  exportForTranslation,
  getSplitCapability,
  autoSplitEntry,
  autoSplitEntriesBatch,
  importTranslations,
  lookupCharacterNamesForPointers,
  extractChinitIdNameMap,
  readPriorTranslationMap,
  loadFolderGlossary,
  scanTranslationProjectStatus,
  scanBinFolder,
  extractAlfToFolder,
  isFolderWritable,
  copyFilesToFolder,
  pathIsDir,
  writeMultiTranslateLaunchers,
  parenteListArc,
  parenteListGame,
  parenteLoadGameEntry,
  parenteExportEnglish,
  parenteSaveProject,
  parenteLoadProject,
  type TranslationRow,
  type TranslationFolderStatus,
  type SplitCapability,
  type SysBinFile,
  type ParenteGameFile,
  type ParentePatchFile,
} from "./ipc";
import { isStructuralOnly, hasTranslatableContent } from "./exportOmit";
import { isNameItemFile, isDialogueBin, isUiAbbreviation } from "./fileCategory";
import { resolveInsertionsForExport } from "./insertions";
import { store } from "./stores.svelte";
import type { CharacterEntry } from "./types";

/** Fetch and cache bridge template quality for auto-split badge. */
async function fetchBridgeQuality(tab: TabState): Promise<void> {
  try {
    const quality = await getBridgeTemplateStatus(tab.id);
    tab.bridgeTemplateQuality = quality as "clean" | "degraded" | "unavailable";
  } catch {
    tab.bridgeTemplateQuality = "unavailable";
  }
}

/**
 * Merge character names from an init BIN file into the tab's character list.
 * Only fills in entries that currently have no name.
 * Returns the number of newly resolved names.
 */
async function mergeNamesFromInitBin(tab: import("./stores.svelte").TabState, initPath: string): Promise<number> {
  try {
    const initChars = await scanBinForCharacters(initPath);
    const nameMap = new Map<number, string>(
      initChars.filter((c) => c.name).map((c) => [c.pointer, c.name])
    );

    let resolved = 0;
    tab.characters = tab.characters.map((c) => {
      if (!c.name && nameMap.has(c.pointer)) {
        resolved++;
        return { ...c, name: nameMap.get(c.pointer)! };
      }
      return c;
    });
    return resolved;
  } catch {
    return 0;
  }
}

/**
 * Eushully games use a `$N$<NAME>.BIN` filename convention for append/DLC
 * content. An append scene like `$1$SC4000.BIN` may reference characters
 * that ONLY exist in the matching append init files (`$1$CHINIT.BIN`,
 * `$1$CVINIT.BIN`, etc.) — without checking those, the editor would leave
 * the new characters unresolved.
 *
 * Returns the prioritized list of init filenames to scan for a given loaded
 * BIN: when the BIN has an `$N$` prefix, the append-prefixed names come
 * first (so append-introduced character IDs win), followed by the bare base
 * names as fallback for everything inherited from the base game.
 */
function getInitFilenameCandidates(binFilename: string, baseNames: string[]): string[] {
  const m = binFilename.match(/^(\$\d+\$)/);
  if (!m) return baseNames;
  const prefix = m[1];
  const out: string[] = [];
  for (const n of baseNames) out.push(prefix + n);
  for (const n of baseNames) out.push(n);
  return out;
}

/**
 * Try to find character init BINs (CVINIT/CNINIT/CIINIT) alongside the loaded BIN
 * and merge their names into the detected speaker ID list. Works for SYS4 and SYS5
 * since both use the same sequential-ID register pattern in their init files.
 * Only fills entries that have no name yet, so inline literal speakers stay intact.
 * Returns the number of names resolved, or -1 if no init files were found.
 */
async function tryMergeInitNames(tab: import("./stores.svelte").TabState): Promise<number> {
  if (!tab.binPath || tab.characters.length === 0) return 0;

  const normalizedPath = tab.binPath.replace(/\\/g, "/");
  const dirParts = normalizedPath.split("/");
  const binFilename = dirParts.pop() ?? "";
  const dir = dirParts.join("/");

  // Order matters: CVINIT first (main cast), then CNINIT/CIINIT/CHINIT to fill NPCs.
  // CHINIT (SYS4) carries an extra set of speaker-slot names at a different
  // address range (e.g. 0x26CC+) that scene BINs reference via the bracket triplet.
  // For append BINs (e.g. `$1$SC4000.BIN`) the matching `$1$CVINIT.BIN`,
  // `$1$CHINIT.BIN`, etc. are scanned FIRST so append-introduced characters
  // resolve from their own init files before falling back to the base game's.
  const initFilenames = getInitFilenameCandidates(
    binFilename,
    ["CVINIT.BIN", "CNINIT.BIN", "CIINIT.BIN", "CHINIT.BIN"],
  );

  let totalResolved = 0;
  let foundAny = false;

  for (const filename of initFilenames) {
    if (!tab.characters.some((c) => !c.name)) break; // all filled, stop early
    const initPath = `${dir}/${filename}`;
    if (await fileExists(initPath)) {
      foundAny = true;
      totalResolved += await mergeNamesFromInitBin(tab, initPath);
    }
  }

  return foundAny ? totalResolved : -1;
}

/**
 * Second-pass character name resolution via direct pointer lookup.
 *
 * `tryMergeInitNames` uses structural heuristics (CVINIT block pattern,
 * sequential-ID register) that miss characters whose names live outside the
 * canonical pattern. This pass walks every set-string opcode across the scene
 * BIN + every *INIT*.BIN in the same folder and matches by pointer.
 *
 * Additive only — fills blanks, never overrides. Returns the number of names
 * newly resolved.
 */
/**
 * Populate a tab's engine indicator (game name + SYS build) on the project-restore
 * paths, which load the BIN via `loadBinForProject` and so don't receive a
 * `LoadBinResult`. Best-effort: falls back to a bare `SYS<n>` label on failure.
 */
async function applyEngineInfo(tab: import("./stores.svelte").TabState): Promise<void> {
  try {
    const e = await getEngineInfo(tab.id);
    tab.engineVersion = e.engine_version;
    tab.engineLabel = e.engine_label;
  } catch {
    tab.engineLabel = `SYS${tab.sysVersion}`;
  }
}

async function tryDirectPointerLookup(tab: import("./stores.svelte").TabState): Promise<number> {
  if (!tab.binPath || tab.characters.length === 0) return 0;
  // Skip pointers that look like absolute addresses, not character IDs.
  const unresolved = tab.characters
    .filter((c) => !c.name && c.pointer < 0x10000)
    .map((c) => c.pointer);
  if (unresolved.length === 0) return 0;

  const normalizedPath = tab.binPath.replace(/\\/g, "/");
  const dirParts = normalizedPath.split("/");
  const binFilename = dirParts.pop() ?? "";
  const dir = dirParts.join("/");

  const candidatePaths: string[] = [tab.binPath];
  // Append-aware: for `$N$<name>.BIN` scenes, scan `$N$*INIT*.BIN` first
  // then base init files. Order doesn't affect correctness here (the lookup
  // collects matching pointers from all files), but putting append-prefixed
  // files first is consistent with the other two resolution passes.
  const initCandidates = getInitFilenameCandidates(
    binFilename,
    ["CVINIT.BIN", "CIINIT.BIN", "CNINIT.BIN", "CNINIT2.BIN", "CHINIT.BIN"],
  );
  for (const fn of initCandidates) {
    const p = `${dir}/${fn}`;
    if (await fileExists(p)) candidatePaths.push(p);
  }

  try {
    const result = await lookupCharacterNamesForPointers(candidatePaths, unresolved);
    if (Object.keys(result).length === 0) return 0;

    let resolved = 0;
    tab.characters = tab.characters.map((c) => {
      if (c.name) return c;
      const found = result[String(c.pointer)] ?? result[c.pointer as unknown as string];
      if (found) {
        resolved++;
        return { ...c, name: found };
      }
      return c;
    });
    return resolved;
  } catch {
    return 0;
  }
}

/**
 * Third-pass character name resolution via CHINIT's consecutive-register
 * character table.
 *
 * Some characters (notably SYS4 NPCs and side-cast) appear in scene BINs as
 * small literal IDs (e.g. `mov g-int:0x37D0, lit:0x61`) that aren't a direct
 * address or a CVINIT/CIINIT-style ID. The engine resolves them at runtime by
 * indexing into a table CHINIT initializes via the pattern
 * `mov g-int:<R>, lit:<ID>` immediately followed by `set-string g-str:<A>,
 * str:"<name>"`. We extract that table once and use it to fill in any IDs the
 * first two passes missed.
 *
 * Additive only — never overrides an already-resolved name. Returns the count
 * of newly resolved names.
 */
async function tryChinitIdMapLookup(tab: import("./stores.svelte").TabState): Promise<number> {
  if (!tab.binPath || tab.characters.length === 0) return 0;
  const unresolved = tab.characters.filter((c) => !c.name && c.pointer < 0x10000);
  if (unresolved.length === 0) return 0;

  const normalized = tab.binPath.replace(/\\/g, "/");
  const dir = normalized.substring(0, normalized.lastIndexOf("/"));
  const binFilename = normalized.substring(normalized.lastIndexOf("/") + 1);

  // ID→name tables in priority order: CHINIT (canonical for triplet/CVINIT
  // games), then CNINIT/CIINIT — where `0x1D2` display-slot games (e.g. Megami
  // Zero) store their character names in the SAME mov-lit-then-set-string pattern
  // `extract_chinit_id_name_map` reads. For each, the `$N$` append variant takes
  // precedence over base (appends add new characters / rebind base IDs).
  const idTableCandidates = ["CHINIT.BIN", "CNINIT.BIN", "CIINIT.BIN"].flatMap(
    (n) => getInitFilenameCandidates(binFilename, [n]),
  );
  const mergedMap: Record<string, string> = {};
  // Iterate high-priority first; first write wins (so CHINIT, and append-over-base,
  // are not overwritten by lower-priority tables).
  for (const fn of idTableCandidates) {
    const path = `${dir}/${fn}`;
    if (!(await fileExists(path))) continue;
    try {
      const m = await extractChinitIdNameMap(path);
      for (const [k, v] of Object.entries(m)) {
        if (!(k in mergedMap)) mergedMap[k] = v;
      }
    } catch {
      // skip unreadable file
    }
  }
  const map = mergedMap;
  if (!map || Object.keys(map).length === 0) return 0;

  let resolved = 0;
  tab.characters = tab.characters.map((c) => {
    if (c.name) return c;
    const name = map[String(c.pointer)];
    if (name) {
      resolved++;
      return { ...c, name };
    }
    return c;
  });
  return resolved;
}

/** Silently save a single tab to its projectPath. No prompts. Creates the Projects folder on first save. */
async function autoSaveTab(tab: TabState): Promise<boolean> {
  if (!tab.projectPath || !tab.isDirty) return false;
  try {
    // Ensure the Projects directory exists (first save creates it)
    const dirParts = tab.projectPath.replace(/\\/g, "/").split("/");
    dirParts.pop(); // remove filename
    await ensureDirectory(dirParts.join("/"));

    await saveProject(
      tab.id,
      tab.projectPath,
      tab.entries,
      tab.glossary,
      tab.characters,
      resolveInsertionsForExport(tab.insertedInstructions, tab.entries),
      tab.claudeModel,
    );
    tab.isDirty = false;
    return true;
  } catch (err) {
    // Autosave failure is the silent-bug case par excellence — user thinks
    // their edits are saved, but they aren't. Surface via toast so the user
    // knows to manually save (or fix the underlying issue: permissions,
    // disk full, file locked by another process).
    const { toastError } = await import("./toast");
    toastError(`Autosave failed for ${tab.sourceBinFilename}: ${err}`);
    return false;
  }
}

/** Auto-save all dirty tabs that have a projectPath. Called reactively on row/tab changes. */
export async function doAutoSaveAll(): Promise<void> {
  // BIN tabs autosave to their project JSON next to the BIN.
  const dirtyTabs = store.tabs.filter((t) => t.format !== "parente" && t.isDirty && t.projectPath);
  // Parente tabs autosave to a per-scene project in the app data dir.
  const dirtyParente = store.tabs.filter((t) => t.format === "parente" && t.isDirty && t.parenteArcPath);
  if (dirtyTabs.length === 0 && dirtyParente.length === 0) return;

  let savedCount = 0;
  for (const tab of dirtyTabs) {
    if (await autoSaveTab(tab)) savedCount++;
  }
  for (const tab of dirtyParente) {
    if (await saveParenteProject(tab)) savedCount++;
  }
  if (savedCount > 0) {
    const now = new Date();
    store.lastSaveTime = now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  }
}

/** Stable per-scene key for a Parente tab's saved project. Keyed by scene
 *  path+name (stable across arc versions); sanitized for use as a filename. */
function parenteProjectKey(tab: TabState): string {
  return `${tab.parentePath}/${tab.parenteName}`.replace(/[^A-Za-z0-9._-]/g, "_");
}

/** Persist a Parente tab's translations + notes to the app data dir. Clears the
 *  dirty flag on success so a saved tab closes without a prompt. */
export async function saveParenteProject(tab: TabState): Promise<boolean> {
  if (tab.format !== "parente") return false;
  try {
    const project = {
      format: "parente",
      arc_name: tab.parenteName,
      path: tab.parentePath,
      // Only store rows that have a translation or note (sparse, ordinal-keyed).
      rows: tab.entries
        .filter((e) => e.translated_text.trim() !== "" || e.notes.trim() !== "")
        .map((e) => ({
          ordinal: Number(e.instruction_index),
          original: e.original_text,
          translated: e.translated_text,
          notes: e.notes,
        })),
    };
    await parenteSaveProject(parenteProjectKey(tab), JSON.stringify(project));
    tab.isDirty = false;
    return true;
  } catch (err) {
    store.statusMessage = `Failed to save Parente project: ${err}`;
    return false;
  }
}

/** Overlay any saved project translations onto freshly-loaded Parente entries,
 *  matched by ordinal (with original-text sanity check). */
async function applyParenteProject(tab: TabState): Promise<void> {
  try {
    const raw = await parenteLoadProject(parenteProjectKey(tab));
    if (!raw) return;
    const project = JSON.parse(raw) as {
      rows?: { ordinal: number; original?: string; translated?: string; notes?: string }[];
    };
    if (!project.rows?.length) return;
    const byOrdinal = new Map(project.rows.map((r) => [r.ordinal, r]));
    let restored = 0;
    tab.entries = tab.entries.map((e) => {
      const saved = byOrdinal.get(Number(e.instruction_index));
      // Only apply if the original text still matches (scene unchanged at that
      // ordinal) — guards against the arc changing under a saved project.
      if (saved && (saved.original === undefined || saved.original === e.original_text)) {
        restored++;
        return { ...e, translated_text: saved.translated ?? e.translated_text, notes: saved.notes ?? e.notes };
      }
      return e;
    });
    if (restored > 0) {
      store.statusMessage = `Restored ${restored} saved translation(s) for ${tab.parenteName}`;
    }
  } catch {
    // Non-fatal — missing/corrupt project just means no restore.
  }
}

/** Compute the project JSON path in a Projects subfolder next to the BIN. Does not create the directory. */
function getProjectPathForBin(binPath: string, binFilename: string): string {
  const normalizedPath = binPath.replace(/\\/g, "/");
  const dirParts = normalizedPath.split("/");
  dirParts.pop(); // remove filename
  const binDir = dirParts.join("/");
  return `${binDir}/Projects/${binFilename.replace(/\.[^.]+$/, ".json")}`;
}

/** Prompt user for path and save the current project as JSON. Returns true if saved. */
export async function doSaveProject(): Promise<boolean> {
  const tab = store.activeTab;
  if (!tab) return false;

  try {
    const defaultPath = tab.projectPath || tab.sourceBinFilename.replace(/\.[^.]+$/, ".json");
    const selected = await save({
      filters: [{ name: "Project Files", extensions: ["json"] }],
      defaultPath,
    });
    if (!selected) return false;

    store.statusMessage = "Saving project...";
    await saveProject(
      tab.id,
      selected,
      tab.entries,
      tab.glossary,
      tab.characters,
      resolveInsertionsForExport(tab.insertedInstructions, tab.entries),
      tab.claudeModel,
    );
    tab.projectPath = selected;
    tab.isDirty = false;
    store.statusMessage = `Project saved to ${selected.replace(/\\/g, "/").split("/").pop()}`;
    return true;
  } catch (err) {
    store.statusMessage = `Error saving: ${err}`;
    return false;
  }
}

/** Load a BIN file into a new tab. If a matching project JSON exists in a Projects subfolder, restores from it. */
export async function doLoadBin(filePath: string): Promise<void> {
  try {
    const parts = filePath.replace(/\\/g, "/").split("/");
    const binFilename = parts[parts.length - 1];

    // Check if a project file already exists for this BIN
    const projectPath = getProjectPathForBin(filePath, binFilename);
    if (await fileExists(projectPath)) {
      // Existing project found — load it with this BIN, preserving all prior work
      store.statusMessage = `Found existing project for ${binFilename}, restoring...`;
      const tab = store.createTab();

      const project = await loadProject(projectPath);
      const hashMatches = await loadBinForProject(tab.id, filePath, project.source_bin_sha256);
      fetchBridgeQuality(tab);

      if (!hashMatches) {
        const proceed = await confirmDialog({
          title: "Hash mismatch",
          confirmLabel: "Continue",
          message: `The BIN file hash does not match the saved project for ${binFilename}. This may be a different version. Continue anyway?`,
        });
        if (!proceed) {
          store.statusMessage = "Cancelled — hash mismatch";
          const tabIndex = store.tabs.indexOf(tab);
          if (tabIndex !== -1) store.closeTab(tabIndex);
          return;
        }
      }

      tab.entries = project.entries;
      tab.glossary = project.glossary;

      // Reconstruct _syntheticIndex links
      const negativeEntries = project.entries.filter((e) => e.instruction_index < 0);
      const usedIndices = new Set<number>();
      for (const ins of project.inserted_instructions) {
        const match = negativeEntries.find(
          (e) =>
            e.label === ins.label &&
            e.opcode === ins.opcode &&
            !usedIndices.has(e.instruction_index)
        );
        if (match) {
          ins._syntheticIndex = match.instruction_index;
          usedIndices.add(match.instruction_index);
        }
      }
      tab.insertedInstructions = project.inserted_instructions;
      tab.characters = project.characters || [];

      tab.binPath = filePath;
      tab.projectPath = projectPath;
      tab.sourceBinFilename = project.source_bin_filename;
      tab.sourceBinHash = project.source_bin_sha256;
      tab.sysVersion = project.sys_version || (project.is_ver5 ? 5 : 4);
      await applyEngineInfo(tab);

      // For SYS3/SYS5 — and any project that saved no characters (e.g. a SYS4
      // 0x1D2 game like Megami Zero opened before that detection existed) —
      // re-detect speakers from the BIN since detection may have improved.
      if (tab.sysVersion <= 3 || tab.sysVersion >= 5 || tab.characters.length === 0) {
        try {
          const speakers = await getReferencedSpeakers(tab.id);
          if (speakers.length > 0) {
            // Preserve any user-edited names from the project
            const existingTranslations = new Map(
              tab.characters
                .filter((c) => c.name && c.name !== "Unknown")
                .map((c) => [c.pointer, c.name])
            );
            tab.characters = speakers
              .sort((a, b) => a.pointer - b.pointer)
              .map((s) => ({
                ...s,
                name: existingTranslations.get(s.pointer) || s.name,
              }));
          }
        } catch {
          // Keep project characters as fallback
        }
      }

      // Fill in any remaining empty names from CVINIT/CNINIT/CIINIT,
      // then by direct address lookup, then by CHINIT's ID-register table.
      await tryMergeInitNames(tab);
      await tryDirectPointerLookup(tab);
      await tryChinitIdMapLookup(tab);

      try {
        tab.rowSpeakers = await getRowSpeakers(tab.id);
      } catch {
        tab.rowSpeakers = {};
      }
      tab.claudeModel = project.settings.claude_model;
      store.apiKeyConfigured = project.settings.api_key_configured;

      store.statusMessage = `Restored project for ${binFilename}`;
      return;
    }

    // No existing project — fresh load
    store.statusMessage = "Loading BIN file...";
    const tab = store.createTab();

    const result = await loadBin(tab.id, filePath);
    tab.entries = result.entries;
    tab.sysVersion = result.sys_version;
    tab.engineVersion = result.engine_version;
    tab.engineLabel = result.engine_label;
    tab.binPath = filePath;

    tab.sourceBinFilename = binFilename;
    tab.sourceBinHash = await computeFileHash(filePath);
    fetchBridgeQuality(tab);

    try {
      const speakers = await getReferencedSpeakers(tab.id);
      if (speakers.length > 0) {
        tab.characters = speakers.sort((a, b) => a.pointer - b.pointer);
      }
    } catch {
      // Some BINs may not have detectable speaker references
    }

    try {
      tab.rowSpeakers = await getRowSpeakers(tab.id);
    } catch {
      tab.rowSpeakers = {};
    }

    // Auto-resolve character names from CVINIT/CNINIT/CIINIT if present,
    // then direct address lookup, then CHINIT's ID-register table.
    const initResult = await tryMergeInitNames(tab);
    await tryDirectPointerLookup(tab);
    await tryChinitIdMapLookup(tab);

    // Folder-level glossary: if <binDir>/glossary.json exists, merge its
    // entries into the tab's project glossary. Skips entries whose japanese
    // already exists to avoid stomping in-editor changes.
    try {
      const folderEntries = await loadFolderGlossary(filePath);
      if (folderEntries.length > 0) {
        const existing = new Set(tab.glossary.map((g) => g.japanese));
        const merged = folderEntries.filter((g) => g.japanese && !existing.has(g.japanese));
        if (merged.length > 0) {
          tab.glossary = [...tab.glossary, ...merged];
        }
      }
    } catch {
      // Non-fatal — glossary.json absent or malformed.
    }

    // Set the intended project path but don't write yet —
    // the project file and Projects folder will be created on first edit via auto-save
    tab.projectPath = projectPath;

    const entryCount = tab.entries.filter((e) => !e.is_context_row).length;
    const hasUnnamed = tab.characters.some((c) => !c.name);
    if (initResult === -1 && hasUnnamed && tab.characters.length > 0) {
      store.statusMessage = `Loaded ${binFilename} — ${entryCount} entries. No init files found — use Characters > Scan BIN for Names to load character names`;
    } else {
      store.statusMessage = `Loaded ${binFilename} — ${entryCount} dialogue entries`;
    }
  } catch (err) {
    store.statusMessage = `Error: ${err}`;
  }
}

/** Load a project JSON file into a new tab. */
export async function doLoadProject(filePath: string): Promise<void> {
  try {
    store.statusMessage = "Loading project...";
    const project = await loadProject(filePath);

    // Try to find the BIN file one directory above the project JSON
    const normalizedPath = filePath.replace(/\\/g, "/");
    const projectDirParts = normalizedPath.split("/");
    projectDirParts.pop(); // remove json filename
    const projectDir = projectDirParts.join("/");
    const parentDir = projectDirParts.slice(0, -1).join("/");
    const autoDetectPath = `${parentDir}/${project.source_bin_filename}`;

    let binFilePath: string;
    if (await fileExists(autoDetectPath)) {
      binFilePath = autoDetectPath;
      store.statusMessage = `Found ${project.source_bin_filename} in parent directory`;
    } else {
      // Also check same directory as the project file
      const sameDirPath = `${projectDir}/${project.source_bin_filename}`;
      if (await fileExists(sameDirPath)) {
        binFilePath = sameDirPath;
        store.statusMessage = `Found ${project.source_bin_filename} alongside project`;
      } else {
        // Fall back to manual selection
        const binFile = await open({
          multiple: false,
          title: `Select the BIN file: ${project.source_bin_filename}`,
          filters: [{ name: "BIN Files", extensions: ["BIN", "bin"] }],
        });

        if (!binFile) {
          store.statusMessage = "Cancelled — BIN file required";
          return;
        }

        // Tauri 2's open() with multiple:false returns `string | null`. The
        // earlier defensive `.path` fallback was a no-op the type checker
        // (rightly) rejected — we never see an object form here.
        binFilePath = binFile;
      }
    }
    const tab = store.createTab();

    const hashMatches = await loadBinForProject(
      tab.id,
      binFilePath,
      project.source_bin_sha256
    );
    fetchBridgeQuality(tab);

    if (!hashMatches) {
      const proceed = await confirmDialog({
        title: "Hash mismatch",
        confirmLabel: "Continue",
        message: "The BIN file hash does not match the project. This may be a different version. Continue anyway?",
      });
      if (!proceed) {
        store.statusMessage = "Cancelled — hash mismatch";
        const tabIndex = store.tabs.indexOf(tab);
        if (tabIndex !== -1) store.closeTab(tabIndex);
        return;
      }
    }

    tab.entries = project.entries;
    tab.glossary = project.glossary;

    // Reconstruct _syntheticIndex links
    const negativeEntries = project.entries.filter((e) => e.instruction_index < 0);
    const usedIndices = new Set<number>();
    for (const ins of project.inserted_instructions) {
      const match = negativeEntries.find(
        (e) =>
          e.label === ins.label &&
          e.opcode === ins.opcode &&
          !usedIndices.has(e.instruction_index)
      );
      if (match) {
        ins._syntheticIndex = match.instruction_index;
        usedIndices.add(match.instruction_index);
      }
    }
    tab.insertedInstructions = project.inserted_instructions;
    tab.characters = project.characters || [];

    tab.binPath = binFilePath;
    tab.projectPath = filePath;
    tab.sourceBinFilename = project.source_bin_filename;
    tab.sourceBinHash = project.source_bin_sha256;
    tab.sysVersion = project.sys_version || (project.is_ver5 ? 5 : 4);
    await applyEngineInfo(tab);
    tab.claudeModel = project.settings.claude_model;
    store.apiKeyConfigured = project.settings.api_key_configured;

    // For SYS3/SYS5, re-detect speakers from the BIN
    if (tab.sysVersion <= 3 || tab.sysVersion >= 5) {
      try {
        const speakers = await getReferencedSpeakers(tab.id);
        if (speakers.length > 0) {
          const existingTranslations = new Map(
            tab.characters
              .filter((c) => c.name && c.name !== "Unknown")
              .map((c) => [c.pointer, c.name])
          );
          tab.characters = speakers
            .sort((a, b) => a.pointer - b.pointer)
            .map((s) => ({
              ...s,
              name: existingTranslations.get(s.pointer) || s.name,
            }));
        }
      } catch {
        // Keep project characters as fallback
      }
    }

    // Fill in empty names from CVINIT/CNINIT/CIINIT, then by CHINIT's table.
    await tryMergeInitNames(tab);
    await tryDirectPointerLookup(tab);
    await tryChinitIdMapLookup(tab);

    try {
      tab.rowSpeakers = await getRowSpeakers(tab.id);
    } catch {
      tab.rowSpeakers = {};
    }

    store.statusMessage = `Loaded project for ${tab.sourceBinFilename}`;
  } catch (err) {
    store.statusMessage = `Error: ${err}`;
  }
}

/** Cap on the Recent Files list. Tuned for "I work with N games" — large
 *  enough to span multiple projects, small enough to scan quickly. */
const RECENT_FILES_MAX = 12;

/** Push a path to the front of the recent-files list, dedupe-by-path,
 *  cap to max size, persist. Called from doLoadFile so any successful
 *  open feeds the menu. Failed opens (file vanished, parse error) are
 *  still recorded — the user can use Recent to "find that file again"
 *  even after a move. */
function trackRecentFile(filePath: string): void {
  const normalized = filePath.replace(/\\/g, "/");
  const without = store.recentFiles.filter((p) => p.replace(/\\/g, "/") !== normalized);
  store.recentFiles = [filePath, ...without].slice(0, RECENT_FILES_MAX);
  // Fire-and-forget; settings save is non-critical.
  void import("./settings").then((m) => m.persistAllSettingsDebounced());
}

/** Load a file by path, auto-detecting BIN vs project JSON. */
export async function doLoadFile(filePath: string): Promise<void> {
  trackRecentFile(filePath);
  const lower = filePath.toLowerCase();
  if (lower.endsWith(".json")) {
    await doLoadProject(filePath);
  } else if (lower.endsWith(".arc")) {
    await doOpenParenteArc(filePath);
  } else {
    await doLoadBin(filePath);
  }
}

/** Open a single Parente .arc: list its dialogue scenes and raise the picker. */
export async function doOpenParenteArc(arcPath: string): Promise<void> {
  try {
    store.statusMessage = "Reading archive…";
    const files = await parenteListArc(arcPath);
    if (files.length === 0) {
      store.statusMessage = "No translatable scene files found in this .arc";
      return;
    }
    const arcName = arcPath.replace(/\\/g, "/").split("/").pop() || "archive.arc";
    store.parenteArcRequest = { title: arcName, files };
    store.statusMessage = `${files.length} scene file(s) in archive — choose which to load`;
  } catch (err) {
    store.statusMessage = `Error reading archive: ${err}`;
  }
}

/** Open a whole game folder: resolve the effective (latest) dialogue scenes
 *  across every archive and raise the picker. */
export async function doOpenParenteGameFolder(dir: string): Promise<void> {
  try {
    store.statusMessage = "Scanning game archives…";
    const files = await parenteListGame(dir);
    if (files.length === 0) {
      store.statusMessage = "No translatable scene files found in this game folder";
      return;
    }
    store.parenteArcRequest = { title: "Game folder", files, gameDir: dir };
    store.statusMessage = `${files.length} dialogue file(s) across the game — choose which to load`;
  } catch (err) {
    store.statusMessage = `Error scanning game folder: ${err}`;
  }
}

/** Scan a SYS3/4/5 game folder for script BINs and open the folder picker.
 *  Returns true if it found AGE BINs (so a folder drop can fall back to the
 *  Parente scanner when this returns false). */
export async function doOpenSysGameFolder(dir: string): Promise<boolean> {
  const name = dir.replace(/\\/g, "/").split("/").pop() || dir;
  // Indeterminate overlay — the scan is one Rust call (parses loose BINs +
  // inflates the .alf index) with no per-item progress; total:0 = spinner.
  store.loadingProgress = { current: 0, total: 0, currentFile: `Scanning ${name} for BIN files…` };
  try {
    const spPrefixes = store.spScenePrefixes.split(",").map((s) => s.trim()).filter(Boolean);
    const files = await scanBinFolder(dir, spPrefixes);
    if (files.length === 0) return false;
    // Detect admin-locked folders so the picker can offer a writable workspace
    // (autosave/backups write next to the BIN and would otherwise fail).
    const writable = await isFolderWritable(dir);
    store.sysBinFolderRequest = { folder: dir, files, writable };
    const scenes = files.filter((f) => f.category === "sc").length;
    const ro = writable ? "" : " — read-only folder, you'll choose a workspace";
    store.statusMessage = `${files.length} BIN file(s) found (${scenes} scene)${ro}.`;
    return true;
  } catch (err) {
    store.statusMessage = `Error scanning folder: ${err}`;
    return false;
  } finally {
    store.loadingProgress = null;
  }
}

/** Set up a writable workspace from a game folder and open the selected files.
 *
 *  Materializes exactly `selected ∪ name-tables` into the workspace — NOT the
 *  whole archive:
 *   - Every SELECTED file (any category) is materialized: loose → copied,
 *     archived (`.alf`) → extracted. This is the user's actual pick, so it must
 *     always land in the workspace (a full-archive install like VERITA keeps the
 *     real scenes packed, so the selection is mostly archived entries).
 *   - The character/item NAME tables (`category === "names"` = CVINIT/CIINIT/
 *     CNINIT/CHINIT/…) are added as support, again loose→copy / archived→extract,
 *     so speaker names resolve. These are the only files the name-resolution
 *     passes read; we deliberately do NOT extract every non-scene BIN (that's
 *     what made VERITA try to unpack ~1600 BINs and fail).
 *
 *  Needed when the source folder is read-only OR a selected/name file is archived.
 *
 *  The chosen workspace is remembered per source game folder
 *  (`store.workspaceFolders`, persisted in settings.json): a later open of the
 *  same game reuses it without re-prompting, and any file already materialized
 *  there is skipped (no re-copy/re-extract). A remembered path that no longer
 *  exists falls back to the picker.
 */
export async function doSetupWorkspaceAndOpen(
  folder: string,
  selected: SysBinFile[],
  all: SysBinFile[],
): Promise<void> {
  if (selected.length === 0) {
    store.statusMessage = "No files selected to open.";
    return;
  }
  try {
    // Normalized source game-folder path = the key into the remembered-workspace
    // map (backslashes→"/", no trailing slash — same normalization used for
    // workspace paths below and in doOpenSysGameFolder).
    const key = folder.replace(/\\/g, "/").replace(/\/$/, "");

    // Reuse a previously-chosen workspace if it still exists on disk, skipping
    // the picker entirely. A remembered path that's gone (drive unmounted, user
    // deleted it) falls through to the picker and re-records the new choice.
    // The picker is inside the try so a dialog/plugin error surfaces as a status
    // message instead of failing silently (the symptom that left the status
    // stuck on the scan line with no tabs + no error).
    let dest: string;
    const remembered = store.workspaceFolders[key];
    if (remembered && (await pathIsDir(remembered))) {
      dest = remembered;
    } else {
      const picked = await open({
        directory: true,
        title: "Choose a writable folder for the translation workspace",
      });
      if (typeof picked !== "string") {
        store.statusMessage = "Cancelled — no workspace chosen.";
        return;
      }
      dest = picked;
      // Remember it for next time. Reassign the whole object so Svelte's
      // reactivity picks up the new entry, then persist via the same flow App
      // uses for settings.json.
      store.workspaceFolders = { ...store.workspaceFolders, [key]: dest };
      void import("./settings").then((m) => m.persistAllSettings());
    }

    const norm = dest.replace(/\\/g, "/").replace(/\/$/, "");

    // selected (any category) + name tables for resolution. Dedup by filename so
    // a name file that's also somehow selected isn't materialized twice.
    const byName = new Map<string, SysBinFile>();
    for (const f of [...selected, ...all.filter((f) => f.category === "names")]) {
      if (!byName.has(f.filename)) byName.set(f.filename, f);
    }
    const provide = [...byName.values()];

    // Skip work already done: only copy/extract files NOT already in the
    // workspace (the win for re-loading a many-scene game across sessions).
    // loadPaths below still references ALL selected so they open whether they
    // were freshly written this run or left over from a previous one.
    let alreadyPresent = 0;
    const missing: SysBinFile[] = [];
    for (const f of provide) {
      if (await fileExists(`${norm}/${f.filename}`)) {
        alreadyPresent++;
      } else {
        missing.push(f);
      }
    }

    const looseSrcs = [...new Set(missing.filter((f) => f.path).map((f) => f.path))];
    const archivedRefs = missing
      .filter((f) => f.alf_offset !== undefined)
      .map((f) => ({
        archive_index: f.alf_archive_index!,
        offset: f.alf_offset!,
        length: f.alf_length!,
        name: f.filename,
      }));

    store.statusMessage = "Setting up workspace…";
    store.loadingProgress = { current: 0, total: 0, currentFile: "Extracting & copying to workspace…" };
    if (looseSrcs.length) await copyFilesToFolder(looseSrcs, dest);
    // Best-effort: returns the entries that actually extracted. A shortfall here
    // is reported (it's the difference between "selected" and "openable").
    let extracted = 0;
    if (archivedRefs.length) {
      const written = await extractAlfToFolder(folder, dest, archivedRefs);
      extracted = written.length;
    }
    store.loadingProgress = null; // doOpenSysBins shows its own per-file progress

    const loadPaths = selected.map((f) => `${norm}/${f.filename}`);
    // Count tabs actually opened so a per-file failure isn't masked by a blanket
    // "success" message (the old silent-failure behavior).
    const before = store.tabs.length;
    await doOpenSysBins(loadPaths);
    const opened = store.tabs.length - before;
    const wsName = norm.split("/").pop();
    const extractNote =
      archivedRefs.length > 0 && extracted < archivedRefs.length
        ? ` (extracted ${extracted} of ${archivedRefs.length} archived)`
        : "";

    if (opened === 0) {
      store.statusMessage =
        `Workspace ${wsName}: opened 0 of ${loadPaths.length} file(s)${extractNote}. ${store.statusMessage}`;
    } else {
      store.statusMessage =
        `Workspace ${wsName} — opened ${opened} of ${loadPaths.length} (${extracted} extracted, ${alreadyPresent} already present)${extractNote}.`;
    }
  } catch (err) {
    store.loadingProgress = null;
    store.statusMessage = `Workspace setup failed: ${err}`;
  }
}

/** Open the chosen folder BINs, each into its own tab. Reuses the single-file
 *  loader so speaker-name resolution (sibling-folder scan) works automatically. */
export async function doOpenSysBins(paths: string[]): Promise<void> {
  if (paths.length === 0) return;
  if (paths.length > 1) {
    store.loadingProgress = { current: 0, total: paths.length, currentFile: "" };
  }
  for (let i = 0; i < paths.length; i++) {
    const filename = paths[i].replace(/\\/g, "/").split("/").pop() || paths[i];
    if (store.loadingProgress) {
      store.loadingProgress = { current: i, total: paths.length, currentFile: filename };
    }
    await doLoadFile(paths[i]);
  }
  store.loadingProgress = null;
}

/** Load the chosen Parente scenes, each into its own tab. */
export async function doLoadParenteEntries(files: ParenteGameFile[]): Promise<void> {
  if (files.length > 1) {
    store.loadingProgress = { current: 0, total: files.length, currentFile: "" };
  }
  for (let i = 0; i < files.length; i++) {
    const f = files[i];
    if (store.loadingProgress) {
      store.loadingProgress = { current: i, total: files.length, currentFile: f.name };
    }
    try {
      const load = await parenteLoadGameEntry(f);
      const tab = store.createTab();
      tab.format = "parente";
      tab.parenteArcPath = f.arc_path;
      tab.parenteName = f.name;
      tab.parentePath = f.path;
      tab.parenteOffset = f.offset;
      tab.parenteSize = f.size;
      tab.entries = load.entries;
      tab.sourceBinFilename = load.name;
      tab.binPath = null;
      tab.projectPath = null; // no project persistence for parente tabs (yet)
      const speakers: Record<string, { source_type: number; source_raw_data: number; literal_name: string | null }> = {};
      const cast: { pointer: number; name: string }[] = [];
      const seenNames = new Set<string>();
      for (const [ordinal, name] of load.speakers) {
        speakers[String(ordinal)] = { source_type: 0, source_raw_data: 0, literal_name: name };
        // Build a per-scene cast list for the Characters panel. Parente has no
        // numeric character IDs — speakers come from `-- Name --` comments — so
        // we list the distinct speaker names (pointer is just a display index).
        if (name && !seenNames.has(name)) {
          seenNames.add(name);
          cast.push({ pointer: cast.length + 1, name });
        }
      }
      tab.rowSpeakers = speakers;
      tab.characters = cast;
      // Restore any previously-saved translations for this scene, then mark
      // clean (loading + restoring isn't an unsaved edit).
      await applyParenteProject(tab);
      tab.isDirty = false;
    } catch (err) {
      store.statusMessage = `Failed to load ${f.name}: ${err}`;
    }
  }
  store.loadingProgress = null;
  store.statusMessage = `Loaded ${files.length} scene file(s)`;
}

/** Export all Parente tabs as one FULL, NON-DESTRUCTIVE English ver999 patch:
 *  English is written into the engine's slot-2 language field (Japanese stays in
 *  slot 1), with text()/LangArray patched so the game renders English where
 *  present and falls back to Japanese otherwise. Rebuilds the current official
 *  patch archive into a complete ver999.arc — drop it in Patch and move the
 *  official ver10x archives out. */
export async function doExportParenteTab(_tab: TabState): Promise<boolean> {
  const tabs = store.tabs.filter((t) => t.format === "parente" && t.parenteArcPath);
  const files: ParentePatchFile[] = tabs
    .map((t) => ({
      arc_path: t.parenteArcPath!,
      name: t.parenteName,
      path: t.parentePath,
      offset: t.parenteOffset,
      size: t.parenteSize,
      translations: t.entries
        .filter((e) => e.translated_text.trim() !== "")
        .map((e) => [Number(e.instruction_index), e.translated_text] as [number, string]),
    }))
    .filter((f) => f.translations.length > 0);

  if (files.length === 0) {
    store.statusMessage = "Nothing translated to export yet.";
    return false;
  }

  // The full patch is built by rebuilding the highest official patch archive
  // present (the one in a "Patch" folder with the highest filename). The user
  // must keep the official patch installed when translating so scenes resolve
  // from it (and so we can rebuild it here).
  const norm = (p: string) => p.replace(/\\/g, "/");
  const patchSources = [...new Set(tabs.map((t) => norm(t.parenteArcPath!)))]
    .filter((p) => /\/patch\//i.test(p))
    .sort();
  const primaryArc = patchSources[patchSources.length - 1];
  if (!primaryArc) {
    store.statusMessage =
      "No official patch found to build on. Restore ver103_x1.arc in Start\\Archives\\Patch, reload via the game folder, then export.";
    return false;
  }

  // Default the save dialog to a WRITABLE location, not the game's
  // admin-protected Archives folder. Prefer the last folder the user exported
  // to; otherwise just a bare filename (dialog opens in its writable default).
  const defaultPath = store.lastParenteExportDir
    ? `${store.lastParenteExportDir}/ver999.arc`
    : "ver999.arc";
  const outputPath = await save({
    filters: [{ name: "Eushully Archive", extensions: ["arc"] }],
    defaultPath,
  });
  if (!outputPath) return false;

  // Remember the chosen folder for next time.
  const outDir = outputPath.replace(/\\/g, "/").replace(/\/[^/]*$/, "");
  if (outDir) {
    store.lastParenteExportDir = outDir;
    void import("./settings").then((m) => m.persistAllSettingsDebounced());
  }

  try {
    store.statusMessage = `Building English patch from ${primaryArc.split("/").pop()}…`;
    const changed = await parenteExportEnglish(primaryArc, files, outputPath);
    for (const t of tabs) t.isDirty = false;
    const outName = outputPath.replace(/\\/g, "/").split("/").pop();
    store.statusMessage =
      `Wrote ${outName} (${changed} scene${changed === 1 ? "" : "s"}; English in slot 2, JP fallback, ` +
      `in-game language toggle enabled). Install: in Start\\Archives\\Patch, rename EVERY official ` +
      `ver10x_x1.arc to .bak, leaving only ${outName}. In game: Config → language button (persists).`;
    return true;
  } catch (err) {
    store.statusMessage = `Error building patch: ${err}`;
    return false;
  }
}

/** Remove a path from Recent Files (e.g. user noticed it's stale). */
export function forgetRecentFile(filePath: string): void {
  store.recentFiles = store.recentFiles.filter((p) => p !== filePath);
  void import("./settings").then((m) => m.persistAllSettingsDebounced());
}

/** Wipe the entire Recent Files list. */
export function clearRecentFiles(): void {
  store.recentFiles = [];
  void import("./settings").then((m) => m.persistAllSettingsDebounced());
}

/** Batch-load project files from a folder, auto-matching BINs from another folder. */
export async function doLoadProjectFolders(): Promise<void> {
  // 1. Pick project folder
  const projectDir = await open({ directory: true, title: "Select folder containing project JSON files" });
  if (!projectDir) return;
  const projectDirPath = projectDir;

  // 2. Pick BIN folder
  const binDir = await open({ directory: true, title: "Select folder containing BIN files" });
  if (!binDir) return;
  const binDirPath = binDir;

  // 3. Scan both folders
  const projectFiles = (await readDir(projectDirPath))
    .filter((e) => !e.isDirectory && e.name?.toLowerCase().endsWith(".json"))
    .map((e) => e.name!);

  const binFiles = (await readDir(binDirPath))
    .filter((e) => !e.isDirectory && e.name?.toLowerCase().endsWith(".bin"))
    .map((e) => e.name!);

  if (projectFiles.length === 0) {
    store.statusMessage = "No .json project files found in selected folder";
    return;
  }

  // Build a case-insensitive lookup for BIN files
  const binLookup = new Map<string, string>();
  for (const name of binFiles) {
    binLookup.set(name.toLowerCase(), name);
  }

  // 4. Load each project, auto-match its BIN
  let loadedCount = 0;
  let skippedCount = 0;

  store.loadingProgress = { current: 0, total: projectFiles.length, currentFile: "" };

  for (let i = 0; i < projectFiles.length; i++) {
    const projectFilename = projectFiles[i];
    store.loadingProgress = { current: i, total: projectFiles.length, currentFile: projectFilename };
    try {
      const projectPath = `${projectDirPath}/${projectFilename}`;
      const project = await loadProject(projectPath);

      // Find matching BIN by source_bin_filename
      const matchedBin = binLookup.get(project.source_bin_filename.toLowerCase());
      if (!matchedBin) {
        store.statusMessage = `Skipped ${projectFilename} — BIN not found: ${project.source_bin_filename}`;
        skippedCount++;
        continue;
      }

      const binFilePath = `${binDirPath}/${matchedBin}`;
      const tab = store.createTab();

      const hashMatches = await loadBinForProject(tab.id, binFilePath, project.source_bin_sha256);
      fetchBridgeQuality(tab);
      if (!hashMatches) {
        const proceed = await confirmDialog({
          title: "Hash mismatch",
          confirmLabel: "Continue",
          message: `BIN hash mismatch for ${project.source_bin_filename}. Continue?`,
        });
        if (!proceed) {
          const tabIndex = store.tabs.indexOf(tab);
          if (tabIndex !== -1) store.closeTab(tabIndex);
          skippedCount++;
          continue;
        }
      }

      tab.entries = project.entries;
      tab.glossary = project.glossary;

      // Reconstruct _syntheticIndex links
      const negativeEntries = project.entries.filter((e) => e.instruction_index < 0);
      const usedIndices = new Set<number>();
      for (const ins of project.inserted_instructions) {
        const match = negativeEntries.find(
          (e) =>
            e.label === ins.label &&
            e.opcode === ins.opcode &&
            !usedIndices.has(e.instruction_index)
        );
        if (match) {
          ins._syntheticIndex = match.instruction_index;
          usedIndices.add(match.instruction_index);
        }
      }
      tab.insertedInstructions = project.inserted_instructions;
      tab.characters = project.characters || [];

      try {
        tab.rowSpeakers = await getRowSpeakers(tab.id);
      } catch {
        tab.rowSpeakers = {};
      }

      tab.binPath = binFilePath;
      tab.projectPath = projectPath;
      tab.sourceBinFilename = project.source_bin_filename;
      tab.sourceBinHash = project.source_bin_sha256;
      tab.sysVersion = project.sys_version || (project.is_ver5 ? 5 : 4);
      await applyEngineInfo(tab);
      tab.claudeModel = project.settings.claude_model;
      store.apiKeyConfigured = project.settings.api_key_configured;

      loadedCount++;
      store.statusMessage = `Loading projects... ${loadedCount} loaded`;
    } catch (err) {
      store.statusMessage = `Error loading ${projectFilename}: ${err}`;
      skippedCount++;
    }
  }

  store.loadingProgress = null;

  const parts: string[] = [`Loaded ${loadedCount} project${loadedCount !== 1 ? "s" : ""}`];
  if (skippedCount > 0) parts.push(`${skippedCount} skipped`);
  store.statusMessage = parts.join(", ");
}

/** Save all dirty tabs as projects to a single folder. Tabs with existing projectPath re-save in place. */
export async function doSaveAllProjects(): Promise<number> {
  const dirtyTabs = store.tabs.filter((t) => t.isDirty);
  if (dirtyTabs.length === 0) return 0;

  // Check if any tabs need a folder (no existing projectPath)
  const tabsNeedingFolder = dirtyTabs.filter((t) => !t.projectPath);
  let outputDir: string | null = null;

  if (tabsNeedingFolder.length > 0) {
    const selected = await open({ directory: true, title: "Select folder to save projects" });
    if (!selected) return 0;
    outputDir = selected;
  }

  let savedCount = 0;
  for (const tab of dirtyTabs) {
    try {
      const outPath = tab.projectPath || `${outputDir}/${tab.sourceBinFilename.replace(/\.[^.]+$/, ".json")}`;
      store.statusMessage = `Saving ${tab.sourceBinFilename}...`;
      await saveProject(
        tab.id,
        outPath,
        tab.entries,
        tab.glossary,
        tab.characters,
        resolveInsertionsForExport(tab.insertedInstructions, tab.entries),
        tab.claudeModel,
      );
      tab.projectPath = outPath;
      tab.isDirty = false;
      savedCount++;
    } catch (err) {
      store.statusMessage = `Error saving ${tab.sourceBinFilename}: ${err}`;
    }
  }

  store.statusMessage = `Saved ${savedCount} of ${dirtyTabs.length} projects`;
  return savedCount;
}

/** Export all open tabs as translated BINs to a single folder. */
export async function doExportAllBins(): Promise<number> {
  if (store.tabs.length === 0) return 0;

  const selected = await open({ directory: true, title: "Select folder to export BINs" });
  if (!selected) return 0;
  const outputDir = selected;

  let exportedCount = 0;
  for (const tab of store.tabs) {
    try {
      const outPath = `${outputDir}/${tab.sourceBinFilename.replace(/\.BIN$/i, "_translated.BIN")}`;
      store.statusMessage = `Exporting ${tab.sourceBinFilename}...`;
      // Pass FULL entries (incl. negative-index continuation rows): exportBin's
      // resolveInsertionsForExport needs them for insertion ordering (_visualOrder)
      // and continuation-row text sync. Rust's apply_changes skips negatives.
      await exportBin(tab.id, outPath, tab.entries, tab.insertedInstructions);
      exportedCount++;
    } catch (err) {
      store.statusMessage = `Error exporting ${tab.sourceBinFilename}: ${err}`;
    }
  }

  store.statusMessage = `Exported ${exportedCount} of ${store.tabs.length} BIN files`;
  return exportedCount;
}

/** Prompt user for path and export the current project as a translated BIN. Returns true if exported. */
/** Build the pre-export diff summary from a tab's in-memory state. Pure
 *  client-side — counts editable-row states and grabs a few sample entries
 *  for visual sanity-check. No Rust round-trip needed. */
function buildExportDiff(tab: TabState): import("./types").ExportDiff {
  const editable = new Set(["show-text", "display-furigana", "comment"]);
  let translated = 0, untranslated = 0, omitted = 0, deleted = 0;
  const changes: { id: number; label: string; original: string; translated: string }[] = [];
  for (const e of tab.entries) {
    if (e.deleted) { deleted++; continue; }
    // Omitted rows are intentionally left untouched on export — they keep their
    // original text. Count them ONLY as omitted; including them in
    // `untranslated` inflates the "original Japanese will be written" warning
    // with rows the user deliberately excluded.
    if (e.omit_export) { omitted++; continue; }
    if (!editable.has(e.label)) continue;
    // Match the tab-dot progress filter — bracket-only / placeholder rows
    // don't count toward "translated" or "untranslated" totals.
    if (!hasTranslatableContent(e.original_text)) continue;
    const tr = e.translated_text.trim();
    if (tr.length > 0) {
      translated++;
      // Collect ALL real changes (not a capped sample) for the review list.
      if (tr !== e.original_text) {
        changes.push({ id: e.instruction_index, label: e.label, original: e.original_text, translated: e.translated_text });
      }
    } else {
      untranslated++;
    }
  }
  return {
    translatedCount: translated,
    untranslatedCount: untranslated,
    insertedCount: tab.insertedInstructions.length,
    omittedCount: omitted,
    deletedCount: deleted,
    changes,
  };
}

export async function doExportBin(): Promise<boolean> {
  const tab = store.activeTab;
  if (!tab) return false;

  // Parente tabs export back to a rebuilt .arc, not a BIN.
  if (tab.format === "parente") {
    return doExportParenteTab(tab);
  }

  // Show the pre-export summary first. The user only sees the OS save dialog
  // after they confirm — saves a wasted click sequence when they accidentally
  // hit Export on the wrong tab.
  return new Promise<boolean>((resolve) => {
    store.pendingExportDiff = {
      tabId: tab.id,
      binFilename: tab.sourceBinFilename,
      diff: buildExportDiff(tab),
      onCancel: () => {
        store.pendingExportDiff = null;
        store.statusMessage = "Export cancelled";
        resolve(false);
      },
      onConfirm: async () => {
        store.pendingExportDiff = null;
        try {
          const outputPath = await save({
            filters: [{ name: "BIN Files", extensions: ["BIN", "bin"] }],
            defaultPath: tab.sourceBinFilename.replace(/\.BIN$/i, "_translated.BIN"),
          });
          if (!outputPath) { resolve(false); return; }

          store.statusMessage = "Exporting BIN...";
          // Full entries (incl. negative continuation rows) — see exportBin note;
          // resolveInsertionsForExport needs them for ordering + text sync.
          await exportBin(tab.id, outputPath, tab.entries, tab.insertedInstructions);
          // Count display-furigana entries that apply.rs converts to show-text
          // (prose translation, no "[reading]" suffix) — mirror its condition so
          // the status surfaces the change; entries kept as ruby are excluded.
          const furiganaFixed = tab.entries.filter((e) => {
            if (e.label !== "display-furigana" || e.deleted || e.is_context_row || e.omit_export) return false;
            const t = e.translated_text ?? "";
            if (t.trim() === "") return false;
            const hasReading = t.endsWith("]") && t.lastIndexOf(" [") !== -1;
            return !hasReading;
          }).length;
          const fileName = outputPath.replace(/\\/g, "/").split("/").pop();
          store.statusMessage = furiganaFixed > 0
            ? `Exported to ${fileName} — ${furiganaFixed} furigana line${furiganaFixed === 1 ? "" : "s"} rendered as normal text`
            : `Exported to ${fileName}`;
          resolve(true);
        } catch (err) {
          store.statusMessage = `Error exporting: ${err}`;
          resolve(false);
        }
      },
    };
  });
}

/**
 * Export all open BIN tabs as a single `.fanpatch` (a ZIP the version.dll loader
 * serves). Each BIN is built with the same corruption guardrail as a normal
 * export. Parente tabs are excluded (they rebuild a .arc, not a BIN).
 */
export async function doExportFanpatchFromTabs(): Promise<boolean> {
  const binTabs = store.tabs.filter((t) => t.format !== "parente" && t.entries.length > 0);
  if (binTabs.length === 0) {
    store.statusMessage = "No BIN tabs open to export";
    return false;
  }

  const outputPath = await save({
    filters: [{ name: "Fanpatch", extensions: ["fanpatch"] }],
    defaultPath: "01_english.fanpatch",
  });
  if (!outputPath) return false;

  store.statusMessage = `Building .fanpatch from ${binTabs.length} open BIN(s)…`;
  try {
    const result = await exportFanpatch(
      binTabs.map((t) => ({
        tabId: t.id,
        filename: t.sourceBinFilename,
        entries: t.entries,
        insertedInstructions: t.insertedInstructions,
      })),
      outputPath,
      store.fanpatchTitle,
      store.fanpatchLabel,
      store.fanpatchUiConfigPath
    );
    return reportFanpatchResult(result, outputPath);
  } catch (err) {
    store.statusMessage = `Error exporting fanpatch: ${err}`;
    return false;
  }
}

/**
 * Package an existing folder of translated BINs into a `.fanpatch` — no tabs
 * needed. The right tool for shipping a finished translation (zips the folder).
 */
export async function doPackFanpatchFromFolder(): Promise<boolean> {
  const sourceDir = await open({ directory: true, title: "Select folder of translated BINs" });
  if (!sourceDir || typeof sourceDir !== "string") return false;

  const outputPath = await save({
    filters: [{ name: "Fanpatch", extensions: ["fanpatch"] }],
    defaultPath: "01_english.fanpatch",
  });
  if (!outputPath) return false;

  store.statusMessage = "Packaging folder into .fanpatch…";
  try {
    const result = await packFanpatchFromDir(
      sourceDir,
      outputPath,
      store.fanpatchTitle,
      store.fanpatchLabel,
      store.fanpatchUiConfigPath
    );
    return reportFanpatchResult(result, outputPath);
  } catch (err) {
    store.statusMessage = `Error packaging fanpatch: ${err}`;
    return false;
  }
}

/**
 * Build a dual-language `.fanpatch`: pick a folder of translated (English) BINs +
 * a folder of the matching Japanese originals. `bilingual=false` repairs narrator
 * boxes only; `bilingual=true` also adds in-game mid-scene EN↔JP switching.
 */
export async function doPackDualLanguageFanpatch(bilingual: boolean): Promise<boolean> {
  const enDir = await open({ directory: true, title: "Select folder of TRANSLATED (English) BINs" });
  if (!enDir || typeof enDir !== "string") return false;

  const jpDir = await open({ directory: true, title: "Select folder of ORIGINAL (Japanese) BINs" });
  if (!jpDir || typeof jpDir !== "string") return false;

  // Optional: a folder of translated assets (AGF UI/CG images) to bundle for a full
  // UI+dialog swap. Cancel this dialog to skip (dialog-only patch).
  const agfPick = await open({ directory: true, title: "Optional: translated ASSET (AGF) folder — Cancel to skip" });
  const agfDir = typeof agfPick === "string" ? agfPick : "";

  const outputPath = await save({
    filters: [{ name: "Fanpatch", extensions: ["fanpatch"] }],
    defaultPath: "01_english.fanpatch",
  });
  if (!outputPath) return false;

  store.statusMessage = bilingual
    ? "Building bilingual .fanpatch (repair + mid-scene switching)…"
    : "Building repaired .fanpatch (speaker boxes)…";
  try {
    const result = await packDualLanguageFanpatch(
      enDir,
      jpDir,
      agfDir,
      bilingual,
      outputPath,
      store.fanpatchTitle,
      store.fanpatchLabel,
      store.fanpatchUiConfigPath
    );
    return reportFanpatchResult(result, outputPath);
  } catch (err) {
    store.statusMessage = `Error building dual-language fanpatch: ${err}`;
    return false;
  }
}

/** Shared status reporting for the two .fanpatch builders. */
function reportFanpatchResult(
  result: { written: number; skipped: string[]; loader_written: boolean },
  outputPath: string
): boolean {
  const name = outputPath.replace(/\\/g, "/").split("/").pop();
  if (result.skipped.length > 0) {
    console.warn("Fanpatch skipped entries:", result.skipped);
  }
  if (result.written === 0) {
    store.statusMessage = `Fanpatch empty — nothing exported (${result.skipped.length} skipped, see console)`;
    return false;
  }
  // version.dll + README are written alongside so it's a complete drop-in.
  const loader = result.loader_written ? " + version.dll" : " (⚠ version.dll not written — see console)";
  const skip = result.skipped.length > 0 ? ` — ${result.skipped.length} skipped (see console)` : "";
  store.statusMessage = `Exported ${result.written} BIN(s)${loader} to ${name}${skip}. Drop both into the game folder.`;
  return true;
}

/** Resolve a speaker display string for an entry, mirroring EditorRow's logic. */
function resolveSpeakerForRow(tab: TabState, instructionIndex: number): string | undefined {
  const ref = tab.rowSpeakers[String(instructionIndex)];
  if (!ref) return undefined;
  if (ref.literal_name) return ref.literal_name;
  const char = tab.characters.find((c) => c.pointer === ref.source_raw_data);
  if (char && char.name.length > 0) return char.name;
  return undefined;
}

/**
 * Build the TranslationRow list for a tab — same logic used by both single-tab
 * and multi-tab export. Extracted so the multi-tab loop doesn't duplicate.
 */
/** Returns true when a string contains only digits, symbols, or whitespace —
 *  nothing that needs translation. Examples: "12345", "...", "★★★", "(○_○)". */
function isNumberOrSymbolOnly(text: string): boolean {
  if (!text) return true;
  // Match if EVERY non-whitespace character is a digit or non-letter symbol.
  // \p{L} = any Unicode letter. If we find none, the string is symbols/numbers.
  return !/\p{L}/u.test(text);
}

function buildTranslationRowsForTab(tab: TabState, opts: { parallel?: boolean } = {}): TranslationRow[] {
  const TRANSLATABLE = new Set([
    "show-text",
    "display-furigana",
    "comment",
    "set-string",
    "draw-string",
  ]);
  const GROUPABLE = new Set(["show-text", "display-furigana"]);

  const groupJp = (e: typeof tab.entries[number]): string => {
    if (e.label === "display-furigana") {
      const m = e.original_text.match(/^(.+?)\s*\[(.+?)\]$/);
      if (m) return `${m[1]}〔${m[2]}〕`;
    }
    return e.original_text;
  };

  // Glossary exact-match lookup — pre-fills translated_text on rows whose
  // original matches a glossary key, so Claude skips them naturally.
  //
  // Leak prevention: in DIALOGUE/scene BINs, skip glossary entries whose English
  // looks like a UI abbreviation (攻撃力 → "ATK"). Those entries exist for stat
  // screens; auto-seeding them here would plant the abbreviation into prose where
  // "attack power" is wanted. Leaving them out lets Claude translate the line
  // fresh. UI/name BINs keep pre-filling normally — abbreviations are correct there.
  const dialogueBin = isDialogueBin(tab.sourceBinFilename);
  const glossaryMap = new Map<string, string>();
  for (const g of [...store.globalGlossary, ...tab.glossary]) {
    if (!g.japanese || !g.english) continue;
    if (dialogueBin && isUiAbbreviation(g.english)) continue;
    glossaryMap.set(g.japanese, g.english);
  }

  // First pass: build raw rows + assign block_id (bumped on each wait-for-input).
  const rawRows: TranslationRow[] = [];
  let blockId = 0;
  let i = 0;
  while (i < tab.entries.length) {
    const e = tab.entries[i];
    // wait-for-input bumps the block. Doesn't emit a row itself.
    if (e.label === "wait-for-input") {
      blockId++;
      i++;
      continue;
    }
    // Parente rows carry Lua block/function names (e.g. "block_00001", "text")
    // as their label, not AGE opcode labels — the TRANSLATABLE opcode gate would
    // drop every one. For Parente every non-deleted string is translatable; its
    // labels aren't GROUPABLE either, so each becomes an individual row below.
    const isTranslatableLabel = tab.format === "parente" || TRANSLATABLE.has(e.label);
    if (e.deleted || e.is_context_row || !isTranslatableLabel) { i++; continue; }
    // Skip rows the user explicitly marked omit_export. The intent is "don't
    // write this to the BIN at export time" → there's no value in asking
    // Claude to translate it, and including it in translation.json makes
    // the cross-folder Status dialog report a lower % than the editor (since
    // the editor's Stats panel filters omit rows out but translation.json
    // doesn't have an omit field). Keeping them out of the export keeps
    // those two views aligned.
    if (e.omit_export) { i++; continue; }
    if (!e.original_text || e.original_text.trim() === "") { i++; continue; }
    if (isStructuralOnly(e.original_text)) { i++; continue; }
    // Skip rows whose JP is just numbers/symbols — they don't need translating
    // and Claude often returns them unchanged anyway, wasting a turn.
    if (isNumberOrSymbolOnly(e.original_text)) { i++; continue; }

    if (GROUPABLE.has(e.label)) {
      const members: typeof tab.entries = [];
      let j = i;
      while (j < tab.entries.length) {
        const x = tab.entries[j];
        if (x.deleted) { j++; continue; }
        if (x.label === "end-text-line") { j++; break; }
        if (!GROUPABLE.has(x.label)) break;
        if (isStructuralOnly(x.original_text)) break;
        // Skip omitted members too — same reason as the outer filter; we
        // don't want a partially-omitted group to send the omitted member's
        // JP to Claude or to inflate the Status dialog's denominator.
        if (x.omit_export) { j++; continue; }
        members.push(x);
        j++;
      }
      if (members.length === 1) {
        const only = members[0];
        const original = only.original_text;
        const existing = only.translated_text ?? "";
        const fromGlossary = !existing.trim() ? glossaryMap.get(original) : undefined;
        rawRows.push({
          id: only.instruction_index,
          label: only.label,
          speaker: resolveSpeakerForRow(tab, only.instruction_index),
          original,
          translated: fromGlossary ?? existing,
          notes: only.notes ?? "",
          block_id: blockId,
        });
      } else {
        // Anchor on the first show-text member (never a display-furigana): the
        // full-line English must land on a 0x6E show-text so the engine renders
        // it as normal dialogue, not ruby. Non-anchor members — including the
        // furigana — get omit-emptied on import (see import_translations'
        // group_member_to_anchor pass). Fall back to members[0] when the group
        // has no show-text at all (lone-furigana lines: unchanged behaviour).
        const anchor = members.find((m) => m.label === "show-text") ?? members[0];
        const concatJp = members.map(groupJp).join("");
        const existing = anchor.translated_text ?? "";
        const fromGlossary = !existing.trim() ? glossaryMap.get(concatJp) : undefined;
        rawRows.push({
          id: anchor.instruction_index,
          label: anchor.label,
          speaker: resolveSpeakerForRow(tab, anchor.instruction_index),
          original: concatJp,
          translated: fromGlossary ?? existing,
          notes: anchor.notes ?? "",
          // Anchor id FIRST so import's skip(1) omit-empties the rest (incl.
          // the furigana); remaining members stay in original reading order.
          group_member_ids: [
            anchor.instruction_index,
            ...members.filter((m) => m !== anchor).map((m) => m.instruction_index),
          ],
          block_id: blockId,
        });
      }
      i = j;
    } else {
      const original = e.original_text;
      const existing = e.translated_text ?? "";
      const fromGlossary = !existing.trim() ? glossaryMap.get(original) : undefined;
      rawRows.push({
        id: e.instruction_index,
        label: e.label,
        speaker: resolveSpeakerForRow(tab, e.instruction_index),
        original,
        translated: fromGlossary ?? existing,
        notes: e.notes ?? "",
        block_id: blockId,
      });
      i++;
    }
  }

  // Second pass: dedup identical originals. First occurrence is the canonical;
  // later identical rows get duplicate_of pointing at the canonical's id. The
  // canonical translates once; on import the result is copied to all dups.
  const seenOriginal = new Map<string, number>();
  for (const row of rawRows) {
    // Don't dedup rows that already have a translation (preserve user edits)
    if (row.translated && row.translated.trim() !== "") continue;
    const canonical = seenOriginal.get(row.original);
    if (canonical !== undefined && canonical !== row.id) {
      row.duplicate_of = canonical;
    } else {
      seenOriginal.set(row.original, row.id);
    }
  }

  // Third pass: embed prior dialogue lines as context_before. Only done when
  // we're in parallel mode — sequential single-session has the whole file in
  // context already, so per-row duplication just wastes tokens. Depth 3 is
  // enough for voice/tone carry without ballooning the JSON.
  if (opts.parallel) {
  const CONTEXT_DEPTH = 3;
  for (let r = 0; r < rawRows.length; r++) {
    // Skip dup-of rows — they don't need context (canonical handles it)
    if (rawRows[r].duplicate_of !== undefined) continue;

    const context: string[] = [];
    for (let k = r - 1; k >= 0 && context.length < CONTEXT_DEPTH; k--) {
      const prev = rawRows[k];
      if (prev.duplicate_of !== undefined) continue; // skip dups in context
      const speaker = prev.speaker ? `[${prev.speaker}] ` : "";
      const text = prev.translated.trim() || prev.original;
      context.unshift(`${speaker}${text}`);
    }
    if (context.length > 0) {
      rawRows[r].context_before = context;
    }
  }
  } // end parallel-only context_before block

  // Speaker dedup was tried but reverted — Claude got confused when iterating
  // rows without speaker fields, leading to whole-file inspection patterns
  // and 32K output-token failures. The few KB saved wasn't worth the risk.
  // Speaker now stays on every row.

  // Duplicate stripping — keep the marker AND the original text (Claude may
  // inspect rows to understand the file). Only `translated` and metadata
  // stay empty since the canonical fills it at import.
  // (No-op pass kept here for clarity — rows already have correct shape.)

  return rawRows;
}

/**
 * Export the current tab's dialogue as translation.json + PROMPT.md to a
 * user-chosen directory, for offline translation via Claude Code.
 */
export async function doExportForTranslation(): Promise<boolean> {
  const tab = store.activeTab;
  if (!tab) {
    store.statusMessage = "No active tab.";
    return false;
  }
  if (tab.entries.length === 0) {
    store.statusMessage = "No entries to export.";
    return false;
  }

  try {
    const opts = await askExportOptions("Choose Claude model for this export", false);
    if (!opts) {
      store.statusMessage = "Export canceled.";
      return false;
    }
    const claudeModel = opts.model;

    const outputDir = await open({
      directory: true,
      multiple: false,
      title: "Choose a folder to write translation.json + PROMPT.md",
    });
    if (!outputDir || typeof outputDir !== "string") return false;

    // Single-tab export is always sequential — skip the heavy context_before.
    const rows = buildTranslationRowsForTab(tab, { parallel: false });

    if (rows.length === 0) {
      store.statusMessage = "No translatable entries found.";
      return false;
    }

    store.statusMessage = "Writing translation files...";
    // Parente tabs have no Rust-side parsed_bin/bridge — getSplitCapability
    // would error. No-split default matching SplitCapability's shape.
    const cap: SplitCapability =
      tab.format === "parente"
        ? { can_add_boxes: false, can_break_lines: false }
        : await getSplitCapability(tab.id);
    const writtenDir = await exportForTranslation(
      outputDir,
      tab.sourceBinFilename,
      tab.sourceBinHash ?? "",
      store.translatedLineCharCap,
      store.maxLinesPerBox,
      cap.can_add_boxes,
      cap.can_break_lines,
      [...store.globalGlossary, ...tab.glossary],
      rows,
      claudeModel,
    );
    store.statusMessage = `Wrote ${rows.length} entries to ${writtenDir} (model: ${claudeModel})`;
    return true;
  } catch (err) {
    store.statusMessage = `Export failed: ${err}`;
    return false;
  }
}

/**
 * Import a previously-exported translation.json (now filled in by Claude Code)
 * and merge translations into the current tab's entries.
 */
type ExportOpts = import("../components/ModelPickerDialog.svelte").ExportOptions;

/** Show the model-picker dialog and resolve with the user's choice (or null on cancel).
 *  Default model is the active tab's claudeModel, or the first available, or empty.
 *  `showParallel` enables the parallel-batch options (only useful for multi-tab export). */
function askExportOptions(title: string, showParallel: boolean): Promise<ExportOpts | null> {
  const tab = store.activeTab;
  const fallback = store.availableModels[0]?.id ?? "";
  const defaultModel = tab?.claudeModel || fallback;
  return new Promise((resolve) => {
    store.modelPickerRequest = { defaultModel, title, showParallel, resolve };
  });
}

/** Show the import-mode dialog and resolve with the user's choice (or null on cancel). */
function askImportMode(): Promise<"fill_missing" | "overwrite" | null> {
  return new Promise((resolve) => {
    store.importModeRequest = { resolve };
  });
}

/**
 * Bulk auto-split every splittable overflowing row in the ACTIVE tab — the
 * reusable core behind the Overflow dialog's "Split all" and the
 * auto-split-on-import option. Callers that target a non-active tab must set
 * `store.activeTabIndex` first (auto-split uses per-tab Rust state keyed by the
 * active tab). No-op when no per-line cap is set.
 *
 * Mirrors OverflowReviewDialog.handleAutoSplitAll: entry-text overflows go
 * through the Rust wrap (`autoSplitEntry` → `applySplitPlan`); group overflows
 * go through `splitOverflowingGroup` (idempotent per group).
 */
export async function bulkAutoSplitActiveTab(
  onProgress?: (current: number, total: number) => void,
): Promise<{ split: number; failed: number; errors: string[] }> {
  const out = { split: 0, failed: 0, errors: [] as string[] };
  const tab = store.activeTab;
  if (!tab || store.translatedLineCharCap <= 0) return out;

  const { findOverflows, entryDisplayLength } = await import("./overflow");
  const { applySplitPlan, splitOverflowingGroup } = await import("./insertions");

  const cap = store.translatedLineCharCap;
  const overflows = findOverflows(tab.entries, cap);
  const candidates = overflows.filter((r) => !r.isContinuation && r.splittable);

  // Partition overflows: entry-text overflows need a Rust wrap plan (one batch
  // round-trip); group overflows are frontend-only. Both apply paths below are
  // synchronous, so Svelte coalesces the inserts into a single table repaint —
  // the awaited per-row call was both N round-trips AND N repaints.
  const reqs: import("./ipc").SplitRequest[] = [];
  const groupRows: typeof candidates = [];
  for (const row of candidates) {
    const current = tab.entries.find((e) => e.instruction_index === row.id);
    if (!current) continue;
    if (row.individualLength > cap) {
      if (entryDisplayLength(current) <= cap) continue;
      reqs.push({ translated_text: current.translated_text, entry_index: row.id });
    } else if (row.partOfGroup) {
      groupRows.push(row);
    }
  }

  // Entry-text overflows: one round-trip for all plans, then apply synchronously.
  if (reqs.length > 0) {
    let outcomes: import("./ipc").SplitOutcome[];
    try {
      outcomes = await autoSplitEntriesBatch(tab.id, reqs, cap);
    } catch (err) {
      out.failed += reqs.length;
      if (out.errors.length < 3) out.errors.push(`batch: ${err}`);
      outcomes = [];
    }
    for (let i = 0; i < outcomes.length; i++) {
      onProgress?.(i + 1, outcomes.length);
      const o = outcomes[i];
      if (o.plan) {
        const r = applySplitPlan(o.plan);
        if (typeof r === "string") {
          out.failed++;
          if (out.errors.length < 3) out.errors.push(`#${o.entry_index}: ${r}`);
        } else {
          out.split++;
        }
      } else {
        out.failed++;
        if (o.error && out.errors.length < 3) out.errors.push(`#${o.entry_index}: ${o.error}`);
      }
    }
  }

  // Group overflows — frontend-only, idempotent ("not over the line cap" once handled).
  const groupsHandled = new Set<number>();
  for (const row of groupRows) {
    if (groupsHandled.has(row.id)) continue;
    const current = tab.entries.find((e) => e.instruction_index === row.id);
    if (!current) continue;
    const result = splitOverflowingGroup(current, cap);
    if (typeof result === "string") {
      if (!result.includes("not over the line cap")) {
        out.failed++;
        if (out.errors.length < 3) out.errors.push(`#${row.id}: ${result}`);
      }
    } else {
      out.split++;
      groupsHandled.add(row.id);
    }
  }

  if (out.split > 0) {
    tab.isDirty = true;
    tab.entries = [...tab.entries]; // single reactivity nudge → one repaint
  }
  return out;
}

/**
 * Run the line-split across EVERY open tab (the global version of
 * `bulkAutoSplitActiveTab`). Auto-split uses per-tab Rust state keyed by the
 * active tab, so we transiently switch the active tab to each, then restore.
 * Returns aggregated totals.
 */
export async function bulkAutoSplitAllTabs(
  onProgress?: (current: number, total: number, tabName: string) => void,
): Promise<{ split: number; failed: number; tabsProcessed: number; cancelled: boolean; errors: string[] }> {
  const agg = { split: 0, failed: 0, tabsProcessed: 0, cancelled: false, errors: [] as string[] };
  if (store.translatedLineCharCap <= 0) return agg;
  store.bulkCancelRequested = false;
  const prevActiveIndex = store.activeTabIndex;
  const tabs = [...store.tabs];
  for (let i = 0; i < tabs.length; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0)); // let a Stop click process between tabs
    if (store.bulkCancelRequested) { agg.cancelled = true; break; } // user stopped between tabs
    const t = tabs[i];
    const tabIndex = store.tabs.indexOf(t);
    if (tabIndex < 0) continue;
    store.activeTabIndex = tabIndex;
    onProgress?.(i + 1, tabs.length, t.sourceBinFilename || `Tab ${i + 1}`);
    const r = await bulkAutoSplitActiveTab();
    t.entries = [...t.entries]; // nudge reactivity after in-place inserts
    agg.split += r.split;
    agg.failed += r.failed;
    agg.tabsProcessed++;
    for (const e of r.errors) if (agg.errors.length < 5) agg.errors.push(`${t.sourceBinFilename}: ${e}`);
  }
  store.activeTabIndex = prevActiveIndex;
  return agg;
}

/**
 * Paginate every over-long box across ALL open tabs (the global version of the
 * Overflow dialog's per-tab "Paginate all"). For each tab, finds boxes that
 * exceed `maxLinesPerBox` and splits them into multiple boxes. Like the split
 * action, this switches the active tab transiently (paginateBox uses per-tab
 * state) and restores it. Returns aggregated totals.
 */
export async function bulkPaginateActiveTab(): Promise<{ paginated: number; errors: string[] }> {
  const out = { paginated: 0, errors: [] as string[] };
  const tab = store.activeTab;
  if (!tab || store.maxLinesPerBox < 1) return out;
  const { findBoxOverflows } = await import("./overflow");
  const { paginateBoxWithBridge } = await import("./insertions");
  const { getBridgeClone } = await import("./ipc");
  const boxes = findBoxOverflows(tab.entries, store.translatedCharCap, store.maxLinesPerBox)
    .filter((b) => b.tooManyLines)
    .sort((a, b) => b.index - a.index); // back-to-front so indices stay valid
  if (boxes.length === 0) return out;
  // Resolve the box-transition bridge ONCE (not per box), then paginate
  // synchronously (Svelte coalesces every insert into one repaint).
  let bridge: import("./types").SerializedInstruction[];
  const q = tab.bridgeTemplateQuality;
  if (q === "clean" || q === "degraded") {
    try {
      bridge = await getBridgeClone(tab.id);
    } catch (err) {
      out.errors.push(`No box bridge: ${err}`);
      return out;
    }
  } else {
    // This scene has no native bridge (e.g. MZ event scenes). Borrow a CALL-FREE
    // box-open from another open scene: strip control-flow (call 0x8F / jmp 0x8C /
    // jcc 0xA0) whose address args are scene-specific and would mis-resolve here.
    // The remaining box-state + box-open opcodes carry no addresses → safe to
    // clone. Verified in-game on MZ SC0740/SC0750 (clean box, no crash).
    const donor = store.tabs.find(
      (t) => t.id !== tab.id && (t.bridgeTemplateQuality === "clean" || t.bridgeTemplateQuality === "degraded"),
    );
    if (!donor) {
      out.errors.push("No box bridge in this scene; open a scene that paginates in another tab to borrow one.");
      return out;
    }
    try {
      bridge = (await getBridgeClone(donor.id)).filter(
        (i) => i.opcode !== 0x8f && i.opcode !== 0x8c && i.opcode !== 0xa0,
      );
    } catch (err) {
      out.errors.push(`borrowed bridge: ${err}`);
      return out;
    }
    if (!bridge.some((i) => i.opcode === 0x1f5)) {
      out.errors.push("borrowed bridge had no box-open after stripping control-flow.");
      return out;
    }
  }
  for (const box of boxes) {
    const startEntry = tab.entries[box.index];
    if (!startEntry) continue;
    const r = paginateBoxWithBridge(startEntry, store.maxLinesPerBox, bridge);
    if (typeof r === "number" && r > 0) out.paginated++;
    else if (typeof r === "string" && !r.includes("already fits")) {
      if (out.errors.length < 5) out.errors.push(r);
    }
  }
  if (out.paginated > 0) tab.entries = [...tab.entries]; // single reactivity nudge
  return out;
}

export async function bulkPaginateAllTabs(
  onProgress?: (current: number, total: number, tabName: string) => void,
): Promise<{ paginated: number; tabsProcessed: number; cancelled: boolean; errors: string[] }> {
  const agg = { paginated: 0, tabsProcessed: 0, cancelled: false, errors: [] as string[] };
  if (store.maxLinesPerBox < 1) return agg;
  store.bulkCancelRequested = false;
  const prevActiveIndex = store.activeTabIndex;
  const tabs = [...store.tabs];
  for (let i = 0; i < tabs.length; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0)); // let a Stop click process between tabs
    if (store.bulkCancelRequested) { agg.cancelled = true; break; } // user stopped between tabs
    const t = tabs[i];
    const tabIndex = store.tabs.indexOf(t);
    if (tabIndex < 0) continue;
    store.activeTabIndex = tabIndex; // paginateBox operates on the active tab
    onProgress?.(i + 1, tabs.length, t.sourceBinFilename || `Tab ${i + 1}`);
    const r = await bulkPaginateActiveTab();
    agg.paginated += r.paginated;
    for (const e of r.errors) if (agg.errors.length < 5) agg.errors.push(`${t.sourceBinFilename}: ${e}`);
    agg.tabsProcessed++;
  }
  store.activeTabIndex = prevActiveIndex;
  return agg;
}

/**
 * Re-flow the ACTIVE tab to new text-box caps. Reverts every existing auto-split
 * beat back to its full English (via `revertAllAutoSplits`), then re-wraps at the
 * given per-line cap and re-paginates at the given lines-per-box. This rebuilds
 * the line/box layout for a corrected box geometry WITHOUT re-translating — the
 * fix for translations baked at the wrong caps (e.g. 50×3 when the game is 47×2).
 *
 * Sets the global caps as a side effect (the wrap/paginate steps read them).
 */
export async function reflowActiveTab(
  lineCharCap: number,
  maxLinesPerBox: number,
): Promise<{ reverted: number; split: number; paginated: number; errors: string[] }> {
  const { revertAllAutoSplits } = await import("./insertions");
  store.translatedLineCharCap = lineCharCap;
  store.maxLinesPerBox = maxLinesPerBox;
  const rev = revertAllAutoSplits();
  const wrap = await bulkAutoSplitActiveTab();
  const pag = await bulkPaginateActiveTab();
  return {
    reverted: rev.reverted,
    split: wrap.split,
    paginated: pag.paginated,
    errors: [...rev.errors, ...wrap.errors, ...pag.errors],
  };
}

/**
 * Re-flow EVERY open tab to new caps (all-tabs version of `reflowActiveTab`).
 * Switches the active tab transiently per tab (the revert/wrap/paginate steps
 * use per-tab Rust + store state keyed by the active tab) and restores it.
 */
export async function reflowAllTabs(
  lineCharCap: number,
  maxLinesPerBox: number,
  onProgress?: (current: number, total: number, tabName: string) => void,
): Promise<{ reverted: number; split: number; paginated: number; tabsProcessed: number; cancelled: boolean; errors: string[] }> {
  const agg = { reverted: 0, split: 0, paginated: 0, tabsProcessed: 0, cancelled: false, errors: [] as string[] };
  store.bulkCancelRequested = false;
  const prevActiveIndex = store.activeTabIndex;
  const tabs = [...store.tabs];
  for (let i = 0; i < tabs.length; i++) {
    // Yield to the MACROtask queue so a queued Stop click is processed before we
    // check the flag. A tab with nothing to do resolves via microtasks only,
    // which never lets the click through — that's why Stop looked dead. This also
    // lets the progress bar repaint between tabs.
    await new Promise((resolve) => setTimeout(resolve, 0));
    if (store.bulkCancelRequested) { agg.cancelled = true; break; } // user stopped between tabs
    const t = tabs[i];
    const tabIndex = store.tabs.indexOf(t);
    if (tabIndex < 0) continue;
    store.activeTabIndex = tabIndex;
    onProgress?.(i + 1, tabs.length, t.sourceBinFilename || `Tab ${i + 1}`);
    const r = await reflowActiveTab(lineCharCap, maxLinesPerBox);
    agg.reverted += r.reverted;
    agg.split += r.split;
    agg.paginated += r.paginated;
    agg.tabsProcessed++;
    for (const e of r.errors) if (agg.errors.length < 8) agg.errors.push(`${t.sourceBinFilename}: ${e}`);
  }
  store.activeTabIndex = prevActiveIndex;
  return agg;
}

export async function doImportTranslations(): Promise<boolean> {
  const tab = store.activeTab;
  if (!tab) {
    store.statusMessage = "No active tab.";
    return false;
  }

  try {
    const jsonPath = await open({
      multiple: false,
      title: "Select translation.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!jsonPath || typeof jsonPath !== "string") return false;

    const mode = await askImportMode();
    if (!mode) {
      store.statusMessage = "Import canceled.";
      return false;
    }

    store.statusMessage = "Importing translations...";
    const result = await importTranslations(jsonPath, tab.sourceBinHash, mode, tab.entries);

    if (result.hash_mismatch) {
      const expected = result.source_bin || "(unknown)";
      const current = tab.sourceBinFilename || "(none loaded)";
      const proceed = await confirmDialog({
        title: "BIN mismatch — proceed with caution",
        danger: true,
        confirmLabel: "Proceed anyway",
        message:
          `The translation file was exported from a different BIN.\n\n` +
          `  Translation file source: ${expected}\n` +
          `  Currently loaded BIN:    ${current}\n\n` +
          `Applying anyway risks corrupting unrelated rows. Proceed?`,
      });
      if (!proceed) {
        store.statusMessage = "Import canceled.";
        return false;
      }
    }

    tab.entries = result.entries;
    tab.isDirty = true;

    const parts: string[] = [`${result.applied} applied`];
    if (result.skipped_existing > 0) parts.push(`${result.skipped_existing} preserved`);
    if (result.unchanged > 0) parts.push(`${result.unchanged} unchanged`);
    if (result.unknown_ids > 0) parts.push(`${result.unknown_ids} unknown ids ignored`);
    store.statusMessage = `Import complete — ${parts.join(", ")}`;

    // Heads-up if the import was empty — the auto-omit pass was skipped on
    // purpose (otherwise it would've flagged every editable row as omit and
    // misrepresented the tab as "complete"). Make the no-op visible so the
    // user doesn't wonder why nothing happened.
    if (result.applied === 0) {
      const { toastWarn } = await import("./toast");
      toastWarn(
        "Import had no translations — auto-omit was skipped to avoid falsely marking rows as 'no export'.",
      );
    }

    // Auto-split on import (opt-in): wrap/box-split overflowing rows now so the
    // user doesn't have to open the Overflow dialog and split manually. Skipped
    // for Parente — box/line splitting is an AGE opcode concept (the Unity/Lua
    // engine wraps text itself), and bulkAutoSplitActiveTab hits Rust per-tab
    // state Parente tabs don't have, which would throw and mislabel a successful
    // import as failed.
    if (
      tab.format !== "parente" &&
      store.autoSplitOnImport &&
      store.translatedLineCharCap > 0 &&
      result.applied > 0
    ) {
      store.statusMessage = "Auto-splitting overflowing rows...";
      const r = await bulkAutoSplitActiveTab();
      tab.entries = [...tab.entries]; // nudge reactivity after in-place inserts
      const extra = r.failed > 0 ? `, ${r.failed} couldn't split` : "";
      store.statusMessage = `Import complete — ${parts.join(", ")} (auto-split ${r.split} row(s)${extra})`;
    }

    // Auto-show overflow review dialog if any rows still exceed the line cap.
    if (store.translatedLineCharCap > 0) {
      const { findOverflows } = await import("./overflow");
      const overflows = findOverflows(tab.entries, store.translatedLineCharCap);
      if (overflows.length > 0) {
        store.overflowReviewDialogOpen = true;
      }
    }

    // Nudge (never auto-applied): surface safe glossary substitutions for review.
    if (store.mergedGlossary.some((g) => g.english.trim())) {
      const { findGlossaryAutoApply } = await import("./glossaryAutoApply");
      const candidates = findGlossaryAutoApply(tab.entries, store.mergedGlossary);
      if (candidates.length > 0) {
        const { toastInfo } = await import("./toast");
        toastInfo(
          `${candidates.length} glossary auto-fix${candidates.length === 1 ? "" : "es"} available — open "Glossary auto-apply" to review.`,
        );
      }
    }

    return true;
  } catch (err) {
    store.statusMessage = `Import failed: ${err}`;
    return false;
  }
}

/**
 * Sanitize a BIN filename into a safe subfolder name (strip extension, replace
 * any path-unsafe OR shell-significant character). Example: "SC0000.BIN" →
 * "SC0000", "$1$SC4000.BIN" → "_1_SC4000".
 *
 * `$` (and other shell-meta chars) MUST be stripped: Eushully append BINs use
 * the `$N$<NAME>.BIN` convention, and an unescaped `$` in the folder name makes
 * the generated `.sh` launchers and Claude's own path handling expand `$1$...`
 * to nothing → broken `cd`/paths. Import round-trips safely regardless because
 * matching is by `source_bin_sha256` (hash) and the real filename is preserved
 * inside translation.json's `source_bin_filename` — the folder name is purely
 * an on-disk label, so sanitizing it needs no reverse map.
 */
function binNameToFolderName(filename: string): string {
  const base = filename.replace(/\.[^.]+$/, "");
  // Path-unsafe chars plus shell-significant chars ($ ` ! & ; ( ) < > ' " space).
  return base.replace(/[\\/:*?"<>|$`!&;()'\s]/g, "_");
}

/**
 * Export every loaded tab's dialogue as a CC Translate project: one subfolder
 * per tab inside the chosen parent directory, plus root-level
 * `translate-all.bat` / `translate-all.sh` for batch translation.
 */
export async function doExportAllTabsForTranslation(): Promise<boolean> {
  const tabs = store.tabs.filter(
    (t) => t.entries.length > 0 && (!!t.binPath || t.format === "parente"),
  );
  if (tabs.length === 0) {
    store.statusMessage = "No tabs with translatable data to export.";
    return false;
  }

  try {
    const opts = await askExportOptions("Choose Claude model + batch options", true);
    if (!opts) {
      store.statusMessage = "Export canceled.";
      return false;
    }
    const claudeModel = opts.model;

    const parentDir = await open({
      directory: true,
      multiple: false,
      title: "Choose a parent folder for the translation project (one subfolder per BIN)",
    });
    if (!parentDir || typeof parentDir !== "string") return false;

    // Phase 1: build rows for each tab without writing yet. We need the full
    // set in memory to run the cross-folder dedup pass before any folder is
    // written — otherwise the first folder's translation.json wouldn't yet
    // know it's the canonical for "X" that appears in folder 3.
    const tabBundles: { tab: TabState; folderName: string; rows: TranslationRow[] }[] = [];
    let skipped = 0;
    for (let idx = 0; idx < tabs.length; idx++) {
      const tab = tabs[idx];
      const folderName = binNameToFolderName(tab.sourceBinFilename);
      store.statusMessage = `Preparing ${idx + 1}/${tabs.length}: ${tab.sourceBinFilename}...`;
      const rows = buildTranslationRowsForTab(tab, { parallel: opts.parallel });
      if (rows.length === 0) { skipped++; continue; }
      tabBundles.push({ tab, folderName, rows });
    }

    // Order name/item (CVINIT/…) bundles FIRST so the batch translates the
    // character/item name tables before any scene — scenes then inherit the
    // final translated names (and the cross-folder dedup below picks a name
    // file as canonical for any shared string). Stable: keeps original tab
    // order within each group.
    tabBundles.sort((a, b) => {
      const an = isNameItemFile(a.tab.sourceBinFilename) ? 0 : 1;
      const bn = isNameItemFile(b.tab.sourceBinFilename) ? 0 : 1;
      return an - bn;
    });

    // Phase 2: cross-folder dedup + translation-memory pre-fill.
    // For every unique `original` text across the export bundle:
    //   - Pick a canonical row: prefer any occurrence that already has a
    //     translation (so Claude doesn't redo your in-editor work); else
    //     first-seen.
    //   - Every other occurrence gets `external_duplicate_of` pointing at the
    //     canonical → translate.py skips them, no Claude tokens.
    //   - The canonical inherits the best-available translation we found,
    //     even if it didn't originally have one (translation-memory pre-fill).
    // Skips entries that are already in-folder duplicates (those use the
    // existing `duplicate_of` mechanism within a single translation.json).
    // Short lines (e.g. "ええ", "そうか") can mean different things depending on
    // which dialog block they're in. Matching by raw `original` alone would
    // wrongly collapse those across folders. For short originals we add a
    // context signature (speaker + prior line in same block_id) to the dedup
    // key. Long originals (≥SHORT_THRESHOLD) match on text alone — they're
    // unique enough that context disambiguation isn't needed.
    const SHORT_THRESHOLD = 15;

    /** Build a context signature for a row within its tab's row list:
     *  speaker + previous translatable row's original in the same block_id. */
    function rowContextKey(tabRowsList: TranslationRow[], idx: number): string {
      const r = tabRowsList[idx];
      let prior = "";
      for (let j = idx - 1; j >= 0; j--) {
        if ((tabRowsList[j].block_id ?? 0) !== (r.block_id ?? 0)) break;
        if (!tabRowsList[j].original) continue;
        prior = tabRowsList[j].original;
        break;
      }
      return `${r.speaker ?? ""}::${prior}`;
    }

    /** Dedup key: short originals carry context; long ones use raw text. */
    function dedupKey(tabRowsList: TranslationRow[], idx: number): string {
      const r = tabRowsList[idx];
      if (r.original.length >= SHORT_THRESHOLD) return r.original;
      return `${r.original}::${rowContextKey(tabRowsList, idx)}`;
    }

    interface CanonRef { folderName: string; id: number; translated?: string }
    const canonicalByKey = new Map<string, CanonRef>();
    for (const { folderName, rows } of tabBundles) {
      for (let i = 0; i < rows.length; i++) {
        const r = rows[i];
        if (!r.original || r.duplicate_of != null) continue;
        const key = dedupKey(rows, i);
        const existing = canonicalByKey.get(key);
        const myTrans = r.translated.trim();
        if (!existing) {
          canonicalByKey.set(key, { folderName, id: r.id, translated: myTrans || undefined });
        } else if (!existing.translated && myTrans) {
          // Upgrade canonical: this occurrence has a translation, prior didn't.
          canonicalByKey.set(key, { folderName, id: r.id, translated: myTrans });
        }
      }
    }
    // Apply: mark non-canonical occurrences as external duplicates; backfill
    // canonical rows with the chosen translation when they didn't have one.
    let externalDupCount = 0;
    let preFilledCount = 0;
    for (const { folderName, rows } of tabBundles) {
      for (let i = 0; i < rows.length; i++) {
        const r = rows[i];
        if (!r.original || r.duplicate_of != null) continue;
        const key = dedupKey(rows, i);
        const canon = canonicalByKey.get(key);
        if (!canon) continue;
        if (canon.folderName === folderName && canon.id === r.id) {
          if (!r.translated.trim() && canon.translated) {
            r.translated = canon.translated;
            preFilledCount++;
          }
        } else {
          r.external_duplicate_of = { folder: canon.folderName, id: canon.id };
          // Clear the duplicate's translation — canonical is the source of truth
          // and will overwrite at import time anyway.
          r.translated = "";
          externalDupCount++;
        }
      }
    }

    // Build the cross-tab glossary union once and pass it to every folder.
    // The proper-noun translations a user added in one BIN's project glossary
    // apply equally to sibling BINs in the same game — without the union
    // Claude has to re-derive them per folder.
    const mergedGlossary = (() => {
      const seen = new Map<string, import("./types").GlossaryEntry>();
      for (const g of store.globalGlossary) {
        if (g.japanese && !seen.has(g.japanese)) seen.set(g.japanese, g);
      }
      for (const { tab } of tabBundles) {
        for (const g of tab.glossary) {
          if (g.japanese && !seen.has(g.japanese)) seen.set(g.japanese, g);
        }
      }
      return [...seen.values()];
    })();

    // Phase 3: write each folder. If a folder already has a translation.json
    // from a prior export, preserve any non-empty translations whose original
    // text still matches — re-export delta mode. The user iterates (edit a
    // few rows, re-export) without Claude redoing the 95% that didn't change.
    let exported = 0;
    let deltaPreserved = 0;
    const folderNames: string[] = [];
    const subfolderPaths: string[] = [];
    for (let i = 0; i < tabBundles.length; i++) {
      const { tab, folderName, rows } = tabBundles[i];
      const subPath = `${parentDir}/${folderName}`;
      store.statusMessage = `Writing ${i + 1}/${tabBundles.length}: ${tab.sourceBinFilename}...`;

      await ensureDirectory(subPath);

      // Re-export delta: pull prior translations for unchanged originals.
      try {
        const priorMap = await readPriorTranslationMap(subPath);
        if (Object.keys(priorMap).length > 0) {
          for (const r of rows) {
            if (r.translated.trim().length > 0) continue;
            if (r.external_duplicate_of) continue; // resolved at import
            const prior = priorMap[r.original];
            if (prior) {
              r.translated = prior;
              deltaPreserved++;
            }
          }
        }
      } catch {
        // Non-fatal — proceed with fresh export.
      }

      // Parente tabs have no Rust-side parsed_bin/bridge — getSplitCapability
      // would error. They also can't grow boxes or break lines the AGE way, so
      // use a no-split default matching SplitCapability's shape.
      const cap: SplitCapability =
        tab.format === "parente"
          ? { can_add_boxes: false, can_break_lines: false }
          : await getSplitCapability(tab.id);
      await exportForTranslation(
        subPath,
        tab.sourceBinFilename,
        tab.sourceBinHash ?? "",
        store.translatedLineCharCap,
        store.maxLinesPerBox,
        cap.can_add_boxes,
        cap.can_break_lines,
        mergedGlossary,
        rows,
        claudeModel,
      );
      folderNames.push(folderName);
      subfolderPaths.push(subPath);
      exported++;
    }

    if (exported === 0) {
      store.statusMessage = "Nothing exported (no translatable entries in any tab).";
      return false;
    }

    // Root-level launchers. Always generates sequential translate-all.*;
    // also generates translate-parallel.* when the user chose parallel mode.
    try {
      await writeMultiTranslateLaunchers(
        parentDir, folderNames, claudeModel, opts.parallel, opts.parallelLimit,
      );
    } catch (err) {
      console.warn("Failed to write root launchers (non-fatal):", err);
    }

    const tail = skipped > 0 ? `, ${skipped} skipped (empty)` : "";
    const mode = opts.parallel ? `parallel ×${opts.parallelLimit}` : "sequential";
    const savings: string[] = [];
    if (externalDupCount > 0) savings.push(`${externalDupCount} cross-folder duplicates`);
    if (preFilledCount > 0) savings.push(`${preFilledCount} pre-filled from memory`);
    if (deltaPreserved > 0) savings.push(`${deltaPreserved} carried over from prior export`);
    const savingsNote = savings.length > 0 ? ` — saved ${savings.join(", ")}` : "";
    store.statusMessage = `Exported ${exported} BIN${exported === 1 ? "" : "s"} to ${parentDir} (model: ${claudeModel}, ${mode})${tail}${savingsNote}`;
    return true;
  } catch (err) {
    store.statusMessage = `Multi-export failed: ${err}`;
    return false;
  }
}

/**
 * Import every translation.json under a chosen parent folder. Matches each
 * subfolder's source_bin_sha256 against currently-loaded tabs and applies the
 * import. Per-folder stats reported in a single status line.
 */
export async function doImportAllTranslations(): Promise<boolean> {
  if (store.tabs.length === 0) {
    store.statusMessage = "No tabs loaded — open the BINs you want to import into first.";
    return false;
  }

  try {
    const parentDir = await open({
      directory: true,
      multiple: false,
      title: "Choose the parent folder containing translation subfolders",
    });
    if (!parentDir || typeof parentDir !== "string") return false;

    const folders = await scanTranslationProjectStatus(parentDir);
    if (folders.length === 0) {
      store.statusMessage = "No translation.json files found under that folder.";
      return false;
    }

    const mode = await askImportMode();
    if (!mode) {
      store.statusMessage = "Import canceled.";
      return false;
    }

    let totalApplied = 0;
    let imported = 0;
    let unmatched = 0;
    const unmatchedNames: string[] = [];
    const importedTabs: TabState[] = [];

    for (let idx = 0; idx < folders.length; idx++) {
      const folder = folders[idx];
      store.statusMessage = `Importing ${idx + 1}/${folders.length}: ${folder.folder}...`;
      if (folder.error) {
        unmatched++;
        unmatchedNames.push(`${folder.folder} (${folder.error})`);
        continue;
      }
      // Find a tab matching this folder's source BIN hash
      const matchingTab = store.tabs.find((t) => t.sourceBinHash === folder.source_bin_sha256);
      if (!matchingTab) {
        unmatched++;
        unmatchedNames.push(folder.folder);
        continue;
      }

      const jsonPath = `${parentDir}/${folder.folder}/translation.json`;
      try {
        const result = await importTranslations(
          jsonPath,
          matchingTab.sourceBinHash,
          mode,
          matchingTab.entries,
        );
        matchingTab.entries = result.entries;
        matchingTab.isDirty = true;
        totalApplied += result.applied;
        imported++;
        if (result.applied > 0) importedTabs.push(matchingTab);
      } catch (err) {
        unmatched++;
        unmatchedNames.push(`${folder.folder} (${err})`);
      }
    }

    // Auto-split on import (opt-in): run the bulk wrap/box-split on each tab we
    // imported into. auto-split uses per-tab Rust state keyed by the active tab,
    // so we transiently switch the active tab to each, then restore.
    let autoSplitTotal = 0;
    if (store.autoSplitOnImport && store.translatedLineCharCap > 0 && importedTabs.length > 0) {
      const prevActiveIndex = store.activeTabIndex;
      for (let i = 0; i < importedTabs.length; i++) {
        const t = importedTabs[i];
        // Splitting is AGE-only (Parente/Lua wraps its own text and has no
        // per-tab Rust bridge state) — skip so the batch doesn't throw.
        if (t.format === "parente") continue;
        const tabIndex = store.tabs.indexOf(t);
        if (tabIndex < 0) continue;
        store.activeTabIndex = tabIndex;
        store.statusMessage = `Auto-splitting ${i + 1}/${importedTabs.length}: ${t.sourceBinFilename}...`;
        const r = await bulkAutoSplitActiveTab();
        t.entries = [...t.entries]; // nudge reactivity after in-place inserts
        autoSplitTotal += r.split;
      }
      store.activeTabIndex = prevActiveIndex;
    }

    // Stale status cache for this folder — we just changed translations.
    invalidateTranslationStatusCache(parentDir);

    const parts: string[] = [`${imported}/${folders.length} imported`, `${totalApplied} translations applied`];
    if (autoSplitTotal > 0) parts.push(`${autoSplitTotal} rows auto-split`);
    if (unmatched > 0) parts.push(`${unmatched} unmatched`);
    let msg = `Multi-import complete — ${parts.join(", ")}`;
    if (unmatchedNames.length > 0) {
      msg += ` (unmatched: ${unmatchedNames.slice(0, 3).join(", ")}${unmatchedNames.length > 3 ? "…" : ""})`;
    }
    store.statusMessage = msg;
    return true;
  } catch (err) {
    store.statusMessage = `Multi-import failed: ${err}`;
    return false;
  }
}

/**
 * Import a single folder's translation.json from the status dialog — exact
 * parity with the manual "Import translations" flow (same askImportMode prompt),
 * just without making the user close the dialog and re-pick the file.
 *
 * Matches the folder's source BIN hash against a currently-loaded tab. Returns
 * the number of translations applied, or throws with a user-facing message.
 */
export async function doImportSingleFolder(
  parentDir: string,
  folder: TranslationFolderStatus,
): Promise<number> {
  if (folder.error) throw new Error(folder.error);

  const matchingTab = store.tabs.find((t) => t.sourceBinHash === folder.source_bin_sha256);
  if (!matchingTab) {
    throw new Error(`No open tab matches ${folder.source_bin || folder.folder}. Open that BIN first.`);
  }

  // Same prompt the manual import shows — let the user choose fill vs overwrite.
  const mode = await askImportMode();
  if (!mode) throw new Error("canceled");

  store.statusMessage = `Importing ${folder.folder}...`;
  const jsonPath = `${parentDir}/${folder.folder}/translation.json`;
  const result = await importTranslations(
    jsonPath,
    matchingTab.sourceBinHash,
    mode,
    matchingTab.entries,
  );
  matchingTab.entries = result.entries;
  matchingTab.isDirty = true;

  // We just changed translations on disk's source-of-truth tab — the cached
  // scan for this parent is now stale.
  invalidateTranslationStatusCache(parentDir);

  store.statusMessage = `Imported ${folder.folder} — ${result.applied} translation${result.applied === 1 ? "" : "s"} applied`;
  return result.applied;
}

/** In-memory cache for the status dialog. Keyed by parentDir + cached timestamp.
 *  Invalidated on multi-import or after 30s (whichever comes first). */
const statusCache = new Map<string, { folders: TranslationFolderStatus[]; ts: number }>();
const STATUS_CACHE_TTL_MS = 30_000;

/** Invalidate the status cache for a given parent folder (or all if undefined). */
export function invalidateTranslationStatusCache(parentDir?: string): void {
  if (parentDir) statusCache.delete(parentDir);
  else statusCache.clear();
}

/**
 * Pick a translation project folder, scan it for per-subfolder progress, and
 * open the status dialog. Used by the "CC Translate → Check progress" menu.
 * Cached for STATUS_CACHE_TTL_MS to make re-open instant.
 */
export async function doCheckTranslationStatus(): Promise<boolean> {
  try {
    const parentDir = await open({
      directory: true,
      multiple: false,
      title: "Choose a translation project folder to check progress",
    });
    if (!parentDir || typeof parentDir !== "string") return false;

    let folders: TranslationFolderStatus[];
    const cached = statusCache.get(parentDir);
    if (cached && Date.now() - cached.ts < STATUS_CACHE_TTL_MS) {
      folders = cached.folders;
    } else {
      store.statusMessage = "Scanning translation folders...";
      folders = await scanTranslationProjectStatus(parentDir);
      statusCache.set(parentDir, { folders, ts: Date.now() });
    }

    if (folders.length === 0) {
      store.statusMessage = "No translation subfolders found at that location.";
      return false;
    }
    store.translationStatusReport = { parentDir, folders };
    const done = folders.filter((f) => !f.error && f.total > 0 && f.translated === f.total).length;
    const ageNote = cached && Date.now() - cached.ts < STATUS_CACHE_TTL_MS
      ? " (cached)" : "";
    store.statusMessage = `${folders.length} folder${folders.length === 1 ? "" : "s"} scanned — ${done} complete${ageNote}`;
    return true;
  } catch (err) {
    store.statusMessage = `Status scan failed: ${err}`;
    return false;
  }
}

/**
 * Re-scan the currently-open translation status report, bypassing the cache.
 * Used by the Refresh button after the user relaunches a folder so the new
 * progress numbers show up without having to close and reopen the dialog.
 */
export async function doRefreshTranslationStatus(): Promise<void> {
  const report = store.translationStatusReport;
  if (!report) return;
  try {
    store.statusMessage = "Rescanning translation folders...";
    const folders = await scanTranslationProjectStatus(report.parentDir);
    statusCache.set(report.parentDir, { folders, ts: Date.now() });
    store.translationStatusReport = { parentDir: report.parentDir, folders };
    const done = folders.filter((f) => !f.error && f.total > 0 && f.translated === f.total).length;
    store.statusMessage = `Rescanned — ${done}/${folders.length} folders complete`;
  } catch (err) {
    store.statusMessage = `Rescan failed: ${err}`;
  }
}
