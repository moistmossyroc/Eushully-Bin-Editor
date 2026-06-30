<script lang="ts">
  /**
   * SYS3/4/5 game-folder picker. Lists the script BINs found in a folder,
   * classified into five staged buckets (Character & item names / Story scenes /
   * Side·special scenes / UI & system / Other) and filterable, so the user can
   * pick what to translate without hunting for files. Names shows first
   * (translate it first so scenes inherit the final character names). Story
   * scenes are pre-selected. Opening from the game folder means the speaker-name
   * init files sit right beside the scenes, so names resolve automatically.
   */
  import type { SysBinFile } from "../lib/ipc";
  import { doOpenSysBins, doSetupWorkspaceAndOpen } from "../lib/actions";
  import { store } from "../lib/stores.svelte";
  import { onMount } from "svelte";

  interface Props {
    folder: string;
    files: SysBinFile[];
    /** False for admin-locked folders → open copies into a writable workspace. */
    writable: boolean;
    onClose: () => void;
  }
  let { folder, files, writable, onClose }: Props = $props();

  const CAT_LABEL: Record<string, string> = {
    names: "Character & item names (do first)",
    sc: "Story scenes",
    sp: "Side / special scenes",
    ui: "UI & system",
    other: "Other text (optional)",
  };
  const catLabel = (c: string) => CAT_LABEL[c] ?? "Other text (optional)";

  // Selection by array index. Story scenes are pre-selected (the common case)
  // on mount; the user can also check name files to translate first.
  let selected = $state<Set<number>>(new Set());
  onMount(() => {
    selected = new Set(
      files.map((f, i) => ({ f, i })).filter((x) => x.f.category === "sc").map((x) => x.i),
    );
  });
  let filter = $state("");
  let activeCats = $state<Set<string>>(new Set());
  let loading = $state(false);
  // Data/structural BINs (ALLMAP, ANIME, SAVE, MP*, …) have nothing to
  // translate; hide them by default. The user can reveal them with the toggle.
  let showEmpty = $state(false);

  /** A file is worth showing when it has counted translatable strings, OR it's
   *  archived (string_count is 0 because we don't parse .alf entries — "unknown"
   *  not "empty", so never hide those). Loose files with 0 count are the empties. */
  const hasContent = (f: SysBinFile) => f.string_count > 0 || f.alf_offset !== undefined;

  let translatedCount = $derived(files.filter((f) => f.likely_translated).length);
  let archivedCount = $derived(files.filter((f) => f.alf_offset !== undefined).length);
  let emptyCount = $derived(files.filter((f) => !hasContent(f)).length);
  // The pool currently in play (respects the empty toggle but not the text /
  // category filters) — drives the "All" chip + the "of N shown" counter.
  let visibleTotal = $derived(files.filter((f) => showEmpty || hasContent(f)).length);

  let catGroups = $derived.by(() => {
    const m = new Map<string, number>();
    for (const f of files) {
      if (!showEmpty && !hasContent(f)) continue; // skip hidden empties
      m.set(f.category, (m.get(f.category) ?? 0) + 1);
    }
    // Stable, meaningful order — names first (translate them first so scenes
    // inherit final names), then story / special scenes, then UI, then other.
    // Categories with no visible files drop out of the map → their chip hides.
    const order = ["names", "sc", "sp", "ui", "other"];
    return [...m.entries()].sort((a, b) => order.indexOf(a[0]) - order.indexOf(b[0]));
  });

  let shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return files
      .map((f, i) => ({ f, i }))
      .filter(({ f }) => {
        if (!showEmpty && !hasContent(f)) return false;
        if (activeCats.size > 0 && !activeCats.has(f.category)) return false;
        if (q && !f.base_name.toLowerCase().includes(q)) return false;
        return true;
      });
  });

  let allShownSelected = $derived(shown.length > 0 && shown.every(({ i }) => selected.has(i)));
  let someShownSelected = $derived(shown.some(({ i }) => selected.has(i)));

  function toggle(i: number) {
    if (selected.has(i)) selected.delete(i);
    else selected.add(i);
    selected = new Set(selected);
  }
  function toggleCat(cat: string) {
    if (activeCats.has(cat)) activeCats.delete(cat);
    else activeCats.add(cat);
    activeCats = new Set(activeCats);
  }
  function clearCats() {
    activeCats = new Set();
  }
  function toggleSelectAllShown() {
    if (allShownSelected) {
      for (const { i } of shown) selected.delete(i);
    } else {
      for (const { i } of shown) selected.add(i);
    }
    selected = new Set(selected);
  }
  function clearSel() {
    selected = new Set();
  }

  async function load() {
    if (loading) return;
    const chosen = [...selected].map((i) => files[i]).filter(Boolean) as SysBinFile[];
    if (chosen.length === 0) {
      store.statusMessage = `No files selected (selection had ${selected.size} entr${selected.size === 1 ? "y" : "ies"} but none resolved).`;
      return;
    }
    // Capture props BEFORE onClose() nulls store.sysBinFolderRequest — otherwise
    // the reactive props fall back to their App-side `?? ""/[]` defaults and we'd
    // pass an EMPTY folder to the loader (→ "read SYS4INI.BIN: not found").
    const folderPath = folder;
    const allFiles = files;
    const isWritable = writable;
    loading = true;
    // Dismiss first so the loading-progress overlay (and the workspace folder
    // picker) shows on top.
    onClose();
    try {
    // A workspace is required when the folder is read-only, a SELECTED file is
    // archived (.alf → must be extracted), OR a character/item NAME table is
    // archived (we extract just those few so speaker names resolve). We do NOT
    // force a workspace merely because *other* archived data exists — keying off
    // the whole non-scene set is what made full-archive games like VERITA try to
    // extract the entire packed game (~1600 BINs) and silently fail.
    const archivedNameTables = allFiles.some(
      (f) => f.category === "names" && f.alf_offset !== undefined,
    );
    const needsWorkspace =
      !isWritable || chosen.some((f) => f.alf_offset !== undefined) || archivedNameTables;
    if (needsWorkspace) {
      await doSetupWorkspaceAndOpen(folderPath, chosen, allFiles);
    } else {
      await doOpenSysBins(chosen.map((f) => f.path));
    }
    } catch (err) {
      store.statusMessage = `Open failed: ${err}`;
    }
  }

  function fmtSize(n: number): string {
    if (n >= 1024 * 1024) return (n / 1024 / 1024).toFixed(1) + " MB";
    if (n >= 1024) return (n / 1024).toFixed(0) + " KB";
    return n + " B";
  }
