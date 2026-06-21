<script lang="ts">
  import type { ParenteGameFile } from "../lib/ipc";
  import { doLoadParenteEntries } from "../lib/actions";

  interface Props {
    title: string;
    files: ParenteGameFile[];
    onClose: () => void;
  }
  let { title, files, onClose }: Props = $props();

  // Selection by array index (stable within this dialog session).
  let selected = $state<Set<number>>(new Set());
  let filter = $state("");
  // Active category view-filters (empty = show all). Independent of selection.
  let activeCats = $state<Set<string>>(new Set());
  // Content filter: all / normal-only / adult-only.
  let adultFilter = $state<"all" | "normal" | "adult">("all");
  let loading = $state(false);

  /** Group a file into a human category by its path + name. */
  function categoryOf(f: ParenteGameFile): string {
    const path = f.path.toLowerCase();
    const base = f.name.replace(/^\$\d+\$/, "").toLowerCase();
    if (base.startsWith("99_test") || base.startsWith("!")) return "Test";
    if (path.includes("/src/sys")) return "System / UI";
    if (path.includes("/src/seq")) return "Sequence";
    if (path.includes("/src/game")) {
      return base.startsWith("sc") ? "Scenes" : "Other (game)";
    }
    return "Other";
  }

  // Category for each file (index-aligned with `files`).
  let cats = $derived(files.map(categoryOf));
  // Distinct categories with counts, for the quick-select chips.
  let catGroups = $derived.by(() => {
    const m = new Map<string, number>();
    for (const c of cats) m.set(c, (m.get(c) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1]);
  });

  // Visible list = text filter AND category view-filter (empty cats = all).
  let shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return files
      .map((f, i) => ({ f, i }))
      .filter(({ f, i }) => {
        if (activeCats.size > 0 && !activeCats.has(cats[i])) return false;
        if (adultFilter === "adult" && !f.adult) return false;
        if (adultFilter === "normal" && f.adult) return false;
        if (q && !(f.path + "/" + f.name).toLowerCase().includes(q)) return false;
        return true;
      });
  });
  let adultCount = $derived(files.filter((f) => f.adult).length);

  // Whether the source archive varies (game-folder mode) — show the column then.
  let showSource = $derived(new Set(files.map((f) => f.source)).size > 1);
  let totalLines = $derived(files.reduce((s, f) => s + f.string_count, 0));
  let shownLines = $derived(shown.reduce((s, { f }) => s + f.string_count, 0));
  let selectedLines = $derived(
    [...selected].reduce((s, i) => s + (files[i]?.string_count ?? 0), 0),
  );
  // Master-select state for the currently-shown rows.
  let allShownSelected = $derived(shown.length > 0 && shown.every(({ i }) => selected.has(i)));
  let someShownSelected = $derived(shown.some(({ i }) => selected.has(i)));

  function toggle(i: number) {
    if (selected.has(i)) selected.delete(i);
    else selected.add(i);
    selected = new Set(selected);
  }
  /** Toggle a category as a VIEW filter (not selection). */
  function toggleCat(cat: string) {
    if (activeCats.has(cat)) activeCats.delete(cat);
    else activeCats.add(cat);
    activeCats = new Set(activeCats);
  }
  function clearCats() {
    activeCats = new Set();
  }
  /** Select or deselect every currently-shown row (tri-state master). */
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
    const chosen = [...selected].map((i) => files[i]).filter(Boolean);
    if (chosen.length === 0) return;
    loading = true;
    // Dismiss the picker FIRST so the global loading-progress overlay (lower
    // z-index) is visible on top while scenes load. The async load continues
    // after unmount — it only mutates the store.
    onClose();
    await doLoadParenteEntries(chosen);
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
        <h2>Load scene files</h2>
        <p class="subtitle" title={title}>{title} — {files.length} file(s) with dialogue · {totalLines} lines</p>
      </div>
      <button class="close-btn" onclick={onClose}>×</button>
    </header>

    <div class="toolbar">
      <input class="filter" placeholder="Filter by name/path…" bind:value={filter} />
      {#if selected.size > 0}
        <button class="btn small" onclick={clearSel}>Clear selection</button>
      {/if}
      <span class="count">
        {shown.length} of {files.length} shown · <strong>{selected.size} selected</strong>{selected.size > 0 ? ` (${selectedLines.toLocaleString()} lines)` : ""}
      </span>
    </div>

    <div class="cat-bar">
      <span class="cat-bar-label">Show:</span>
      <button class="cat-chip" class:active={activeCats.size === 0} onclick={clearCats} title="Show all categories">
        All <span class="cat-n">{files.length}</span>
      </button>
      {#each catGroups as [cat, n]}
        <button class="cat-chip" class:active={activeCats.has(cat)} onclick={() => toggleCat(cat)} title={`Show ${cat} files`}>
          {cat} <span class="cat-n">{n}</span>
        </button>
      {/each}
      {#if adultCount > 0}
        <span class="cat-divider"></span>
        <button class="cat-chip" class:active={adultFilter === "normal"} onclick={() => (adultFilter = adultFilter === "normal" ? "all" : "normal")} title="Show only non-adult scenes">
          Normal <span class="cat-n">{files.length - adultCount}</span>
        </button>
        <button class="cat-chip adult" class:active={adultFilter === "adult"} onclick={() => (adultFilter = adultFilter === "adult" ? "all" : "adult")} title="Show only adult scenes (heuristic)">
          🔞 Adult <span class="cat-n">{adultCount}</span>
        </button>
      {/if}
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
      <span class="select-bar-lines">{shownLines.toLocaleString()} lines shown</span>
    </div>

    <div class="list">
      {#each shown as { f, i } (i)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <label class="row" class:wide={showSource}>
          <input type="checkbox" checked={selected.has(i)} onchange={() => toggle(i)} />
          <span class="name">{f.name}{#if f.adult}<span class="adult-badge" title="Adult content (heuristic)">🔞</span>{/if}</span>
          <span class="cat-tag cat-{cats[i].replace(/[^a-z]/gi,'').toLowerCase()}">{cats[i]}</span>
          <span class="path">{f.path}</span>
          {#if showSource}<span class="src" title={f.arc_path}>{f.source}</span>{/if}
          <span class="lines">{f.string_count} lines</span>
          <span class="size">{fmtSize(f.size)}</span>
        </label>
      {/each}
      {#if shown.length === 0}
        <p class="empty">No files match “{filter}”.</p>
      {/if}
    </div>

    <footer>
      <button class="btn cancel" onclick={onClose} disabled={loading}>Cancel</button>
      <button class="btn primary" onclick={load} disabled={loading || selected.size === 0}>
        {loading ? "Loading…" : `Load ${selected.size} file(s)`}
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
    width: 820px; max-width: 95vw; max-height: 85vh;
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
    font-family: monospace; max-width: 700px; overflow: hidden;
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
  .cat-chip.active {
    background: #2f4a7a; border-color: #6f9ad8; color: #e0eeff;
  }
  .cat-n { color: #7a90b8; font-size: 0.68rem; }
  .cat-divider { width: 1px; height: 18px; background: #3a3a5a; margin: 0 4px; }
  .cat-chip.adult { border-color: #6a3a4a; color: #e0a0b8; }
  .cat-chip.adult.active { background: #5a2a3e; border-color: #b06a86; color: #ffe0ec; }
  .adult-badge { margin-left: 6px; font-size: 0.72rem; }
  .select-bar {
    padding: 6px 20px;
    display: flex; align-items: center; gap: 10px;
    border-bottom: 1px solid #2a2a4a; background: #15152e;
  }
  .select-all {
    display: flex; align-items: center; gap: 7px;
    color: #c0c0e0; font-size: 0.8rem; cursor: pointer;
  }
  .select-bar-lines { color: #6fd886; font-size: 0.74rem; margin-left: auto; }
  .list { flex: 1; overflow-y: auto; padding: 4px 0; min-height: 0; }
  .row {
    display: grid;
    grid-template-columns: 22px 1fr auto auto auto auto;
    align-items: center; gap: 10px;
    padding: 5px 20px; cursor: pointer; font-size: 0.82rem;
    border-bottom: 1px solid #1a1a30;
  }
  .row.wide { grid-template-columns: 22px 1fr auto auto auto auto auto; }
  .cat-tag {
    font-size: 0.66rem; padding: 1px 6px; border-radius: 3px;
    background: #2a2a4a; color: #9aa0c0; white-space: nowrap;
  }
  .cat-tag.catscenes { background: #1a3a2e; color: #6fd886; }
  .cat-tag.catsystemui { background: #2a2a5e; color: #8ab4f8; }
  .cat-tag.catsequence { background: #3a2e1a; color: #e0b070; }
  .cat-tag.cattest { background: #3a1a2a; color: #d07090; }
  .row:hover { background: #1d1d3a; }
  .name { color: #d0d0e8; font-family: monospace; }
  .path { color: #6c9bd2; font-family: monospace; font-size: 0.72rem; }
  .src { color: #b58ad0; font-size: 0.7rem; font-family: monospace; }
  .lines { color: #6fd886; font-size: 0.72rem; white-space: nowrap; }
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
