<script lang="ts">
  import { store } from "../lib/stores.svelte";
  import { tick } from "svelte";
  import { confirmDialog } from "../lib/confirm";
  import { findOverflows, findBoxOverflows, entryDisplayLength, type OverflowRow, type BoxOverflow } from "../lib/overflow";
  import { autoSplitEntry, optimizeEntry } from "../lib/ipc";
  import { applySplitPlan, splitBoxBridge, splitOverflowingGroup, paginateBox, canInsertBoxBreak } from "../lib/insertions";
  import { dialogueBoxCaps, type GameBoxCaps } from "../lib/gameLayout";
  import { engineBoxRulesFor, engineMetricsFor } from "../lib/engineMode";
  import { findEngineBoxOverflows, type EngineBoxOverflow } from "../lib/engineOverflow";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  /** Reactive list of current overflows — re-derives as fixes are applied. */
  let overflows = $derived.by<OverflowRow[]>(() => {
    const tab = store.activeTab;
    if (!tab) return [];
    return findOverflows(tab.entries, store.translatedLineCharCap);
  });

  /** Boxes that exceed the per-box limits (too many lines, or over the total
   *  char cap) — typically a side effect of an over-eager auto-split. These
   *  can't be auto-fixed (the split already happened); the user jumps in to
   *  shorten a line or merge one back. Re-derives as edits are made. */
  let boxOverflows = $derived.by<BoxOverflow[]>(() => {
    const tab = store.activeTab;
    if (!tab) return [];
    return findBoxOverflows(tab.entries, store.translatedCharCap, store.maxLinesPerBox, store.bottomLineCharCap);
  });

  // Engine box mode (loader word wrap on + metrics reported): the engine wraps rows itself,
  // so only whole boxes can overflow and the fix is more boxes, never 0x6F line breaks.
  let engineRules = $derived(engineBoxRulesFor(store.activeTab));
  let engineOverflows = $derived.by<EngineBoxOverflow[]>(() => {
    const tab = store.activeTab;
    return tab && engineRules ? findEngineBoxOverflows(tab.entries, engineRules) : [];
  });

  let working = $state(false);
  let progress = $state<{ current: number; total: number; action: string; cancellable?: boolean } | null>(null);

  // Caps measured from the game's dialogue box (engine wrap width + rows, with the
  // game's enabled text box profile) — null when no game folder is known for this tab.
  let gameCaps = $derived.by<GameBoxCaps | null>(() => {
    const tab = store.activeTab;
    const layout = tab ? store.textLayoutByTab[tab.id] : null;
    if (!layout) return null;
    return dialogueBoxCaps(layout, layout.gameDir ? store.boxProfiles[layout.gameDir] : undefined);
  });

  // Re-flow inputs — pre-filled from the game's measured box when known, else the
  // current caps; the user can still correct them before rebuilding the layout.
  const initialCaps = (() => {
    const tab = store.activeTab;
    const layout = tab ? store.textLayoutByTab[tab.id] : null;
    return layout ? dialogueBoxCaps(layout, layout.gameDir ? store.boxProfiles[layout.gameDir] : undefined) : null;
  })();
  let reflowLineCap = $state(initialCaps?.charsPerLine ?? (store.translatedLineCharCap > 0 ? store.translatedLineCharCap : 47));
  let reflowLinesPerBox = $state(initialCaps?.linesPerBox ?? (store.maxLinesPerBox > 0 ? store.maxLinesPerBox : 2));

  /** Close the dialog; if a bulk pass is running, request its cancel too so
   *  closing it doesn't orphan a long run (it stops after the current file). */
  function handleClose() {
    if (working) {
      store.bulkCancelRequested = true;
      store.statusMessage = "Stopping after the current file…";
    }
    onClose();
  }

  // Line-splitting (wrapping within one box) needs only a per-line cap.
  let canAutoSplit = $derived(store.translatedLineCharCap > 0);
  // Pagination (adding boxes) clones the BIN's box-transition bridge — which
  // carries the engine "advance box" call — so it needs a usable bridge.
  let templateQuality = $derived(store.activeTab?.bridgeTemplateQuality);
  let canAddBoxes = $derived(canInsertBoxBreak(templateQuality));

  /** Name of the first OTHER open tab whose bridge we could borrow (donor), or null. */
  let donorName = $derived.by<string | null>(() => {
    const a = store.activeTab;
    const d = store.tabs.find(
      (t) => t.id !== a?.id && (t.bridgeTemplateQuality === "clean" || t.bridgeTemplateQuality === "degraded"),
    );
    return d ? (d.sourceBinFilename || d.id) : null;
  });


  /** Total splittable overflowing rows across ALL open tabs (for the global
   *  split action — so the user doesn't have to visit each tab after a CC import). */
  let globalOverflowCount = $derived.by<number>(() => {
    if (store.translatedLineCharCap <= 0) return 0;
    let n = 0;
    for (const tab of store.tabs) {
      n += findOverflows(tab.entries, store.translatedLineCharCap)
        .filter((r) => !r.isContinuation && r.splittable).length;
    }
    return n;
  });

  /** Total over-long boxes (more than maxLinesPerBox display lines) across ALL
   *  open tabs — for the global paginate action. */
  let globalBoxOverflowCount = $derived.by<number>(() => {
    let n = 0;
    for (const tab of store.tabs) {
      n += findBoxOverflows(tab.entries, store.translatedCharCap, store.maxLinesPerBox, store.bottomLineCharCap)
        .filter((b) => b.tooManyLines || b.bottomLineTooLong).length;
    }
    return n;
  });

  /** Handle the per-row "Split lines" button. LINE-ONLY — wraps the overflow
   *  into multiple display lines (end-text-line breaks); never opens a new box.
   *
   *  Two cases:
   *  - Row's own text exceeds the cap → Rust auto-split wraps it into lines.
   *  - Row is part of an over-cap group → insert end-text-line opcodes to
   *    break the group into multiple display lines.
   */
  async function handleAutoSplitRow(row: OverflowRow) {
    const tab = store.activeTab;
    if (!tab || working) return;
    working = true;
    try {
      if (row.individualLength > store.translatedLineCharCap) {
        // Single-entry text overflow → Rust wrap into lines (single box).
        const plan = await autoSplitEntry(
          tab.id,
          row.entry.translated_text,
          row.id,
          store.translatedLineCharCap,
          store.translatedCharCap,
          store.maxLinesPerBox,
          store.bottomLineCharCap,
        );
        const result = applySplitPlan(plan, await splitBoxBridge(tab.id));
        if (typeof result === "string") {
          store.statusMessage = result;
        }
      } else if (row.partOfGroup) {
        // Group overflow → insert end-text-line between entries (line-only).
        const result = splitOverflowingGroup(row.entry, store.translatedLineCharCap);
        if (typeof result === "string") {
          store.statusMessage = result;
        }
      } else {
        store.statusMessage = `Row #${row.id} doesn't need splitting.`;
      }
    } catch (err) {
      store.statusMessage = `Split failed for #${row.id}: ${err}`;
    }
    working = false;
  }

  /** Engine mode: split single-text boxes that don't fit into balanced engine boxes. */
  async function handleEngineSplit(boxes: EngineBoxOverflow[]) {
    const tab = store.activeTab;
    const metrics = engineMetricsFor(tab);
    if (!tab || !metrics || !engineRules || working) return;
    const ids = boxes.filter((b) => b.singleText).map((b) => b.id);
    if (ids.length === 0) return;
    store.captureBulkUndo(ids.length === 1 ? "Split into boxes" : "Split into boxes (all)", [tab]);
    working = true;
    progress = { current: 0, total: 0, action: "Splitting into boxes" };
    await tick();
    const { engineSplitEntries } = await import("../lib/actions");
    const r = await engineSplitEntries(tab, ids, metrics, engineRules.rowsPerBox, engineRules.bottomCap);
    working = false;
    progress = null;
    let msg = `Split ${r.split} text(s) into ${r.paginated > 0 ? "balanced boxes" : "boxes"}`;
    if (r.errors.length > 0) msg += ` (${r.errors.length} issue(s), e.g. ${r.errors[0]})`;
    store.statusMessage = msg;
  }

  function jumpToEngineBox(box: EngineBoxOverflow) {
    const tab = store.activeTab;
    if (!tab) return;
    tab.selectedRowIndex = box.index;
    store.pendingScrollRequest = { tabId: tab.id, rowIndex: box.index, nonce: Date.now() };
    onClose();
  }

  /** Paginate a single over-long box — clone the box-transition bridge every
   *  maxLinesPerBox lines so it splits into multiple boxes. */
  async function handlePaginateBox(box: BoxOverflow) {
    const tab = store.activeTab;
    if (!tab || working) return;
    const startEntry = tab.entries[box.index];
    if (!startEntry) return;
    working = true;
    const result = await paginateBox(startEntry, store.maxLinesPerBox, store.bottomLineCharCap);
    if (typeof result === "string") store.statusMessage = result;
    tab.entries = [...tab.entries]; // nudge reactivity after in-place inserts
    tab.isDirty = true;
    working = false;
  }

  /** Paginate every over-long box in one pass. Works back-to-front so earlier
   *  box indices stay valid as inserts happen. */
  async function handlePaginateAll() {
    const tab = store.activeTab;
    if (!tab || working) return;
    const tooLong = boxOverflows.filter((b) => b.tooManyLines || b.bottomLineTooLong);
    if (tooLong.length === 0) {
      store.statusMessage = "No boxes exceed the line limit.";
      return;
    }
    store.captureBulkUndo("Paginate boxes", [tab]);
    working = true;
    progress = { current: 0, total: 0, action: "Paginating" };
    await tick();
    // Delegate to the shared routine — fetches the box bridge once and paginates
    // synchronously (one repaint) instead of an IPC + repaint per box.
    const { bulkPaginateActiveTab } = await import("../lib/actions");
    const r = await bulkPaginateActiveTab();
    tab.entries = [...tab.entries];
    tab.isDirty = true;
    working = false;
    progress = null;
    let msg = `Paginated ${r.paginated} box(es) into multiple boxes`;
    if (r.errors.length > 0) msg += ` (${r.errors.length} issue(s))`;
    store.statusMessage = msg;
  }

  /** Global paginate: split over-long boxes into multiple boxes across EVERY
   *  open tab (the companion to global line-split, post-import). */
  async function handlePaginateAllTabs() {
    if (working) return;
    store.captureBulkUndo("Paginate — all tabs", [...store.tabs]);
    working = true;
    progress = { current: 0, total: store.tabs.length, action: "Paginating all tabs", cancellable: true };
    await tick();
    const { bulkPaginateAllTabs } = await import("../lib/actions");
    const r = await bulkPaginateAllTabs((current, total, name) => {
      progress = { current, total, action: `Paginating ${name}`, cancellable: true };
    });
    working = false;
    progress = null;
    if (store.activeTab) store.activeTab.entries = [...store.activeTab.entries]; // nudge
    const parts = [`${r.paginated} box(es) across ${r.tabsProcessed} tab(s)`];
    if (r.errors.length > 0) parts.push(`${r.errors.length} issue(s)`);
    store.statusMessage = `${r.cancelled ? "Paginate STOPPED" : "Paginate all tabs"} — ${parts.join(", ")}`;
  }

  /** Global split: run the line-split across EVERY open tab in one go (the
   *  post-CC-import workflow — no need to visit each tab). */
  async function handleAutoSplitAllTabs() {
    if (working) return;
    store.captureBulkUndo("Split lines — all tabs", [...store.tabs]);
    working = true;
    progress = { current: 0, total: store.tabs.length, action: "Splitting all tabs", cancellable: true };
    await tick();
    const { bulkAutoSplitAllTabs } = await import("../lib/actions");
    const r = await bulkAutoSplitAllTabs((current, total, name) => {
      progress = { current, total, action: `Splitting ${name}`, cancellable: true };
    });
    working = false;
    progress = null;
    if (store.activeTab) store.activeTab.entries = [...store.activeTab.entries]; // nudge
    const parts = [`${r.split} fixed across ${r.tabsProcessed} tab(s)`];
    if (r.failed > 0) parts.push(`${r.failed} failed`);
    store.statusMessage = `${r.cancelled ? "Split STOPPED" : "Split all tabs"} — ${parts.join(", ")}`;
  }

  /** Re-flow: revert every existing split, then re-wrap + re-paginate at the
   *  caps in the re-flow inputs. The fix for translations baked at the wrong box
   *  geometry (e.g. 50×3 when the game is 47×2) — text is preserved, only the
   *  line/box layout is rebuilt. `allTabs` does it across every open tab. */
  async function handleReflow(allTabs: boolean) {
    if (working) return;
    const tab = store.activeTab;
    if (!tab) return;
    if (reflowLineCap < 1 || reflowLinesPerBox < 1) {
      store.statusMessage = "Set chars/line and lines/box (both ≥ 1) first.";
      return;
    }
    const scope = allTabs ? `all ${store.tabs.length} open tabs` : "this tab";
    const ok = await confirmDialog({
      title: "Re-flow to box caps",
      confirmLabel: "Re-flow",
      message:
        `Revert every existing split in ${scope} and rebuild the layout at ` +
        `${reflowLineCap} chars/line × ${reflowLinesPerBox} lines/box?\n\n` +
        `Your translations are preserved — only the line/box wrapping is redone. ` +
        `This overwrites all current auto-split + pagination (undoable). Export afterward to save to the BIN.`,
    });
    if (!ok) return;
    store.captureBulkUndo(allTabs ? "Re-flow — all tabs" : "Re-flow", allTabs ? [...store.tabs] : [tab]);
    working = true;
    progress = { current: 0, total: allTabs ? store.tabs.length : 1, action: "Re-flowing", cancellable: allTabs };
    await tick(); // paint the progress overlay before the heavy work starts
    const { reflowActiveTab, reflowAllTabs } = await import("../lib/actions");
    let msg: string;
    if (allTabs) {
      const r = await reflowAllTabs(reflowLineCap, reflowLinesPerBox, (current, total, name) => {
        progress = { current, total, action: `Re-flowing ${name}`, cancellable: true };
      });
      const head = r.cancelled ? "Re-flow STOPPED" : "Re-flow all tabs";
      msg = `${head} — ${r.reverted} beats reverted, ${r.split} re-wrapped, ${r.paginated} paginated across ${r.tabsProcessed} tab(s)`;
      if (r.errors.length > 0) msg += ` (${r.errors.length} issue(s), e.g. ${r.errors[0]})`;
    } else {
      const r = await reflowActiveTab(reflowLineCap, reflowLinesPerBox);
      tab.entries = [...tab.entries];
      tab.isDirty = true;
      msg = `Re-flow — ${r.reverted} beats reverted, ${r.split} re-wrapped, ${r.paginated} paginated`;
      if (r.errors.length > 0) msg += ` (e.g. ${r.errors[0]})`;
    }
    working = false;
    progress = null;
    if (store.activeTab) store.activeTab.entries = [...store.activeTab.entries]; // reactivity nudge
    store.statusMessage = msg;
  }

  /** Bulk split: sequential pass through every splittable overflowing row.
   *  Handles two kinds: entry-text overflow (Rust auto-split) and group
   *  overflow (frontend ETL insertion). Group splits are deduped by group
   *  identity so we don't try to split the same group multiple times. */
  async function handleAutoSplitAll() {
    const tab = store.activeTab;
    if (!tab || working) return;
    const candidateCount = overflows.filter((r) => !r.isContinuation && r.splittable).length;
    if (candidateCount === 0) {
      store.statusMessage = "Nothing to auto-split.";
      return;
    }
    store.captureBulkUndo("Split lines", [tab]);

    working = true;
    progress = { current: 0, total: candidateCount, action: "Splitting" };
    await tick();
    // Delegate to the shared bulk routine (also used by auto-split-on-import).
    const { bulkAutoSplitActiveTab } = await import("../lib/actions");
    const r = await bulkAutoSplitActiveTab((current, total) => {
      progress = { current, total, action: "Splitting" };
    });

    working = false;
    progress = null;
    tab.entries = [...tab.entries]; // nudge reactivity after in-place inserts
    tab.isDirty = true;
    const parts: string[] = [`${r.split} fixed`];
    if (r.failed > 0) parts.push(`${r.failed} failed`);
    let msg = `Split complete — ${parts.join(", ")}`;
    if (r.errors.length > 0) msg += ` (e.g. ${r.errors[0]})`;
    store.statusMessage = msg;
  }

  /** Per-row optimize via Claude API. */
  async function handleOptimizeRow(row: OverflowRow) {
    const tab = store.activeTab;
    if (!tab || working) return;
    if (!store.apiKeyConfigured) {
      store.statusMessage = "Set your API key in Settings first";
      return;
    }
    working = true;
    try {
      await runOptimize(row, tab);
      store.statusMessage = `Optimized row #${row.id}`;
    } catch (err) {
      store.statusMessage = `Optimize failed for #${row.id}: ${err}`;
    }
    working = false;
  }

  /** Bulk optimize via Claude API (sequential, costs money — confirm up front). */
  async function handleOptimizeAll() {
    const tab = store.activeTab;
    if (!tab || working) return;
    if (!store.apiKeyConfigured) {
      store.statusMessage = "Set your API key in Settings first";
      return;
    }
    const toFix = overflows.filter((r) => !r.isContinuation);
    if (toFix.length === 0) {
      store.statusMessage = "Nothing to optimize.";
      return;
    }
    const ok = await confirmDialog({
      title: "Optimize via Claude API",
      confirmLabel: "Optimize",
      message:
        `Optimize ${toFix.length} overflowing rows via Claude API?\n\n` +
        `Each row uses one API call. This will be billed to your API key.`,
    });
    if (!ok) return;

    working = true;
    progress = { current: 0, total: toFix.length, action: "Optimizing" };
    let done = 0;
    let failed = 0;

    for (let i = 0; i < toFix.length; i++) {
      progress = { current: i + 1, total: toFix.length, action: "Optimizing" };
      const row = toFix[i];
      // Skip if no longer overflowing
      const current = tab.entries.find((e) => e.instruction_index === row.id);
      if (!current || entryDisplayLength(current) <= store.translatedLineCharCap) continue;
      try {
        await runOptimize(row, tab);
        done++;
      } catch {
        failed++;
      }
    }

    working = false;
    progress = null;
    tab.isDirty = true;
    store.statusMessage = `Optimize complete — ${done} updated${failed > 0 ? `, ${failed} failed` : ""}`;
  }

  /** Shared optimize call — mirrors handleOptimizeBatch in TranslationControls. */
  async function runOptimize(row: OverflowRow, tab: NonNullable<typeof store.activeTab>) {
    const i = tab.entries.findIndex((e) => e.instruction_index === row.id);
    if (i < 0) throw new Error("Row no longer exists");
    const entry = tab.entries[i];

    const contextStart = Math.max(0, i - 15);
    const context = tab.entries.slice(contextStart, i).filter(
      (e) => !e.is_context_row && !e.deleted,
    );

    let blockLength = 0;
    for (let j = i; j >= 0; j--) {
      if (tab.entries[j].label === "wait-for-input") break;
      if ((tab.entries[j].label === "show-text" || tab.entries[j].label === "display-furigana") && !tab.entries[j].deleted) {
        blockLength += entryDisplayLength(tab.entries[j]);
      }
    }

    const currentLen = entryDisplayLength(entry);
    const previousTranslation = entry.translated_text;

    const result = await optimizeEntry(
      entry,
      context,
      store.promptGlossary,
      tab.claudeModel,
      store.translatedLineCharCap,
      store.translatedCharCap,
      currentLen,
      blockLength,
      store.minimalNotes,
    );

    const newLen = result.translation.length;
    let notes = `[LLM: ${tab.claudeModel}] [Optimized] ${currentLen}->${newLen} chars`;
    if (result.notes && result.notes.trim() !== "") notes += ` | ${result.notes}`;
    notes += ` | Pre-optimize: "${previousTranslation}"`;

    tab.entries[i].translated_text = result.translation;
    tab.entries[i].notes = notes;
  }

  function truncate(s: string, n: number): string {
    return s.length <= n ? s : s.slice(0, n) + "…";
  }

  /** Jump to the row in the editor table. `row.id` is the instruction_index;
   *  the scroll request needs the array index. Reuses the shared
   *  pendingScrollRequest mechanism (scrolls + flashes the row), then closes
   *  this dialog so the user can edit the line directly instead of splitting. */
  function jumpToRow(row: OverflowRow) {
    const tab = store.activeTab;
    if (!tab) return;
    const idx = tab.entries.findIndex((e) => e.instruction_index === row.id);
    if (idx < 0) return;
    tab.selectedRowIndex = idx;
    store.pendingScrollRequest = { tabId: tab.id, rowIndex: idx, nonce: Date.now() };
    onClose();
  }

  /** Jump to the first line of an over-limit box. `box.index` is already the
   *  array index, so no instruction_index lookup is needed. */
  function jumpToBox(box: BoxOverflow) {
    const tab = store.activeTab;
    if (!tab) return;
    tab.selectedRowIndex = box.index;
    store.pendingScrollRequest = { tabId: tab.id, rowIndex: box.index, nonce: Date.now() };
    onClose();
  }
