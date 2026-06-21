<script lang="ts">
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { store } from "../lib/stores.svelte";
  import { confirmDialog } from "../lib/confirm";
  import { exportGlossary, importGlossary, saveFolderGlossary, openFolderInExplorer, fileExists } from "../lib/ipc";
  import { toastInfo, toastError } from "../lib/toast";
  import { clampToViewport } from "../lib/clampMenu";
  import { appConfigDir, dirname, join } from "@tauri-apps/api/path";
  import type { GlossaryEntry } from "../lib/types";
  import ResizeHandle from "./ResizeHandle.svelte";
  import { persistAllSettingsDebounced } from "../lib/settings";
  import GlossarySuggestDialog from "./GlossarySuggestDialog.svelte";

  let suggestOpen = $state(false);

  type GlossaryView = "project" | "global";
  let activeView = $state<GlossaryView>("project");

  let filterText = $state("");
  let editingIndex = $state<number | null>(null);
  /** True when the currently editing entry was just created via "+ Add" */
  let isNewEntry = $state(false);
  /** When true, only show entries with duplicate japanese keys */
  let showDuplicatesOnly = $state(false);

  /** The backing array for the current view */
  let glossaryArray = $derived(
    activeView === "global" ? store.globalGlossary : (store.activeTab?.glossary ?? [])
  );

  /** Persist a mutation to the right place: project glossaries ride
   *  `tab.isDirty` (saved on autosave/export); the global glossary is the only
   *  one that lives in settings.json, so it must be flushed explicitly. */
  function markChanged() {
    if (activeView === "global") {
      persistAllSettingsDebounced();
    } else if (store.activeTab) {
      store.activeTab.isDirty = true;
    }
  }

  type SortMode = "none" | "japanese" | "english" | "category" | "usage";
  let sortMode = $state<SortMode>("none");
  let hideUnused = $state(false);

  /** While an entry is being edited we freeze the display order to this
   *  snapshot of realIndexes. Otherwise typing into a sorted field (e.g. the
   *  Japanese key while sorting by Japanese) makes the row jump around mid-edit. */
  let frozenOrder = $state<number[] | null>(null);

  /** Concatenated original-text corpus across all loaded tabs. Reads only
   *  `original_text` (immutable after load) so this derived stays cached while
   *  the user edits translations or the glossary — it only recomputes when a
   *  BIN is opened/closed or its entries array is replaced (e.g. on import). */
  let usageCorpus = $derived.by(() => {
    let s = "";
    for (const t of store.tabs) {
      for (const e of t.entries) {
        if (e.original_text) s += e.original_text + "\n";
      }
    }
    return s;
  });

  function countOccurrences(haystack: string, needle: string): number {
    if (!needle) return 0;
    let count = 0;
    let pos = haystack.indexOf(needle);
    while (pos !== -1) {
      count++;
      pos = haystack.indexOf(needle, pos + needle.length);
    }
    return count;
  }

  /** Occurrence count per Japanese term across the loaded BIN(s). Lets the user
   *  spot stale entries (0 uses) and prioritize frequent terms. */
  let usageCounts = $derived.by<Map<string, number>>(() => {
    const map = new Map<string, number>();
    const corpus = usageCorpus;
    for (const g of glossaryArray) {
      if (!g.japanese || map.has(g.japanese)) continue;
      map.set(g.japanese, countOccurrences(corpus, g.japanese));
    }
    return map;
  });

  /** Set of japanese keys that appear more than once */
  let duplicateKeys = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const g of glossaryArray) {
      if (g.japanese) {
        counts.set(g.japanese, (counts.get(g.japanese) || 0) + 1);
      }
    }
    const dupes = new Set<string>();
    for (const [key, count] of counts) {
      if (count > 1) dupes.add(key);
    }
    return dupes;
  });

  let duplicateCount = $derived(duplicateKeys.size);

  /** Active category filter — `null` = show all. Toggled via category pills
   *  in the panel toolbar so the user can quickly narrow to just characters
   *  / locations / items without typing into the text filter. */
  let categoryFilter = $state<string | null>(null);

  /** Per-category counts for the filter pills, plus an "all" total. */
  let categoryCounts = $derived.by<Record<string, number>>(() => {
    const out: Record<string, number> = { __all: glossaryArray.length };
    for (const g of glossaryArray) {
      const cat = g.category || "general";
      out[cat] = (out[cat] ?? 0) + 1;
    }
    return out;
  });

  /** Filtered glossary with stable indices into the backing array */
  let filteredGlossary = $derived.by(() => {
    let items = glossaryArray.map((g, idx) => ({ entry: g, realIndex: idx }));

    // The row currently being edited is always kept, even if the user's edit
    // makes it stop matching the active filters — otherwise it would vanish from
    // under the cursor mid-edit. It re-filters once editing ends (closeEditor
    // clears editingIndex on click-away / Update / Enter / Escape).
    const keep = (realIndex: number) => realIndex === editingIndex;

    if (categoryFilter) {
      items = items.filter(({ entry, realIndex }) => keep(realIndex) || (entry.category || "general") === categoryFilter);
    }

    if (showDuplicatesOnly) {
      items = items.filter(({ entry, realIndex }) => keep(realIndex) || duplicateKeys.has(entry.japanese));
    }

    if (filterText.trim()) {
      const lower = filterText.toLowerCase();
      items = items.filter(
        ({ entry: g, realIndex }) =>
          keep(realIndex) ||
          g.japanese.toLowerCase().includes(lower) ||
          g.english.toLowerCase().includes(lower) ||
          g.category.toLowerCase().includes(lower) ||
          g.notes.toLowerCase().includes(lower)
      );
    }

    if (hideUnused) {
      items = items.filter(({ entry, realIndex }) => keep(realIndex) || (usageCounts.get(entry.japanese) ?? 0) > 0);
    }

    if (frozenOrder) {
      // Editing in progress — preserve the order captured when the editor
      // opened so live field edits don't reshuffle the list under the cursor.
      const pos = new Map(frozenOrder.map((ri, i) => [ri, i]));
      items = [...items].sort(
        (a, b) => (pos.get(a.realIndex) ?? Number.MAX_SAFE_INTEGER) - (pos.get(b.realIndex) ?? Number.MAX_SAFE_INTEGER),
      );
    } else if (sortMode !== "none") {
      // Copy before sort — never reorder the backing array; realIndex must stay
      // a stable pointer into glossaryArray for edits/removes.
      items = [...items].sort((a, b) => compareEntries(a.entry, b.entry));
    }

    return items;
  });

  /** Display-order comparator for the active sort mode. */
  function compareEntries(a: GlossaryEntry, b: GlossaryEntry): number {
    switch (sortMode) {
      case "japanese":
        return a.japanese.localeCompare(b.japanese, "ja");
      case "english":
        return a.english.localeCompare(b.english);
      case "category":
        return (a.category || "general").localeCompare(b.category || "general") ||
          a.japanese.localeCompare(b.japanese, "ja");
      case "usage":
        return (usageCounts.get(b.japanese) ?? 0) - (usageCounts.get(a.japanese) ?? 0);
      default:
        return 0;
    }
  }

  let listEl: HTMLDivElement | null = $state(null);
  let panelEl: HTMLDivElement | null = $state(null);

  function addEntry() {
    glossaryArray.unshift({
      japanese: "",
      english: "",
      category: "general",
      notes: "",
    });
    editingIndex = 0;
    isNewEntry = true;
    filterText = "";
    showDuplicatesOnly = false;
    // A brand-new empty entry has no japanese key yet — sorting/hide-unused
    // would bury or hide it, so neutralize them while the user fills it in.
    sortMode = "none";
    hideUnused = false;
    markChanged();
    queueMicrotask(() => {
      if (listEl) listEl.scrollTop = 0;
    });
  }

  function removeEntry(realIndex: number) {
    glossaryArray.splice(realIndex, 1);
    editingIndex = null;
    isNewEntry = false;
    markChanged();
  }

  function updateField(
    realIndex: number,
    field: keyof GlossaryEntry,
    value: string
  ) {
    // Field is typed as `keyof GlossaryEntry`, so this assignment is safe.
    // Cast via `unknown` to satisfy strict structural typing (the previous
    // direct cast to Record<string, string> failed because GlossaryEntry has
    // no index signature).
    (glossaryArray[realIndex] as unknown as Record<string, string>)[field] = value;
    markChanged();
  }

  /** Keep the entry at realIndex, remove all other entries with the same japanese key */
  function keepAndRemoveDuplicates(realIndex: number) {
    const key = glossaryArray[realIndex].japanese;
    // Remove from end to start so indices stay valid
    for (let i = glossaryArray.length - 1; i >= 0; i--) {
      if (i !== realIndex && glossaryArray[i].japanese === key) {
        glossaryArray.splice(i, 1);
        // Adjust realIndex if a removed entry was before it
        if (i < realIndex) realIndex--;
      }
    }
    editingIndex = null;
    markChanged();
  }

  /** Drop an in-progress new entry that the user abandoned without typing a
   *  Japanese key (so we never persist a blank `(empty) → (empty)` row).
   *  Returns the index it removed, or -1. */
  function cleanupNewEntry(): number {
    if (isNewEntry && editingIndex !== null) {
      const e = glossaryArray[editingIndex];
      if (e && !e.japanese.trim() && !e.english.trim()) {
        const removedAt = editingIndex;
        glossaryArray.splice(removedAt, 1);
        markChanged();
        isNewEntry = false;
        return removedAt;
      }
    }
    isNewEntry = false;
    return -1;
  }

  /** Open/close the inline editor for a row. Cleans up any abandoned new entry
   *  first, adjusting the clicked index if that removal shifted the array. */
  function toggleEditor(realIndex: number) {
    if (editingIndex === realIndex) {
      closeEditor();
      return;
    }
    let target = realIndex;
    const removedAt = cleanupNewEntry();
    if (removedAt !== -1 && removedAt < target) target--;
    // Snapshot the current (live-sorted) display order, then freeze it.
    frozenOrder = null;
    frozenOrder = filteredGlossary.map((it) => it.realIndex);
    editingIndex = target;
  }

  function closeEditor() {
    cleanupNewEntry();
    editingIndex = null;
    isNewEntry = false;
    frozenOrder = null;
  }

  function handleEditKeydown(e: KeyboardEvent, entry: GlossaryEntry) {
    if (e.key === "Enter") {
      e.preventDefault();
      if (entry.japanese.trim()) closeEditor();
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeEditor();
    }
  }

  /** Close (and clean up) the editor when the user clicks anywhere outside the
   *  panel — the missing "click-away to dismiss" behavior. Clicks within the
   *  panel (toolbar, other rows) are handled by their own handlers. */
  function handleWindowPointerDown(e: PointerEvent) {
    // Dismiss the location menu on any outside click.
    if (locMenu) {
      const t = e.target as HTMLElement;
      if (!t.closest(".loc-menu")) locMenu = null;
    }
    if (editingIndex === null) return;
    if (panelEl && !panelEl.contains(e.target as Node)) closeEditor();
  }

  function switchView(view: GlossaryView) {
    closeEditor();
    activeView = view;
    filterText = "";
    showDuplicatesOnly = false;
  }

  /** Right-click context menu on the view toggles exposing where that glossary
   *  is stored on disk, with copy + reveal actions. */
  let locMenu = $state<{
    x: number; y: number; label: string; path: string;
    folder: string | null;
    /** Whether the glossary file actually exists on disk yet. */
    exists: boolean;
    /** Optional notice shown when there's no file to reveal. */
    notice?: string;
  } | null>(null);

  async function showLocationMenu(e: MouseEvent, view: GlossaryView) {
    e.preventDefault();
    try {
      if (view === "global") {
        const dir = await appConfigDir();
        const path = await join(dir, "settings.json");
        const exists = await fileExists(path);
        locMenu = {
          x: e.clientX, y: e.clientY,
          label: "Global glossary — stored in settings.json",
          path, folder: dir, exists,
          notice: exists ? undefined : "settings.json not written yet — it's created on first settings save.",
        };
      } else {
        const bp = store.activeTab?.binPath;
        if (!bp) {
          locMenu = {
            x: e.clientX, y: e.clientY, label: "Project glossary",
            path: "No BIN open. The project glossary is embedded in the saved project .json; use “Save to folder” to also write glossary.json next to the BIN.",
            folder: null, exists: false,
            notice: "No current glossary available.",
          };
          return;
        }
        const dir = await dirname(bp);
        const path = await join(dir, "glossary.json");
        const exists = await fileExists(path);
        locMenu = {
          x: e.clientX, y: e.clientY,
          label: "Project glossary — “Save to folder” target (auto-loaded next to the BIN)",
          path, folder: dir, exists,
          notice: exists ? undefined : "No glossary.json saved here yet — use “Save to folder” to create it.",
        };
      }
    } catch (err) {
      toastError(`Couldn't resolve location: ${err}`);
    }
  }

  async function copyLocationPath() {
    if (!locMenu) return;
    try { await navigator.clipboard.writeText(locMenu.path); toastInfo("Path copied to clipboard"); }
    catch (err) { toastError(`Copy failed: ${err}`); }
    locMenu = null;
  }

  async function revealLocation() {
    const folder = locMenu?.folder;
    locMenu = null;
    if (!folder) return;
    try { await openFolderInExplorer(folder); }
    catch (err) { toastError(`Reveal failed: ${err}`); }
  }

  async function handleExportGlossary() {
    const path = await save({
      filters: [{ name: "JSON Files", extensions: ["json"] }],
      defaultPath: "glossary.json",
    });
    if (!path) return;
    try {
      await exportGlossary(glossaryArray, path);
      store.statusMessage = `Glossary exported to ${path.replace(/\\/g, "/").split("/").pop()}`;
    } catch (err) {
      store.statusMessage = `Error exporting glossary: ${err}`;
    }
  }

  /** Save the current project glossary to <binDir>/glossary.json so sibling
   *  BINs auto-pick it up on next load. The project convention: keep one
   *  glossary file at the game folder root, used by every BIN under it. */
  async function handleSaveToFolder() {
    const tab = store.activeTab;
    if (!tab?.binPath) return;
    try {
      await saveFolderGlossary(tab.binPath, glossaryArray);
      toastInfo(`Saved ${glossaryArray.length} entries to glossary.json next to the BIN`);
    } catch (err) {
      toastError(`Failed to save folder glossary: ${err}`);
    }
  }

  async function handleClearAll() {
    if (glossaryArray.length === 0) return;
    const label = activeView === "global" ? "global" : "project";
    const ok = await confirmDialog({
      title: "Clear glossary",
      danger: true,
      confirmLabel: "Clear all",
      message: `Clear all ${glossaryArray.length} entries from the ${label} glossary? This cannot be undone.`,
    });
    if (!ok) return;
    if (activeView === "global") {
      store.globalGlossary = [];
    } else if (store.activeTab) {
      store.activeTab.glossary = [];
    }
    editingIndex = null;
    isNewEntry = false;
    markChanged();
    store.statusMessage = `Cleared ${label} glossary`;
  }

  async function handleImportGlossary() {
    const selected = await open({
      multiple: false,
      filters: [{ name: "JSON Files", extensions: ["json"] }],
    });
    if (!selected) return;
    const filePath = selected;
    try {
      const imported = await importGlossary(filePath);
      let addedCount = 0;
      for (const entry of imported) {
        const exists = glossaryArray.some((g) => g.japanese === entry.japanese);
        if (!exists) {
          glossaryArray.push(entry);
          addedCount++;
        }
      }
      if (addedCount > 0) markChanged();
      store.statusMessage = `Imported ${addedCount} new entries (${imported.length - addedCount} duplicates skipped)`;
    } catch (err) {
      store.statusMessage = `Error importing glossary: ${err}`;
    }
  }

  const CATEGORIES = [
    "character",
    "location",
    "item",
    "skill",
    "title",
    "general",
  ];