</script>

<div class="dialog-backdrop" onclick={onClose} onkeydown={(e) => e.key === "Escape" && onClose()} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <header>
      <div>
        <h2>Open game folder</h2>
        <p class="subtitle" title={folder}>{folder} — {files.length} script BIN(s)</p>
      </div>
      <button class="close-btn" onclick={onClose}>×</button>
    </header>

    <div class="tip-banner">💡 Tip: translate Character &amp; item names first so scenes inherit final character names.</div>

    <div class="toolbar">
      <input class="filter" placeholder="Filter by filename…" bind:value={filter} />
      {#if selected.size > 0}
        <button class="btn small" onclick={clearSel}>Clear selection</button>
      {/if}
      <span class="count">{shown.length} of {visibleTotal} shown · <strong>{selected.size} selected</strong></span>
    </div>

    {#if !writable}
      <div class="ro-banner">🔒 This folder is read-only. On Open you'll choose a writable folder — the selected scenes plus the character-name files are copied there so autosave, backups, and speaker names all work.</div>
    {/if}
    {#if translatedCount > 0}
      <div class="tl-banner">⚠ {translatedCount} file{translatedCount === 1 ? "" : "s"} look already-translated (mostly non-Japanese text) — they may be patched copies, not the original game data. You can still open them, but double-check before translating.</div>
    {/if}
    {#if archivedCount > 0}
      <div class="alf-banner">📦 {archivedCount} original BIN{archivedCount === 1 ? "" : "s"} are packed in this game's .alf archives. On open they're extracted into a writable workspace folder you choose (loose files are copied there too), so names resolve and you can edit/export normally.</div>
    {/if}

    <div class="cat-bar">
      <span class="cat-bar-label">Show:</span>
      <button class="cat-chip" class:active={activeCats.size === 0} onclick={clearCats} title="Show all">
        All <span class="cat-n">{visibleTotal}</span>
      </button>
      {#each catGroups as [cat, n]}
        <button class="cat-chip" class:active={activeCats.has(cat)} onclick={() => toggleCat(cat)} title={`Show ${catLabel(cat)}`}>
          {catLabel(cat)} <span class="cat-n">{n}</span>
        </button>
      {/each}
      <span class="cat-hint">Name files resolve speaker names automatically — open them only when you also want to translate the name tables.</span>
    </div>

    <div class="select-bar">
      <label class="select-all" title="Select / deselect everything currently shown">
        <input
          type="checkbox"
          checked={allShownSelected}
          indeterminate={someShownSelected && !allShownSelected}
          onchange={toggleSelectAllShown}
          disabled={shown.length === 0}
        />
        <span>Select all shown ({shown.length})</span>
      </label>
      {#if emptyCount > 0}
        <label class="show-empty" title="Reveal data/structural BINs that have no translatable text">
          <input type="checkbox" bind:checked={showEmpty} />
          <span>Show {emptyCount} file{emptyCount === 1 ? "" : "s"} with no translatable text</span>
        </label>
      {/if}
    </div>

    <div class="list">
      {#each shown as { f, i } (i)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <label class="row">
          <input type="checkbox" checked={selected.has(i)} onchange={() => toggle(i)} />
          <span class="name">
            {f.base_name}<span
              class="src-tag {f.alf_offset !== undefined ? 'archived' : f.layer > 0 ? 'append' : 'base'}"
              title={f.alf_offset !== undefined
                ? "Packed in a .alf archive (original game data) — extracted to your workspace on open"
                : f.layer > 0
                  ? `Append file ($${f.layer}$) — an update/patch that overrides the base game BIN; this is what the game loads`
                  : "Loose base game BIN (no $N$ prefix)"}
            >{f.alf_offset !== undefined ? "📦 archived" : f.layer > 0 ? `append $${f.layer}$` : "base"}</span>{#if f.likely_translated}<span class="tl-badge" title="Dialogue is mostly non-Japanese — this looks already-translated / not original game data">⚠</span>{/if}
          </span>
          <span class="cat-tag cat-{f.category}">{catLabel(f.category)}</span>
          <span class="lines">{f.string_count > 0 ? `${f.string_count} lines` : "—"}</span>
          <span class="sys">SYS{f.sys_version}</span>
          <span class="size">{fmtSize(f.size)}</span>
        </label>
      {/each}
      {#if shown.length === 0}
        {#if filter.trim()}
          <p class="empty">No files match “{filter}”.</p>
        {:else if !showEmpty && emptyCount > 0}
          <p class="empty">Nothing to translate here — {emptyCount} file{emptyCount === 1 ? "" : "s"} have no translatable text. Tick the box above to show them.</p>
        {:else}
          <p class="empty">No files to show.</p>
        {/if}
      {/if}
    </div>

    <footer>
      <button class="btn cancel" onclick={onClose} disabled={loading}>Cancel</button>
      <button class="btn primary" onclick={load} disabled={loading || selected.size === 0}>
        {#if loading}
          {writable ? "Opening…" : "Copying…"}
        {:else}
          {writable ? `Open ${selected.size} file(s)` : `Copy & open ${selected.size} file(s)`}
        {/if}
      </button>
    </footer>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed; inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex; align-items: center; justify-content: center;
    z-index: 1000;
  }
  .dialog {
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    width: 760px; max-width: 95vw; max-height: 85vh;
    display: flex; flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  header {
    padding: 14px 20px;
    border-bottom: 1px solid #2a2a4a;
    display: flex; justify-content: space-between; align-items: flex-start;
  }
  h2 { margin: 0; color: #8ab4f8; font-size: 1.05rem; }
  .subtitle {
    margin: 4px 0 0 0; color: #888; font-size: 0.75rem;
    font-family: monospace; max-width: 640px; overflow: hidden;
    text-overflow: ellipsis; white-space: nowrap;
  }
  .close-btn {
    background: none; border: none; color: #a0a0c0;
    font-size: 1.4rem; cursor: pointer; padding: 0 6px; line-height: 1;
  }
  .close-btn:hover { color: #fff; }
  .toolbar {
    padding: 10px 20px;
    display: flex; align-items: center; gap: 8px;
    border-bottom: 1px solid #2a2a4a;
  }
  .filter {
    flex: 1;
    background: #12122a; border: 1px solid #3a3a5a; border-radius: 4px;
    color: #e0e0f0; padding: 5px 9px; font-size: 0.82rem;
  }
  .count { color: #8ab4f8; font-size: 0.78rem; margin-left: auto; }
  .cat-bar {
    padding: 8px 20px;
    display: flex; align-items: center; flex-wrap: wrap; gap: 6px;
    border-bottom: 1px solid #2a2a4a;
  }
  .cat-bar-label { color: #888; font-size: 0.75rem; margin-right: 2px; }
  .cat-chip {
    background: #1f2a4a; border: 1px solid #3a4a6a; border-radius: 12px;
    color: #bcd0f0; font-size: 0.74rem; padding: 3px 9px; cursor: pointer;
  }
  .cat-chip:hover { background: #2a3a5e; border-color: #5a7aaa; }
  .cat-chip.active { background: #2f4a7a; border-color: #6f9ad8; color: #e0eeff; }
  .cat-n { color: #7a90b8; font-size: 0.68rem; }
  .cat-hint { color: #777; font-size: 0.7rem; margin-left: auto; max-width: 320px; }
  .ro-banner {
    padding: 8px 20px; background: #3a2e16; border-bottom: 1px solid #5a4a2a;
    color: #f0c070; font-size: 0.78rem; line-height: 1.4;
  }
  .tl-banner {
    padding: 8px 20px; background: #3a2016; border-bottom: 1px solid #5a3020;
    color: #f0a070; font-size: 0.78rem; line-height: 1.4;
  }
  .alf-banner {
    padding: 8px 20px; background: #2a2410; border-bottom: 1px solid #4a3e1a;
    color: #e0c070; font-size: 0.78rem; line-height: 1.4;
  }
  .tip-banner {
    padding: 8px 20px; background: #16243a; border-bottom: 1px solid #2a3a5a;
    color: #9cc0f0; font-size: 0.78rem; line-height: 1.4;
  }
  .tl-badge { margin-left: 6px; color: #f0a070; cursor: help; }
  .select-bar {
    padding: 6px 20px;
    display: flex; align-items: center; gap: 10px;
    border-bottom: 1px solid #2a2a4a; background: #15152e;
  }
  .select-all { display: flex; align-items: center; gap: 7px; color: #c0c0e0; font-size: 0.8rem; cursor: pointer; }
  .show-empty { display: flex; align-items: center; gap: 7px; margin-left: auto; color: #9a9ac0; font-size: 0.76rem; cursor: pointer; }
  .show-empty:hover { color: #c0c0e0; }
  .show-empty input[type="checkbox"] { cursor: pointer; accent-color: #8ab4f8; }
  .list { flex: 1; overflow-y: auto; padding: 4px 0; min-height: 0; }
  .row {
    display: grid;
    grid-template-columns: 22px 1fr auto auto auto auto;
    align-items: center; gap: 10px;
    padding: 5px 20px; cursor: pointer; font-size: 0.82rem;
    border-bottom: 1px solid #1a1a30;
  }
  .row:hover { background: #1d1d3a; }
  .name { color: #d0d0e8; font-family: monospace; }
  .src-tag {
    margin-left: 6px; font-size: 0.62rem; padding: 0 6px; border-radius: 3px;
    font-family: monospace; text-transform: uppercase; letter-spacing: 0.02em;
  }
  .src-tag.base { background: #1a3a2e; color: #6fd886; }
  .src-tag.append { background: #2a3a5e; color: #8ab4f8; }
  .src-tag.archived { background: #3a2e1a; color: #e0b070; }
  .lines { color: #6fd886; font-size: 0.72rem; white-space: nowrap; text-align: right; }
  .cat-tag {
    font-size: 0.66rem; padding: 1px 6px; border-radius: 3px;
    background: #2a2a4a; color: #9aa0c0; white-space: nowrap;
  }
  .cat-tag.cat-names { background: #2a2a5e; color: #8ab4f8; }
  .cat-tag.cat-sc { background: #1a3a2e; color: #6fd886; }
  .cat-tag.cat-sp { background: #2e1a3a; color: #c89ad8; }
  .cat-tag.cat-ui { background: #3a2e1a; color: #e0b070; }
  .cat-tag.cat-other { background: #2a2a3a; color: #999; }
  .sys { color: #b58ad0; font-size: 0.7rem; font-family: monospace; }
  .size { color: #777; font-size: 0.72rem; }
  .empty { padding: 24px; text-align: center; color: #888; }
  footer {
    padding: 12px 20px;
    border-top: 1px solid #2a2a4a;
    display: flex; justify-content: flex-end; gap: 8px;
  }
  .btn {
    padding: 6px 14px;
    border: 1px solid #3a3a5a; border-radius: 4px;
    background: #1a1a3e; color: #c0c0e0; cursor: pointer; font-size: 0.85rem;
  }
  .btn:hover:not(:disabled) { background: #2a2a5e; }
  .btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .btn.small { padding: 4px 9px; font-size: 0.76rem; }
  .btn.primary { background: #1f3a5a; border-color: #3a6a9a; color: #bfe0ff; }
</style>