</script>

<div class="dialog-backdrop" onclick={handleClose} onkeydown={(e) => e.key === "Escape" && handleClose()} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <header>
      <h2>Overflow Review</h2>
      <button class="close-btn" onclick={handleClose}>×</button>
    </header>

    <div class="reflow-bar">
      <div class="reflow-head">
        🔄 <strong>Re-flow to box caps</strong> — rebuild the line/box layout for a corrected box size. Reverts every existing split and re-wraps at the caps below; your translations are preserved.
      </div>
      <div class="reflow-controls">
        <label>Chars/line <input type="number" min="1" max="200" bind:value={reflowLineCap} disabled={working} /></label>
        <label>Lines/box <input type="number" min="1" max="10" bind:value={reflowLinesPerBox} disabled={working} /></label>
        {#if gameCaps}
          <button
            class="game-caps"
            disabled={working || (reflowLineCap === gameCaps.charsPerLine && reflowLinesPerBox === gameCaps.linesPerBox)}
            onclick={() => { reflowLineCap = gameCaps.charsPerLine; reflowLinesPerBox = gameCaps.linesPerBox; }}
            title="Measured from the game's dialogue box (engine wrap width; includes the text box profile when enabled)"
          >Game box: {gameCaps.charsPerLine} × {gameCaps.linesPerBox}</button>
        {/if}
        <button class="btn split" onclick={() => handleReflow(false)} disabled={working} title="Revert all splits in this tab and re-wrap at the caps above">
          Re-flow this tab
        </button>
        {#if store.tabs.length > 1}
          <button class="btn split" onclick={() => handleReflow(true)} disabled={working} title="Revert + re-wrap every open tab — fix a whole game's box geometry in one pass">
            Re-flow all {store.tabs.length} tabs
          </button>
        {/if}
        {#if !canAddBoxes}
          <span class="no-bridge">No box bridge in this BIN — re-flow wraps lines only (no pagination).</span>
        {/if}
      </div>
      {#if engineRules}
        <div class="engine-note">English word wrap: the engine breaks rows itself — re-flow only splits texts into balanced boxes of {reflowLinesPerBox} rows (chars/line is not used).</div>
      {/if}
    </div>

    {#if progress}
      <div class="progress">
        <span class="spinner"></span>
        <span class="progress-text">{progress.action}…</span>
        {#if progress.total > 0}<span class="progress-count">{progress.current} / {progress.total}</span>{/if}
        {#if progress.cancellable}
          <button
            class="stop-btn"
            onclick={() => { store.bulkCancelRequested = true; store.statusMessage = "Stopping after the current file…"; }}
            disabled={store.bulkCancelRequested}
          >
            {store.bulkCancelRequested ? "Stopping…" : "Stop"}
          </button>
        {/if}
      </div>
    {/if}

    {#if engineRules}
      <div class="box-section">
        <div class="box-section-head">
          {#if engineOverflows.length === 0}
            ✓ Every box fits the game's box ({engineRules.rowsPerBox} rows{engineRules.bottomCap > 0 ? `, bottom row ≤ ${engineRules.bottomCap}` : ""}) — the engine wraps the rows itself.
          {:else}
            ⚠ <strong>{engineOverflows.length}</strong> box{engineOverflows.length === 1 ? "" : "es"} don't fit the game's box
            ({engineRules.rowsPerBox} rows{engineRules.bottomCap > 0 ? `, bottom row ≤ ${engineRules.bottomCap}` : ""}). The engine wraps rows itself, so the fix is more boxes, not line breaks.
            {#if engineOverflows.some((b) => b.singleText)}
              <button class="paginate-all" onclick={() => handleEngineSplit(engineOverflows)} disabled={working} title="Split every single-text box that doesn't fit into balanced boxes at word boundaries (adds a click per box)">
                Split into boxes (all)
              </button>
            {/if}
          {/if}
        </div>
        <div class="box-list">
          {#each engineOverflows as box (box.id)}
            <div class="box-row">
              <span class="box-badges">
                {#if box.bottomRowTooLong}
                  <span class="box-badge bad" title="The bottom row shares its row with UI art (Settings ▸ Bottom-line cap)">bottom row &gt; {engineRules.bottomCap}</span>
                {:else}
                  <span class="box-badge bad">{box.rows} rows &gt; {box.rowsPerBox}</span>
                {/if}
              </span>
              <button class="box-preview-btn" onclick={() => jumpToEngineBox(box)} title={`Go to this box (starts at row #${box.id})`}>{truncate(box.preview, 78)}</button>
              {#if box.singleText}
                <button class="row-btn" onclick={() => handleEngineSplit([box])} disabled={working} title="Split this text into balanced boxes at word boundaries">Split into boxes</button>
              {:else}
                <span class="no-bridge" title="This box holds several lines separated by the game's own line breaks (0x6F); each starts a new row. Shorten them, or move the text into the first line (go to the box to edit).">several lines: shorten</span>
              {/if}
            </div>
          {/each}
        </div>
      </div>
    {:else}
    {#if store.tabs.length > 1 && (globalOverflowCount > 0 || globalBoxOverflowCount > 0)}
      <div class="global-bar">
        <span>
          Across all {store.tabs.length} tabs:
          {#if globalOverflowCount > 0}<strong>{globalOverflowCount}</strong> over line cap{/if}{#if globalOverflowCount > 0 && globalBoxOverflowCount > 0}, {/if}{#if globalBoxOverflowCount > 0}<strong>{globalBoxOverflowCount}</strong> box{globalBoxOverflowCount === 1 ? "" : "es"} over limit{/if}
        </span>
        <div class="global-btns">
          {#if globalOverflowCount > 0}
            <button
              class="btn split"
              onclick={handleAutoSplitAllTabs}
              disabled={working || !canAutoSplit}
              title={!canAutoSplit ? "Set a per-line character cap first" : "Run the line-split across every open tab (post-import workflow)"}
            >
              Split lines — all tabs
            </button>
          {/if}
          {#if globalBoxOverflowCount > 0}
            <button
              class="btn split"
              onclick={handlePaginateAllTabs}
              disabled={working}
              title="Paginate over-long boxes into multiple boxes across every open tab (needs a box-transition bridge per BIN)"
            >
              Paginate — all tabs
            </button>
          {/if}
        </div>
      </div>
    {/if}

    {#if boxOverflows.length > 0}
      <div class="box-section">
        <div class="box-section-head">
          ⚠ <strong>{boxOverflows.length}</strong> text box{boxOverflows.length === 1 ? "" : "es"}
          exceed the per-box limits — extra lines render off-screen in-game.
          {#if boxOverflows.some((b) => b.tooManyLines || b.bottomLineTooLong) && canAddBoxes}
            <button class="paginate-all" onclick={handlePaginateAll} disabled={working} title="Split every over-long box into multiple boxes (clones the box-transition bridge; adds a click per box)">
              Paginate all
            </button>
          {:else if boxOverflows.some((b) => b.tooManyLines || b.bottomLineTooLong) && !canAddBoxes}
            {#if donorName}
              <button
                class="paginate-all"
                onclick={handlePaginateAll}
                disabled={working}
                title={`This scene has no box bridge of its own — borrows a call-free box-open from ${donorName} (control-flow stripped; verified safe for event scenes).`}
              >
                Paginate all (borrowed from {donorName})
              </button>
            {:else}
              <span class="no-bridge">No box bridge in this scene — open a scene that paginates (e.g. SC0000) in another tab to borrow one.</span>
            {/if}
          {/if}
        </div>
        <div class="box-list">
          {#each boxOverflows as box (box.id)}
            <div class="box-row">
              <span class="box-badges">
                {#if box.tooManyLines}
                  <span class="box-badge bad">{box.lines} lines &gt; {box.maxLines} max</span>
                {/if}
                {#if box.overTotalCap}
                  <span class="box-badge bad">{box.totalChars}/{box.totalCap} chars</span>
                {/if}
                {#if box.bottomLineTooLong}
                  <span class="box-badge bad" title="The bottom line shares its row with UI art (Settings ▸ Bottom-line cap)">bottom line &gt; {store.bottomLineCharCap} chars</span>
                {/if}
              </span>
              <button class="box-preview-btn" onclick={() => jumpToBox(box)} title={`Go to this box (starts at row #${box.id})`}>{truncate(box.preview, 78)}</button>
              {#if (box.tooManyLines || box.bottomLineTooLong) && canAddBoxes}
                <button class="row-btn" onclick={() => handlePaginateBox(box)} disabled={working} title="Split this box into multiple boxes every {store.maxLinesPerBox} lines (clones the box-transition bridge)">Paginate</button>
              {/if}
            </div>
          {/each}
        </div>
      </div>
    {/if}

    {#if store.translatedLineCharCap === 0}
      {#if boxOverflows.length === 0}
        <div class="empty">No per-line character cap is set. Configure one in Settings to detect line overflows.</div>
      {/if}
    {:else if overflows.length === 0}
      {#if boxOverflows.length === 0}
        <div class="empty">No overflowing rows. Everything fits within {store.translatedLineCharCap} chars per line.</div>
      {/if}
    {:else}
      <div class="summary">
        <strong>{overflows.length}</strong> row{overflows.length === 1 ? "" : "s"} exceed the {store.translatedLineCharCap}-char line cap.
        <div class="warning">Tip: "Split lines" wraps within one box ({store.maxLinesPerBox} lines); content beyond that needs "Paginate" (in the box section above, adds a click per box) — the engine won't paginate line breaks on its own.</div>
      </div>

      <div class="bulk-actions">
        <button class="btn split" onclick={handleAutoSplitAll} disabled={working || !canAutoSplit} title={!canAutoSplit ? "Set a per-line character cap first" : "Free, local — wraps overflowing rows into line breaks only (use per-row 'New box' to add a text box)"}>
          Split lines (all, free)
        </button>
        <button class="btn optimize" onclick={handleOptimizeAll} disabled={working || !store.apiKeyConfigured} title={!store.apiKeyConfigured ? "API key not configured" : "Sequential calls, billed to your API key"}>
          Optimize all (Claude API)
        </button>
      </div>

      <div class="table-scroll">
        <table>
          <thead>
            <tr>
              <th class="col-id">ID</th>
              <th class="col-len">Length</th>
              <th class="col-over">Over</th>
              <th class="col-text">Translation</th>
              <th class="col-actions">Fix</th>
            </tr>
          </thead>
          <tbody>
            {#each overflows as row (row.id)}
              <tr class:continuation={row.isContinuation}>
                <td class="col-id">
                  #{row.id}
                  {#if row.partOfGroup}
                    <span class="group-badge" title="Part of a multi-entry line group (e.g. show-text + furigana)">grouped</span>
                  {/if}
                </td>
                <td class="col-len">{row.length}/{row.cap}</td>
                <td class="col-over"><span class="over-badge">+{row.overflow}</span></td>
                <td class="col-text">
                  <button
                    type="button"
                    class="jump-text"
                    title={`Go to row #${row.id} in the editor — ${row.entry.translated_text}`}
                    onclick={() => jumpToRow(row)}
                  >{truncate(row.entry.translated_text, 100)}</button>
                </td>
                <td class="col-actions">
                  <button
                    class="row-btn"
                    onclick={() => handleAutoSplitRow(row)}
                    disabled={working || row.isContinuation || !row.splittable || !canAutoSplit}
                    title={
                      row.isContinuation ? "Already a continuation row" :
                      !row.splittable ? "Cannot split this row" :
                      !canAutoSplit ? "Set a per-line character cap first" :
                      "Wrap the overflow into multiple display lines (line breaks only — no new box)"
                    }
                  >
                    Split lines
                  </button>
                  <button class="row-btn" onclick={() => handleOptimizeRow(row)} disabled={working || !store.apiKeyConfigured}>
                    Optimize
                  </button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
    {/if}

    <footer>
      <button class="btn cancel" onclick={handleClose}>Close</button>
    </footer>
  </div>
</div>

<style>
  .engine-note {
    margin-top: 6px;
    font-size: 0.8rem;
    color: #9ad0a0;
  }

  .dialog-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .dialog {
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    width: 880px;
    max-width: 90vw;
    max-height: 85vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }

  header {
    padding: 14px 20px;
    border-bottom: 1px solid #2a2a4a;
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  h2 {
    margin: 0;
    color: #8ab4f8;
    font-size: 1.05rem;
  }

  .close-btn {
    background: none;
    border: none;
    color: #a0a0c0;
    font-size: 1.4rem;
    cursor: pointer;
    padding: 0 6px;
    line-height: 1;
  }

  .close-btn:hover {
    color: #fff;
  }

  .empty {
    padding: 32px 24px;
    text-align: center;
    color: #a0a0c0;
    font-size: 0.9rem;
  }

  .box-section {
    border-bottom: 1px solid #2a2a4a;
    background: #2a1a1a;
  }
  .box-section-head {
    padding: 10px 20px;
    font-size: 0.83rem;
    color: #f0b090;
    line-height: 1.4;
  }
  .box-section-head strong { color: #f0a070; }
  .box-list {
    max-height: 180px;
    overflow-y: auto;
    padding: 0 12px 8px;
  }
  .box-row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 4px 8px;
  }
  .box-row:hover { background: #3a2424; }
  .box-badges { flex-shrink: 0; display: flex; gap: 4px; }
  .box-badge {
    display: inline-block;
    padding: 1px 6px;
    border-radius: 3px;
    font-size: 0.72rem;
    font-weight: 600;
    white-space: nowrap;
  }
  .box-badge.bad { background: #5a2a2a; color: #f0a070; }
  .box-preview-btn {
    flex: 1;
    min-width: 0;
    text-align: left;
    background: none;
    border: none;
    padding: 0;
    color: #b0b0c8;
    font: inherit;
    font-size: 0.78rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .box-preview-btn:hover { color: #8ab4f8; text-decoration: underline; }
  .paginate-all {
    margin-left: 8px;
    padding: 3px 10px;
    border: 1px solid #6c9bd2;
    border-radius: 3px;
    background: #1a1a3e;
    color: #8ab4f8;
    cursor: pointer;
    font-size: 0.75rem;
  }
  .paginate-all:hover:not(:disabled) { background: #2a2a5e; }
  .paginate-all:disabled { opacity: 0.4; cursor: not-allowed; }
  .no-bridge { margin-left: 8px; color: #f0a070; font-size: 0.78rem; }

  .summary {
    padding: 12px 20px;
    font-size: 0.85rem;
    color: #c0c0e0;
    border-bottom: 1px solid #2a2a4a;
  }

  .summary strong {
    color: #f0a070;
  }

  .warning {
    margin-top: 6px;
    color: #f0a070;
    font-size: 0.8rem;
  }

  .reflow-bar {
    padding: 10px 20px;
    background: #16261a;
    border-bottom: 1px solid #2a4a30;
  }
  .reflow-head {
    font-size: 0.83rem;
    color: #a0d0a8;
    line-height: 1.4;
    margin-bottom: 8px;
  }
  .reflow-head strong { color: #b8e8c0; }
  .reflow-controls {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .reflow-controls label {
    font-size: 0.8rem;
    color: #c0c0e0;
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .reflow-controls input {
    width: 56px;
    padding: 3px 6px;
    background: #0e1530;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    color: #e0e0f0;
    font-size: 0.8rem;
  }

  .global-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 20px;
    background: #16213e;
    border-bottom: 1px solid #2a2a4a;
    font-size: 0.85rem;
    color: #c0c0e0;
  }
  .global-bar strong { color: #f0a070; }
  .global-btns { display: flex; gap: 8px; flex-shrink: 0; }

  .bulk-actions {
    padding: 10px 20px;
    display: flex;
    gap: 8px;
    border-bottom: 1px solid #2a2a4a;
  }

  .progress {
    padding: 10px 20px;
    color: #8ab4f8;
    font-size: 0.85rem;
    border-bottom: 1px solid #2a2a4a;
    background: #0e1530;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .progress-text { flex: 1; }
  .progress-count {
    color: #b0c4e8;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid #2a3a5e;
    border-top-color: #8ab4f8;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    flex-shrink: 0;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .stop-btn {
    padding: 4px 14px;
    border: 1px solid #d27c7c;
    border-radius: 4px;
    background: #2a1414;
    color: #f0a0a0;
    cursor: pointer;
    font-size: 0.8rem;
    flex-shrink: 0;
  }
  .stop-btn:hover:not(:disabled) { background: #3a1a1a; }
  .stop-btn:disabled { opacity: 0.5; cursor: not-allowed; }

  .table-scroll {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.82rem;
  }

  th {
    position: sticky;
    top: 0;
    background: #16213e;
    color: #8ab4f8;
    padding: 8px 10px;
    text-align: left;
    font-weight: 600;
    border-bottom: 1px solid #2a2a4a;
  }

  td {
    padding: 6px 10px;
    color: #c0c0e0;
    border-bottom: 1px solid #20204a;
    vertical-align: top;
  }

  tr.continuation td {
    opacity: 0.6;
  }

  .col-id {
    width: 110px;
    color: #6c9bd2;
    font-family: monospace;
  }

  .group-badge {
    display: inline-block;
    margin-left: 4px;
    padding: 1px 5px;
    background: #2a3a5e;
    color: #8ab4f8;
    border-radius: 2px;
    font-size: 0.65rem;
    font-family: sans-serif;
    font-weight: 600;
  }

  .col-len {
    width: 80px;
    font-family: monospace;
  }

  .col-over {
    width: 60px;
  }

  .col-actions {
    width: 150px;
    white-space: nowrap;
  }

  .over-badge {
    display: inline-block;
    padding: 2px 6px;
    border-radius: 3px;
    background: #5a2a2a;
    color: #f0a070;
    font-size: 0.75rem;
    font-weight: 600;
  }

  .col-text {
    color: #d0d0e0;
    line-height: 1.4;
  }

  .jump-text {
    display: block;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    padding: 0;
    margin: 0;
    color: #d0d0e0;
    font: inherit;
    line-height: 1.4;
    cursor: pointer;
  }

  .jump-text:hover {
    color: #8ab4f8;
    text-decoration: underline;
  }

  footer {
    padding: 12px 20px;
    border-top: 1px solid #2a2a4a;
    display: flex;
    justify-content: flex-end;
  }

  .btn {
    padding: 6px 14px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.85rem;
  }

  .btn:hover:not(:disabled) {
    background: #2a2a5e;
  }

  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .btn.split {
    border-color: #6c9bd2;
    color: #8ab4f8;
  }

  .btn.optimize {
    border-color: #d2a06c;
    color: #f0c080;
  }

  .row-btn {
    padding: 3px 8px;
    margin-right: 4px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #20204a;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.75rem;
  }

  .row-btn:hover:not(:disabled) {
    background: #2a2a5e;
  }

  .row-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
