<script lang="ts">
  import { store } from "../lib/stores.svelte";
  import { closeTab } from "../lib/ipc";
  import { confirmDialog } from "../lib/confirm";
  import { clampToViewport } from "../lib/clampMenu";
  import { doSaveProject, doExportBin, doLoadFile, saveParenteProject } from "../lib/actions";
  import BackupHistoryDialog from "./BackupHistoryDialog.svelte";
  import { hasTranslatableContent } from "../lib/exportOmit";
  import { tabOverflowCounts } from "../lib/engineMode";

  /** Tab whose backup history dialog is currently open, if any. */
  let backupTabIndex = $state<number | null>(null);

  /** The scrollable tab strip — used to keep the active tab in view. */
  let barEl = $state<HTMLDivElement | null>(null);

  /** Double-clicking a tab scrolls its editor back to the top. Switch to it
   *  first so the request targets the now-active tab's EditorTable. */
  function handleTabDblClick(index: number) {
    const tab = store.tabs[index];
    if (!tab) return;
    store.switchTab(index);
    store.scrollToTopRequest = { tabId: tab.id, nonce: Date.now() };
  }

  // Keep the active tab visible in the (possibly overflowing) strip — when the
  // user cycles tabs with the wheel/keyboard, auto-scroll so the focused tab
  // isn't hidden off-edge. inline:nearest scrolls only the strip; block:nearest
  // avoids nudging the page vertically.
  $effect(() => {
    const idx = store.activeTabIndex;
    void idx;
    if (!barEl) return;
    queueMicrotask(() => {
      const active = barEl?.querySelector(".tab.active") as HTMLElement | null;
      active?.scrollIntoView({ inline: "nearest", block: "nearest" });
    });
  });

  /** Inline context menu state — opens on right-click of a tab. */
  let ctxOpen = $state(false);
  let ctxPos = $state({ x: 0, y: 0 });
  let ctxTabIndex = $state<number | null>(null);

  /** Drag-and-drop reorder state. Source index captured on dragstart,
   *  drop target highlighted on dragover. Native HTML5 drag because tabs
   *  are simple horizontal items — no need for pointer-event reimplementation. */
  let dragFromIndex = $state<number | null>(null);
  let dragOverIndex = $state<number | null>(null);

  function handleDragStart(e: DragEvent, index: number) {
    if (!e.dataTransfer) return;
    dragFromIndex = index;
    e.dataTransfer.effectAllowed = "move";
    // Required by Firefox to start a drag — value doesn't matter.
    e.dataTransfer.setData("text/plain", String(index));
  }
  function handleDragOver(e: DragEvent, index: number) {
    if (dragFromIndex === null) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    dragOverIndex = index;
  }
  function handleDragLeave(index: number) {
    if (dragOverIndex === index) dragOverIndex = null;
  }
  function handleDrop(e: DragEvent, index: number) {
    e.preventDefault();
    if (dragFromIndex !== null && dragFromIndex !== index) {
      store.reorderTab(dragFromIndex, index);
    }
    dragFromIndex = null;
    dragOverIndex = null;
  }
  function handleDragEnd() {
    dragFromIndex = null;
    dragOverIndex = null;
  }

  /** Status-dot popup — shown when the user clicks the colored status dot.
   *  Lists the tab's untranslated rows with click-to-jump. Positioned with
   *  `fixed` coordinates computed from the clicked tab's bounding rect so
   *  the popup follows the tab no matter what scroll offset the bar has. */
  let dotPopupTabIndex = $state<number | null>(null);
  let dotPopupLeft = $state(0);
  let dotPopupTop = $state(0);

  function openDotPopup(e: MouseEvent, index: number) {
    e.stopPropagation();
    if (dotPopupTabIndex === index) {
      // Toggle close on second click of the same dot.
      dotPopupTabIndex = null;
      return;
    }
    const tabEl = (e.currentTarget as HTMLElement).closest(".tab") as HTMLElement | null;
    if (tabEl) {
      const r = tabEl.getBoundingClientRect();
      dotPopupLeft = r.left;
      dotPopupTop = r.bottom + 4;
    }
    dotPopupTabIndex = index;
  }
  function closeDotPopup() { dotPopupTabIndex = null; }

  /** Untranslated rows for the popup's target tab. Each entry returns
   *  enough info for the list row + click-to-jump. Recomputed reactively
   *  as translations are added/removed. */
  let dotPopupRows = $derived.by(() => {
    if (dotPopupTabIndex === null) return [];
    const tab = store.tabs[dotPopupTabIndex];
    if (!tab) return [];
    const isParente = tab.format === "parente";
    const out: { rowIndex: number; label: string; original: string; speaker?: string }[] = [];
    for (let i = 0; i < tab.entries.length; i++) {
      const e = tab.entries[i];
      if (e.deleted || e.is_context_row || e.omit_export) continue;
      if (!isParente && !EDITABLE.has(e.label)) continue;
      if (e.translated_text.trim().length > 0) continue;
      // Same filter the progress dot uses — popup must show the same set
      // of rows that count toward the "remaining" total. Parente rows are all
      // translatable strings, so the content filter doesn't apply to them.
      if (!isParente && !hasTranslatableContent(e.original_text)) continue;
      const sref = tab.rowSpeakers?.[String(e.instruction_index)];
      const speaker = sref?.literal_name ?? undefined;
      out.push({ rowIndex: i, label: e.label, original: e.original_text, speaker });
    }
    return out;
  });

  /** Jump to a specific row in the popup's target tab. Switches tabs (so
   *  the editor renders that tab) then fires the standard pendingScrollRequest
   *  the editor table watches for. Popup closes after jump. */
  function jumpToUntranslatedRow(rowIndex: number) {
    if (dotPopupTabIndex === null) return;
    const tab = store.tabs[dotPopupTabIndex];
    if (!tab) return;
    store.switchTab(dotPopupTabIndex);
    store.pendingScrollRequest = {
      tabId: tab.id,
      rowIndex,
      nonce: Date.now(),
    };
    closeDotPopup();
  }

  function handleTabContextMenu(e: MouseEvent, index: number) {
    e.preventDefault();
    ctxTabIndex = index;
    ctxPos = { x: e.clientX, y: e.clientY };
    ctxOpen = true;
  }
  function closeCtxMenu() { ctxOpen = false; ctxTabIndex = null; }

  function openBackupHistory() {
    if (ctxTabIndex === null) return;
    backupTabIndex = ctxTabIndex;
    closeCtxMenu();
  }

  async function handleReverted(idx: number) {
    // The on-disk project JSON was replaced; reload the tab so the in-memory
    // state reflects the reverted snapshot.
    const tab = store.tabs[idx];
    if (!tab || !tab.projectPath) return;
    const path = tab.projectPath;
    // Close + reopen — simpler than mutating tab state in place and ensures
    // every derived/cached field re-derives cleanly from the restored JSON.
    await closeTab(tab.id);
    store.closeTab(idx);
    await doLoadFile(path);
  }

  /** Per-tab translation progress signal: (translated / translatable) % over
   *  user-editable entries. Drives the colored status dot. Recomputed per tab
   *  per render — cheap because we only count, no string ops. */
  const EDITABLE = new Set(["show-text", "display-furigana", "comment"]);
  function tabProgress(tab: typeof store.tabs[number]): { pct: number; translated: number; total: number; overCap: number } {
    // Parente: every extracted row is a translatable string.
    if (tab.format === "parente") {
      let t = 0, d = 0;
      for (const e of tab.entries) {
        if (e.deleted || !e.original_text) continue;
        t++;
        if (e.translated_text.trim().length > 0) d++;
      }
      const pct = t === 0 ? 0 : Math.round((d / t) * 100);
      return { pct, translated: d, total: t, overCap: 0 };
    }
    let total = 0, done = 0;
    for (const e of tab.entries) {
      if (e.deleted || e.is_context_row || e.omit_export) continue;
      if (!EDITABLE.has(e.label)) continue;
      // Skip rows with no real content to translate (bracket-only triplet
      // middle refs, `????` placeholders, symbol-only). These would never
      // get a translation by design — counting them as "untranslated" would
      // permanently pin tabs to amber even after the user is done.
      if (!hasTranslatableContent(e.original_text)) continue;
      total++;
      if (e.translated_text.trim().length > 0) done++;
    }
    const pct = total === 0 ? 0 : Math.round((done / total) * 100);
    // Same counts as the Stats panel + Review-overflows button (engineMode): line +
    // box overflows, or engine boxes that don't fit when the loader wraps rows itself.
    const counts = tabOverflowCounts(tab);
    const overCap = counts.line + counts.box;
    return { pct, translated: done, total, overCap };
  }
  function statusColor(pct: number, total: number): string {
    if (total === 0) return "empty";
    if (pct === 100) return "done";
    if (pct === 0) return "pending";
    return "progress";
  }

  function handleWheel(e: WheelEvent) {
    if (store.tabs.length <= 1) return;
    // Horizontal intent (trackpad sideways swipe or Shift+wheel) should scroll
    // the overflowing tab bar natively — don't hijack it for tab switching.
    // Only a dominantly-vertical wheel cycles tabs.
    if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
    e.preventDefault();
    if (e.deltaY > 0) {
      // Scroll down → next tab (right)
      const next = (store.activeTabIndex + 1) % store.tabs.length;
      store.switchTab(next);
    } else if (e.deltaY < 0) {
      // Scroll up → previous tab (left)
      const prev = (store.activeTabIndex - 1 + store.tabs.length) % store.tabs.length;
      store.switchTab(prev);
    }
  }

  async function handleCloseTab(index: number, e: MouseEvent) {
    e.stopPropagation();
    const tab = store.tabs[index];
    if (!tab) return;

    // Parente tabs autosave their translations to the app data dir, so closing
    // never loses progress — persist now (clears dirty) before any prompt.
    if (tab.format === "parente" && tab.isDirty) {
      await saveParenteProject(tab);
    }

    // Dirty tabs always confirm (never lose edits silently). Clean tabs only
    // confirm when the user has opted into close confirmation.
    if (tab.isDirty || store.confirmTabClose) {
      const name = tab.sourceBinFilename || "this tab";
      const confirmed = await confirmDialog({
        title: tab.isDirty ? "Close tab — unsaved changes" : "Close tab",
        message: tab.isDirty
          ? `“${name}” has unsaved changes. Close it anyway? Your edits will be lost.`
          : `Close “${name}”?`,
        confirmLabel: "Close tab",
        danger: tab.isDirty,
      });
      if (!confirmed) return;
    }

    await closeTab(tab.id);
    store.closeTab(index);
  }
