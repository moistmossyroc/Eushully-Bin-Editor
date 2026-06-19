<script lang="ts">
  import { store } from "../lib/stores.svelte";
  import { findOverflows, findBoxOverflows, entryDisplayLength, type OverflowRow, type BoxOverflow } from "../lib/overflow";
  import { autoSplitEntry, optimizeEntry } from "../lib/ipc";
  import { applySplitPlan, splitOverflowingGroup } from "../lib/insertions";

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
    return findBoxOverflows(tab.entries, store.translatedCharCap, store.maxLinesPerBox);
  });

  let working = $state(false);
  let progress = $state<{ current: number; total: number; action: string } | null>(null);

  let templateQuality = $derived(store.activeTab?.bridgeTemplateQuality);
  // Line-splitting (wrapping within one box) needs no bridge — only a per-line
  // cap. A bridge is required only to spill into an EXTRA box.
  let canAutoSplit = $derived(store.translatedLineCharCap > 0);
  let canAddBoxes = $derived(
    templateQuality === "clean" || templateQuality === "degraded",
  );

  /** Handle the per-row "Split" button.
   *
   *  Two cases:
   *  - If the row's own text exceeds the cap → call Rust auto-split to wrap
   *    the text into multiple lines/boxes.
   *  - Else, if the row is part of an over-cap group → insert end-text-line
   *    opcodes inside the group to break it into multiple display lines.
   */
  async function handleAutoSplitRow(row: OverflowRow) {
    const tab = store.activeTab;
    if (!tab || working) return;
    working = true;
    try {
      if (row.individualLength > store.translatedLineCharCap) {
        // Single-entry text overflow → use Rust auto-split (wrap text).
        const plan = await autoSplitEntry(
          tab.id,
          row.entry.translated_text,
          row.id,
          store.translatedLineCharCap,
          store.translatedCharCap,
          store.maxLinesPerBox,
        );
        const result = applySplitPlan(plan);
        if (typeof result === "string") {
          store.statusMessage = result;
        }
      } else if (row.partOfGroup) {
        // Group overflow → insert end-text-line between entries, bridge to
        // a new box if needed to respect max-lines-per-box.
        const result = await splitOverflowingGroup(
          row.entry,
          store.translatedLineCharCap,
          store.maxLinesPerBox,
        );
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

    working = true;
    progress = { current: 0, total: candidateCount, action: "Splitting" };
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
    const ok = confirm(
      `Optimize ${toFix.length} overflowing rows via Claude API?\n\n` +
      `Each row uses one API call. This will be billed to your API key.`,
    );
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

<div class="dialog-backdrop" onclick={onClose} onkeydown={(e) => e.key === "Escape" && onClose()} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <header>
      <h2>Overflow Review</h2>
      <button class="close-btn" onclick={onClose}>×</button>
    </header>

    {#if boxOverflows.length > 0}
      <div class="box-section">
        <div class="box-section-head">
          ⚠ <strong>{boxOverflows.length}</strong> text box{boxOverflows.length === 1 ? "" : "es"}
          exceed the per-box limits — these won't render fully in-game. Jump in and shorten or merge a line.
        </div>
        <div class="box-list">
          {#each boxOverflows as box (box.id)}
            <button class="box-row" onclick={() => jumpToBox(box)} title={`Go to this box (starts at row #${box.id})`}>
              <span class="box-badges">
                {#if box.tooManyLines}
                  <span class="box-badge bad">{box.lines} lines &gt; {box.maxLines} max</span>
                {/if}
                {#if box.overTotalCap}
                  <span class="box-badge bad">{box.totalChars}/{box.totalCap} chars</span>
                {/if}
              </span>
              <span class="box-preview">{truncate(box.preview, 90)}</span>
            </button>
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
        {#if !canAddBoxes}
          <div class="warning">⚠ No textbox bridge in this BIN — long lines still wrap within one box, but any text exceeding one box ({store.maxLinesPerBox} lines) can't spill into a new box and must be shortened.</div>
        {/if}
      </div>

      <div class="bulk-actions">
        <button class="btn split" onclick={handleAutoSplitAll} disabled={working || !canAutoSplit} title={!canAutoSplit ? "Set a per-line character cap first" : "Free, local — wraps lines and (where supported) splits into multiple boxes"}>
          Auto-split all (free)
        </button>
        <button class="btn optimize" onclick={handleOptimizeAll} disabled={working || !store.apiKeyConfigured} title={!store.apiKeyConfigured ? "API key not configured" : "Sequential calls, billed to your API key"}>
          Optimize all (Claude API)
        </button>
      </div>

      {#if progress}
        <div class="progress">
          {progress.action} {progress.current}/{progress.total}…
        </div>
      {/if}

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
                      !row.splittable ? "Cannot auto-split this row" :
                      !canAutoSplit ? "Set a per-line character cap first" :
                      row.individualLength > row.cap ? "Wrap or split text into multiple lines/boxes" :
                      row.partOfGroup ? "Insert end-text-line to break the group into multiple display lines" :
                      "Split"
                    }
                  >
                    Split
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

    <footer>
      <button class="btn cancel" onclick={onClose}>Close</button>
    </footer>
  </div>
</div>

<style>
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
    align-items: baseline;
    gap: 10px;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    border-radius: 3px;
    color: #d0d0e0;
    font: inherit;
    padding: 5px 8px;
    cursor: pointer;
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
  .box-preview {
    color: #b0b0c8;
    font-size: 0.78rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

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

  .bulk-actions {
    padding: 10px 20px;
    display: flex;
    gap: 8px;
    border-bottom: 1px solid #2a2a4a;
  }

  .progress {
    padding: 8px 20px;
    color: #8ab4f8;
    font-size: 0.85rem;
    border-bottom: 1px solid #2a2a4a;
    background: #0e1530;
  }

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