</script>

<svelte:window onpointerdown={handleWindowPointerDown} />

<div class="glossary-panel" bind:this={panelEl} style="width: {store.panelWidths.glossary}px; min-width: {store.panelWidths.glossary}px">
  <ResizeHandle
    width={store.panelWidths.glossary}
    minWidth={240}
    maxWidth={800}
    onResize={(w) => (store.panelWidths.glossary = w)}
    onResizeEnd={() => persistAllSettingsDebounced()}
  />
  <div class="panel-header">
    <h3>Glossary</h3>
    <span class="count">{glossaryArray.length} entries</span>
  </div>

  <div class="view-toggle">
    <button
      class="toggle-btn"
      class:active={activeView === "project"}
      onclick={() => switchView("project")}
      oncontextmenu={(e) => showLocationMenu(e, "project")}
      disabled={!store.activeTab}
      title="Right-click to see where this glossary is stored"
    >
      Project
    </button>
    <button
      class="toggle-btn"
      class:active={activeView === "global"}
      onclick={() => switchView("global")}
      oncontextmenu={(e) => showLocationMenu(e, "global")}
      title="Right-click to see where this glossary is stored"
    >
      Global
    </button>
  </div>

  <div class="category-pills">
    <button
      class="cat-pill"
      class:active={categoryFilter === null}
      onclick={() => (categoryFilter = null)}
      title="Show all categories"
    >All <span class="pill-count">{categoryCounts.__all ?? 0}</span></button>
    {#each CATEGORIES as cat}
      {@const count = categoryCounts[cat] ?? 0}
      {#if count > 0 || cat === "general"}
        <button
          class="cat-pill cat-{cat}"
          class:active={categoryFilter === cat}
          onclick={() => (categoryFilter = categoryFilter === cat ? null : cat)}
          title="Filter to {cat} entries"
        >{cat} <span class="pill-count">{count}</span></button>
      {/if}
    {/each}
  </div>

  <div class="panel-toolbar">
    <input
      type="text"
      class="filter-input"
      placeholder="Filter..."
      bind:value={filterText}
    />
    <button class="add-btn" onclick={addEntry} title="Add glossary entry">
      +
    </button>
    {#if duplicateCount > 0}
      <button
        class="dupes-btn"
        class:active={showDuplicatesOnly}
        onclick={() => (showDuplicatesOnly = !showDuplicatesOnly)}
        title="Show duplicate entries"
      >
        {duplicateCount} dupes
      </button>
    {/if}
    <button
      class="io-btn"
      onclick={() => (suggestOpen = true)}
      title="Auto-suggest glossary entries from proper-noun terms (katakana, kanji, mixed) in the loaded BIN(s)"
      disabled={store.tabs.length === 0}
    >
      Suggest
    </button>
    <button
      class="io-btn"
      onclick={() => (store.glossaryConsistencyOpen = true)}
      title="Across ALL open tabs: flag translated rows where a glossary term's Japanese appears but its English rendering is missing. Fix inline or jump to the row."
      disabled={store.tabs.length === 0}
    >
      Check
    </button>
    <button
      class="io-btn"
      onclick={() => (store.glossaryAutoApplyOpen = true)}
      title="Across ALL open tabs: review safe glossary substitutions (fill exact-term rows, replace leaked Japanese). Nothing applies until you confirm; the batch is undoable."
      disabled={store.tabs.length === 0}
    >
      Auto-apply
    </button>
    <button class="io-btn" onclick={handleImportGlossary} title="Import glossary from JSON">
      Import
    </button>
    <button class="io-btn" onclick={handleExportGlossary} title="Export glossary to JSON" disabled={glossaryArray.length === 0}>
      Export
    </button>
    {#if activeView === "project" && store.activeTab?.binPath}
      <button
        class="io-btn"
        onclick={handleSaveToFolder}
        title="Write current entries to glossary.json next to the BIN — auto-loaded on next BIN open from the same folder"
        disabled={glossaryArray.length === 0}
      >
        Save to folder
      </button>
    {/if}
    <button class="clear-btn" onclick={handleClearAll} title="Clear all entries from the current glossary view" disabled={glossaryArray.length === 0}>
      Clear All
    </button>
  </div>

  <div class="sort-row">
    <label class="sort-label">
      Sort
      <select class="sort-select" bind:value={sortMode}>
        <option value="none">Added order</option>
        <option value="japanese">Japanese A→Z</option>
        <option value="english">English A→Z</option>
        <option value="category">Category</option>
        <option value="usage">Usage (most first)</option>
      </select>
    </label>
    <button
      class="hide-unused-btn"
      class:active={hideUnused}
      onclick={() => (hideUnused = !hideUnused)}
      disabled={store.tabs.length === 0}
      title={store.tabs.length === 0
        ? "Open a BIN to compute usage"
        : hideUnused
          ? "Currently hiding entries with no occurrences — click to show all"
          : "Hide entries that don't appear in any loaded BIN"}
    >
      {hideUnused ? "Show unused" : "Hide unused"}
    </button>
  </div>

  <div class="glossary-list" bind:this={listEl}>
    {#each filteredGlossary as { entry, realIndex } (entry)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="glossary-item"
        class:editing={editingIndex === realIndex}
        class:duplicate={entry.japanese && duplicateKeys.has(entry.japanese)}
        onclick={() => toggleEditor(realIndex)}
      >
        {#if editingIndex === realIndex}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="edit-form" onkeydown={(e) => handleEditKeydown(e, entry)}>
            <label>
              <span>Japanese</span>
              <input
                type="text"
                value={entry.japanese}
                oninput={(e) =>
                  updateField(realIndex, "japanese", (e.target as HTMLInputElement).value)}
                onclick={(e) => e.stopPropagation()}
              />
            </label>
            <label>
              <span>English</span>
              <input
                type="text"
                value={entry.english}
                oninput={(e) =>
                  updateField(realIndex, "english", (e.target as HTMLInputElement).value)}
                onclick={(e) => e.stopPropagation()}
              />
            </label>
            <label>
              <span>Category</span>
              <select
                value={entry.category}
                onchange={(e) =>
                  updateField(realIndex, "category", (e.target as HTMLSelectElement).value)}
                onclick={(e) => e.stopPropagation()}
              >
                {#each CATEGORIES as cat}
                  <option value={cat}>{cat}</option>
                {/each}
              </select>
            </label>
            <label>
              <span>Notes</span>
              <input
                type="text"
                value={entry.notes}
                oninput={(e) =>
                  updateField(realIndex, "notes", (e.target as HTMLInputElement).value)}
                onclick={(e) => e.stopPropagation()}
              />
            </label>
            <div class="form-actions">
              <button
                class="confirm-btn"
                disabled={!entry.japanese.trim()}
                title={!entry.japanese.trim() ? "Enter a Japanese term first" : ""}
                onclick={(e) => { e.stopPropagation(); closeEditor(); }}
              >
                {isNewEntry ? "Add" : "Update"}
              </button>
              {#if entry.japanese && duplicateKeys.has(entry.japanese)}
                <button
                  class="keep-btn"
                  onclick={(e) => { e.stopPropagation(); keepAndRemoveDuplicates(realIndex); }}
                  title="Keep this entry and remove all other entries with the same Japanese text"
                >
                  Keep
                </button>
              {/if}
              <button
                class="remove-btn"
                onclick={(e) => { e.stopPropagation(); removeEntry(realIndex); }}
              >
                Remove
              </button>
            </div>
          </div>
        {:else}
          <div class="item-summary">
            <span class="jp">{entry.japanese || "(empty)"}</span>
            <span class="arrow">&rarr;</span>
            <span class="en">{entry.english || "(empty)"}</span>
            {#if store.tabs.length > 0 && entry.japanese}
              {@const uses = usageCounts.get(entry.japanese) ?? 0}
              <span class="use-badge" class:zero={uses === 0} title="{uses} occurrence{uses === 1 ? '' : 's'} in loaded BIN(s)">{uses}×</span>
            {/if}
            <span class="cat-badge cat-{entry.category || 'general'}">{entry.category || 'general'}</span>
          </div>
        {/if}
      </div>
    {/each}

    {#if filteredGlossary.length === 0}
      <div class="empty-glossary">
        {#if filterText}
          No matches for "{filterText}"
        {:else}
          No glossary entries yet
        {/if}
      </div>
    {/if}
  </div>
</div>

{#if suggestOpen}
  <GlossarySuggestDialog {activeView} onClose={() => (suggestOpen = false)} />
{/if}

{#if locMenu}
  <div class="loc-menu" use:clampToViewport style="left: {locMenu.x}px; top: {locMenu.y}px">
    <div class="loc-label">{locMenu.label}</div>
    <div class="loc-path" title={locMenu.path}>{locMenu.path}</div>
    {#if locMenu.notice}
      <div class="loc-notice">⚠ {locMenu.notice}</div>
    {/if}
    <div class="loc-actions">
      {#if locMenu.exists}
        <button class="loc-btn" onclick={copyLocationPath}>Copy path</button>
      {/if}
      {#if locMenu.folder}
        <button class="loc-btn" onclick={revealLocation}>
          {locMenu.exists ? "Reveal in Explorer" : "Open folder"}
        </button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .glossary-panel {
    display: flex;
    flex-direction: column;
    border-left: 1px solid #2a2a4a;
    background: #141430;
    position: relative;
    flex-shrink: 0;
  }

  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    border-bottom: 1px solid #2a2a4a;
  }

  .panel-header h3 {
    margin: 0;
    font-size: 0.9rem;
    color: #8ab4f8;
  }

  .count {
    font-size: 0.75rem;
    color: #666;
  }

  .view-toggle {
    display: flex;
    padding: 6px 8px;
    gap: 4px;
    border-bottom: 1px solid #222240;
  }

  .toggle-btn {
    flex: 1;
    padding: 4px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #1a1a3e;
    color: #888;
    font-size: 0.75rem;
    cursor: pointer;
    transition: all 0.15s;
  }

  .toggle-btn:hover:not(:disabled) {
    color: #c0c0e0;
    border-color: #5a5a8a;
  }

  .toggle-btn.active {
    background: #2a3a6e;
    border-color: #6c9bd2;
    color: #8ab4f8;
  }

  .toggle-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .category-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 6px 8px;
    border-bottom: 1px solid #222240;
    background: #1a1a30;
  }
  .cat-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: transparent;
    border: 1px solid #3a3a5a;
    border-radius: 12px;
    color: #888;
    font-size: 0.7rem;
    padding: 2px 8px;
    cursor: pointer;
    text-transform: capitalize;
    transition: background 0.1s, border-color 0.1s, color 0.1s;
  }
  .cat-pill:hover { color: #c0c0e0; border-color: #5a5a8a; }
  .cat-pill.active {
    background: #2a3a6e;
    color: #fff;
    border-color: #5a7aff;
  }
  .pill-count {
    font-family: monospace;
    font-size: 0.65rem;
    color: #6a6a8a;
    background: rgba(0, 0, 0, 0.25);
    padding: 0 4px;
    border-radius: 8px;
    min-width: 14px;
    text-align: center;
  }
  .cat-pill.active .pill-count { color: #c0d0ff; background: rgba(0, 0, 0, 0.35); }
  /* Per-category accents — kept subtle so the active state still pops. */
  .cat-pill.cat-character.active { background: #2a3a6e; border-color: #6c9bd2; }
  .cat-pill.cat-location.active { background: #2a4a3a; border-color: #6fd886; }
  .cat-pill.cat-item.active { background: #4a3a1a; border-color: #e8a838; }
  .cat-pill.cat-skill.active { background: #4a2a3a; border-color: #e070a0; }
  .cat-pill.cat-title.active { background: #3a2a4a; border-color: #c08aff; }
  .cat-pill.cat-general.active { background: #2a3a4a; border-color: #6a8aaa; }

  .panel-toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 6px 8px;
    border-bottom: 1px solid #222240;
  }

  .filter-input {
    flex: 1 1 120px;
    min-width: 0;
    padding: 4px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #1a1a3e;
    color: #c0c0e0;
    font-size: 0.8rem;
    outline: none;
  }

  .filter-input:focus {
    border-color: #6c9bd2;
  }

  .add-btn {
    padding: 4px 10px;
    border: 1px solid #2a5a3e;
    border-radius: 3px;
    background: #1a3a2e;
    color: #6fd886;
    font-size: 0.8rem;
    cursor: pointer;
  }

  .add-btn:hover {
    background: #2a4a3e;
  }

  .io-btn {
    padding: 4px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #1a1a3e;
    color: #888;
    font-size: 0.75rem;
    cursor: pointer;
    white-space: nowrap;
  }

  .io-btn:hover:not(:disabled) {
    color: #c0c0e0;
    border-color: #5a5a8a;
  }

  .io-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .clear-btn {
    padding: 4px 8px;
    border: 1px solid #5a2a2a;
    border-radius: 3px;
    background: #3a1a1a;
    color: #e06060;
    font-size: 0.75rem;
    cursor: pointer;
    white-space: nowrap;
  }

  .clear-btn:hover:not(:disabled) {
    background: #4a2a2a;
  }

  .clear-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .dupes-btn {
    padding: 4px 8px;
    border: 1px solid #5a4a2a;
    border-radius: 3px;
    background: #3a2a1a;
    color: #e0a030;
    font-size: 0.75rem;
    cursor: pointer;
    white-space: nowrap;
  }

  .dupes-btn:hover {
    background: #4a3a2a;
  }

  .dupes-btn.active {
    background: #4a3a1a;
    border-color: #e0a030;
  }

  .sort-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-bottom: 1px solid #222240;
    background: #16162c;
  }
  .sort-label {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 0.7rem;
    color: #888;
  }
  .sort-select {
    padding: 2px 4px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #1a1a3e;
    color: #c0c0e0;
    font-size: 0.72rem;
    outline: none;
  }
  .sort-select:focus { border-color: #6c9bd2; }
  .hide-unused-btn {
    margin-left: auto;
    padding: 3px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #1a1a3e;
    color: #888;
    font-size: 0.7rem;
    cursor: pointer;
    white-space: nowrap;
  }
  .hide-unused-btn:hover:not(:disabled) { color: #c0c0e0; border-color: #5a5a8a; }
  .hide-unused-btn.active { background: #2a3a6e; border-color: #6c9bd2; color: #8ab4f8; }
  .hide-unused-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  .glossary-list {
    flex: 1;
    overflow-y: auto;
    padding: 4px 0;
  }

  .glossary-item {
    padding: 6px 10px;
    border-bottom: 1px solid #1e1e3e;
    cursor: pointer;
    transition: background 0.1s;
  }

  .glossary-item:hover {
    background: #1e1e42;
  }

  .glossary-item.editing {
    background: #1e2a4e;
    cursor: default;
  }

  .glossary-item.duplicate {
    border-left: 2px solid #e0a030;
  }

  .item-summary {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 0.8rem;
  }

  .jp {
    color: #c0c0e0;
    flex-shrink: 0;
  }

  .arrow {
    color: #444;
    flex-shrink: 0;
  }

  .en {
    color: #8ab4f8;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .use-badge {
    flex-shrink: 0;
    font-family: monospace;
    font-size: 0.62rem;
    color: #7aa0c0;
    background: rgba(108, 155, 210, 0.12);
    border-radius: 3px;
    padding: 1px 4px;
  }
  .use-badge.zero {
    color: #c08a6a;
    background: rgba(224, 160, 48, 0.12);
  }

  .cat-badge {
    padding: 1px 5px;
    border-radius: 3px;
    background: #2a2a4a;
    color: #888;
    font-size: 0.65rem;
    flex-shrink: 0;
    text-transform: capitalize;
  }
  .cat-badge.cat-character { background: #2a3a6e; color: #c0d8ff; }
  .cat-badge.cat-location { background: #2a4a3a; color: #b0e8c0; }
  .cat-badge.cat-item { background: #4a3a1a; color: #f4d8a0; }
  .cat-badge.cat-skill { background: #4a2a3a; color: #f0a0c8; }
  .cat-badge.cat-title { background: #3a2a4a; color: #d8b8f0; }
  .cat-badge.cat-general { background: #2a3a4a; color: #a0c0d0; }

  .edit-form {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .edit-form label {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .edit-form label span {
    font-size: 0.7rem;
    color: #666;
    text-transform: uppercase;
  }

  .edit-form input,
  .edit-form select {
    padding: 4px 6px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #1a1a3e;
    color: #c0c0e0;
    font-size: 0.8rem;
    outline: none;
  }

  .edit-form input:focus,
  .edit-form select:focus {
    border-color: #6c9bd2;
  }

  .form-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    margin-top: 4px;
  }

  .confirm-btn {
    padding: 3px 10px;
    border: 1px solid #2a5a3e;
    border-radius: 3px;
    background: #1a3a2e;
    color: #6fd886;
    font-size: 0.75rem;
    cursor: pointer;
  }

  .confirm-btn:hover:not(:disabled) {
    background: #2a4a3e;
  }

  .confirm-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .remove-btn {
    padding: 3px 10px;
    border: 1px solid #5a2a2a;
    border-radius: 3px;
    background: #3a1a1a;
    color: #e06060;
    font-size: 0.75rem;
    cursor: pointer;
  }

  .remove-btn:hover {
    background: #4a2a2a;
  }

  .keep-btn {
    padding: 3px 10px;
    border: 1px solid #5a4a2a;
    border-radius: 3px;
    background: #3a2a1a;
    color: #e0a030;
    font-size: 0.75rem;
    cursor: pointer;
  }

  .keep-btn:hover {
    background: #4a3a2a;
  }

  .empty-glossary {
    padding: 20px;
    text-align: center;
    color: #555;
    font-size: 0.85rem;
  }

  .loc-menu {
    position: fixed;
    z-index: 1100;
    width: 320px;
    max-width: 90vw;
    background: #15152a;
    border: 1px solid #3a3a5a;
    border-radius: 5px;
    box-shadow: 0 6px 24px rgba(0, 0, 0, 0.6);
    padding: 8px 10px;
  }
  .loc-label {
    font-size: 0.72rem;
    color: #8ab4f8;
    margin-bottom: 5px;
  }
  .loc-path {
    font-family: monospace;
    font-size: 0.72rem;
    color: #c0c0e0;
    background: #1a1a3e;
    border: 1px solid #2a2a4a;
    border-radius: 3px;
    padding: 5px 7px;
    margin-bottom: 7px;
    word-break: break-all;
    max-height: 80px;
    overflow-y: auto;
  }
  .loc-notice {
    font-size: 0.72rem;
    color: #e8a838;
    background: rgba(232, 168, 56, 0.1);
    border-radius: 3px;
    padding: 4px 7px;
    margin-bottom: 7px;
  }
  .loc-actions { display: flex; gap: 6px; justify-content: flex-end; }
  .loc-btn {
    padding: 3px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #1a1a3e;
    color: #c0c0e0;
    font-size: 0.74rem;
    cursor: pointer;
  }
  .loc-btn:hover { background: #2a2a5e; border-color: #5a5a8a; }
</style>