</script>

{#if store.tabs.length > 0}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="tab-bar" bind:this={barEl} onwheel={handleWheel}>
    {#each store.tabs as tab, index (tab.id)}
      {@const prog = tabProgress(tab)}
      {@const colorClass = statusColor(prog.pct, prog.total)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="tab"
        class:active={store.activeTabIndex === index}
        class:drop-target={dragOverIndex === index && dragFromIndex !== null && dragFromIndex !== index}
        class:dragging={dragFromIndex === index}
        onclick={() => store.switchTab(index)}
        ondblclick={() => handleTabDblClick(index)}
        oncontextmenu={(e) => handleTabContextMenu(e, index)}
        title="{tab.sourceBinFilename || 'Untitled'}{prog.total > 0 ? ` — ${prog.translated}/${prog.total} (${prog.pct}%)` : ''}{tab.isDirty ? ' • unsaved changes' : ''}"
        draggable="true"
        ondragstart={(e) => handleDragStart(e, index)}
        ondragover={(e) => handleDragOver(e, index)}
        ondragleave={() => handleDragLeave(index)}
        ondrop={(e) => handleDrop(e, index)}
        ondragend={handleDragEnd}
      >
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <span
          class="status-dot status-{colorClass}"
          class:dot-clickable={colorClass !== "done" && colorClass !== "empty"}
          class:has-overflow={prog.overCap > 0}
          onclick={(e) => {
            if (colorClass === "done" || colorClass === "empty") return;
            openDotPopup(e, index);
          }}
          title={(() => {
            const parts: string[] = [];
            if (colorClass === "done") parts.push("Fully translated");
            else if (colorClass === "empty") parts.push("No translatable rows");
            else parts.push(`Click to see ${prog.total - prog.translated} untranslated row${prog.total - prog.translated === 1 ? "" : "s"}`);
            if (prog.overCap > 0) parts.push(`⚠ ${prog.overCap} item${prog.overCap === 1 ? "" : "s"} over limits`);
            return parts.join(" · ");
          })()}
        ></span>
        <span class="tab-label">
          {tab.sourceBinFilename || "Untitled"}
        </span>
        {#if tab.isDirty}
          <span class="dirty-dot" title="Unsaved changes">●</span>
        {/if}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <span
          class="tab-close"
          onclick={(e) => handleCloseTab(index, e)}
          title="Close tab"
        >&times;</span>
      </div>
    {/each}
  </div>
{/if}

{#if dotPopupTabIndex !== null}
  {@const tab = store.tabs[dotPopupTabIndex]}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="dot-popup-backdrop" onclick={closeDotPopup}></div>
  <div class="dot-popup" use:clampToViewport style="left: {dotPopupLeft}px; top: {dotPopupTop}px">
    <header class="dot-popup-header">
      <span>
        <strong>{dotPopupRows.length}</strong> untranslated row{dotPopupRows.length === 1 ? "" : "s"}
        in <span class="dot-popup-fname">{tab?.sourceBinFilename || "tab"}</span>
      </span>
      <button class="dot-popup-close" onclick={closeDotPopup} title="Close">×</button>
    </header>
    {#if dotPopupRows.length === 0}
      <p class="dot-popup-empty">All caught up. 🎉</p>
    {:else}
      <div class="dot-popup-list">
        {#each dotPopupRows.slice(0, 200) as r}
          <button
            class="dot-popup-row"
            onclick={() => jumpToUntranslatedRow(r.rowIndex)}
            title="Jump to this row"
          >
            <span class="dpr-label">{r.label}</span>
            {#if r.speaker}<span class="dpr-speaker">{r.speaker}</span>{/if}
            <span class="dpr-orig">{r.original}</span>
          </button>
        {/each}
        {#if dotPopupRows.length > 200}
          <p class="dot-popup-truncate">
            (showing first 200 of {dotPopupRows.length} — translate some to see the rest)
          </p>
        {/if}
      </div>
    {/if}
  </div>
{/if}

{#if ctxOpen && ctxTabIndex !== null}
  {@const ctxTab = store.tabs[ctxTabIndex]}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="ctx-backdrop" onclick={closeCtxMenu} oncontextmenu={(e) => { e.preventDefault(); closeCtxMenu(); }}></div>
  <div class="ctx-menu" use:clampToViewport style="left: {ctxPos.x}px; top: {ctxPos.y}px">
    <button
      class="ctx-item"
      disabled={!ctxTab?.projectPath}
      title={ctxTab?.projectPath ? "" : "Tab needs to have been autosaved at least once before backups exist"}
      onclick={openBackupHistory}
    >Restore from backup…</button>
  </div>
{/if}

{#if backupTabIndex !== null}
  {@const tab = store.tabs[backupTabIndex]}
  {#if tab?.projectPath}
    <BackupHistoryDialog
      projectPath={tab.projectPath}
      binFilename={tab.sourceBinFilename || "Untitled"}
      onClose={() => (backupTabIndex = null)}
      onReverted={() => handleReverted(backupTabIndex!)}
    />
  {/if}
{/if}

<style>
  .tab-bar {
    display: flex;
    align-items: stretch;
    background: #12122a;
    overflow-x: auto;
    min-height: 32px;
    flex: 1;
    /* anchor for the status-dot popup's absolute positioning */
    position: relative;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px 4px 8px;
    cursor: pointer;
    font-size: 0.8rem;
    color: #888;
    border-right: 1px solid #2a2a4a;
    white-space: nowrap;
    transition: background 0.1s, color 0.1s, min-width 0.15s, max-width 0.15s;
    /* Inactive tabs: wide enough to read most filenames, short enough that
     * many tabs fit without horizontal scroll. Active tab gets larger
     * range so its filename is always fully readable. */
    min-width: 90px;
    max-width: 170px;
    flex-shrink: 0;
  }

  .tab:hover {
    background: #1a1a3e;
    color: #c0c0e0;
  }

  .tab.active {
    /* Active tab grows so its name is easy to read and the close button is
     * a clearly larger target — reduces accidental-close fat-fingers. */
    min-width: 180px;
    max-width: 280px;
  }
  .tab.dragging {
    opacity: 0.4;
  }
  .tab.drop-target {
    /* Blue left-edge bar indicating where the dragged tab will land. */
    box-shadow: inset 3px 0 0 0 #5a7aff;
  }

  /* Translation-progress dot: red = nothing started, amber = in progress,
   * green = fully translated, gray = no editable rows. Quick visual scan
   * across many tabs.
   *
   * Uniform size across ALL states. Use `outline` (which doesn't affect
   * layout/footprint) for the overflow ring and hover hint so the dot's
   * visible footprint stays exactly 8×8 in every state — previously the
   * box-shadow ring + hover scale made dots look different sizes per state. */
  .status-dot {
    flex-shrink: 0;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #444;
    outline: 2px solid transparent;
    outline-offset: 1px;
    transition: outline-color 0.1s;
  }
  .status-dot.status-pending { background: #d04444; }
  .status-dot.status-progress { background: #e0a030; }
  .status-dot.status-done { background: #4ad080; }
  .status-dot.status-empty { background: #444; }
  .status-dot.dot-clickable {
    cursor: pointer;
  }
  .status-dot.dot-clickable:hover {
    outline-color: rgba(255, 255, 255, 0.35);
  }
  /* Overflow ring — composes with any base status color to signal
   * "fully/partially translated but some row exceeds the line cap." Amber
   * outline so it reads as "needs attention" without changing the dot's
   * core translation-progress meaning. Outline (not box-shadow) so the
   * dot's reserved space stays uniform across all states. */
  .status-dot.has-overflow {
    outline-color: #e0a030;
  }
  .status-dot.has-overflow.dot-clickable:hover {
    outline-color: #f4c060;
  }

  /* Untranslated-rows popup anchored beneath the clicked tab's dot. */
  .dot-popup-backdrop {
    position: fixed;
    inset: 0;
    z-index: 998;
  }
  .dot-popup {
    position: fixed;
    z-index: 999;
    background: #15152a;
    border: 1px solid #5a7aff;
    border-radius: 5px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.6);
    width: 520px;
    max-width: 92vw;
    max-height: 480px;
    display: flex;
    flex-direction: column;
  }
  .dot-popup-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    border-bottom: 1px solid #2a3a6e;
    font-size: 0.82rem;
    color: #c0c0e0;
  }
  .dot-popup-header strong { color: #ffd060; }
  .dot-popup-fname {
    color: #6c9bd2;
    font-family: monospace;
  }
  .dot-popup-close {
    background: transparent;
    border: none;
    color: #8a8aaa;
    font-size: 1.1rem;
    cursor: pointer;
    padding: 0 4px;
  }
  .dot-popup-close:hover { color: #fff; }

  .dot-popup-empty {
    padding: 22px 16px;
    text-align: center;
    color: #6fd886;
    font-size: 0.9rem;
  }

  .dot-popup-list {
    overflow-y: auto;
    padding: 4px 0;
  }
  .dot-popup-row {
    display: grid;
    grid-template-columns: 110px auto 1fr;
    gap: 8px;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    border-bottom: 1px solid #1a1a30;
    color: #c0c0e0;
    font-size: 0.78rem;
    padding: 5px 12px;
    cursor: pointer;
    align-items: baseline;
  }
  .dot-popup-row:hover {
    background: #1d1d3a;
  }
  .dpr-label {
    color: #8aa0ff;
    font-family: monospace;
    font-size: 0.7rem;
  }
  .dpr-speaker {
    color: #6fd886;
    font-size: 0.7rem;
    background: #102a20;
    padding: 1px 5px;
    border-radius: 3px;
  }
  .dpr-orig {
    color: #d8a86f;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .dot-popup-truncate {
    padding: 10px 12px;
    font-size: 0.72rem;
    color: #6a6a8a;
    font-style: italic;
    text-align: center;
  }

  .dirty-dot {
    flex-shrink: 0;
    color: #f4c87a;
    font-size: 0.7rem;
    line-height: 1;
    margin-left: -2px;
  }

  .ctx-backdrop {
    position: fixed;
    inset: 0;
    z-index: 999;
  }
  .ctx-menu {
    position: fixed;
    z-index: 1000;
    background: #1e1e42;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
    padding: 4px 0;
    min-width: 200px;
  }
  .ctx-item {
    display: block;
    width: 100%;
    background: transparent;
    border: none;
    color: #c0c0e0;
    font-size: 0.85rem;
    text-align: left;
    padding: 7px 14px;
    cursor: pointer;
  }
  .ctx-item:hover:not(:disabled) { background: #2a2a5e; }
  .ctx-item:disabled { color: #555; cursor: not-allowed; }

  .tab.active {
    background: #1a1a2e;
    color: #8ab4f8;
    border-bottom: 2px solid #6c9bd2;
  }

  .tab-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tab-close {
    flex-shrink: 0;
    font-size: 1rem;
    color: #555;
    line-height: 1;
    padding: 2px 5px;
    border-radius: 3px;
    /* Hidden by default on inactive tabs — appears on hover or when active.
     * Prevents accidental closes when cycling/scrolling through many tabs. */
    opacity: 0;
    transition: opacity 0.1s;
  }
  .tab:hover .tab-close,
  .tab.active .tab-close {
    opacity: 1;
  }
  .tab-close:hover {
    color: #e06060;
    background: rgba(224, 96, 96, 0.2);
  }
</style>
