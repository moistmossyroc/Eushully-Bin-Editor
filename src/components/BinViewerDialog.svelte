<script lang="ts">
  import { tick } from "svelte";
  import { store } from "../lib/stores.svelte";
  import { getAllInstructions, type BinInstruction } from "../lib/ipc";
  import { persistAllSettings } from "../lib/settings";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  let instructions = $state<BinInstruction[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let searchQuery = $state("");
  let searchField = $state<"all" | "label" | "string">("all");

  // Virtual scrolling state (scroll mode). Fixed row height lets us render
  // only the slice currently in the viewport (+ a small buffer) regardless of
  // the BIN's total instruction count. Eliminates the progressive-renderer
  // race that the click-to-jump path had to dance around.
  const ROW_HEIGHT = 28;
  const VIEWPORT_BUFFER = 12; // rows rendered above + below the visible area
  let scrollTop = $state(0);
  let containerHeight = $state(600);

  // Pagination state (paged mode)
  const PAGE_SIZE = 200;
  let currentPage = $state(0);

  // Jump-to-context state. When the user clicks a filtered row, we stash the
  // active query so they can restore it, clear the filter to reveal the full
  // BIN, scroll the target row into view, and flash it. The stashed query also
  // drives keyboard ↑/↓ stepping through the prior match set.
  let stashedQuery = $state("");
  let stashedField = $state<"all" | "label" | "string">("all");
  let pendingJumpIndex = $state<number | null>(null);
  let tableContainerEl = $state<HTMLDivElement | null>(null);

  /** Shared matcher used by both `filtered` and `matchIndices`. Tests label,
   *  hex opcode, decoded string args, AND the formatted arg display
   *  (`lit:0x37D0`, `g-int:0x5E4` etc.) so the user's query matches anything
   *  visible in the args column — including type prefixes like "lit:". */
  function instructionMatches(instr: BinInstruction, q: string, field: "all" | "label" | "string"): boolean {
    if (field === "label" || field === "all") {
      if (instr.label.toLowerCase().includes(q)) return true;
      if (("0x" + instr.opcode.toString(16)).includes(q)) return true;
    }
    if (field === "string" || field === "all") {
      for (const arg of instr.arguments) {
        if (arg.decoded_string && arg.decoded_string.toLowerCase().includes(q)) return true;
        // Also match against the formatted arg display so the user can search
        // for things like "lit:", "g-int:", "0x37D0", etc. — the same text
        // they see in the Arguments column.
        if (formatArg(arg).toLowerCase().includes(q)) return true;
      }
    }
    return false;
  }

  /** Indices (in the full `instructions` array) that match the stashed or
   *  active query — used for ↑/↓ keyboard stepping. */
  let matchIndices = $derived.by<number[]>(() => {
    const q = (stashedQuery || searchQuery).trim().toLowerCase();
    if (!q) return [];
    const field = stashedQuery ? stashedField : searchField;
    const out: number[] = [];
    for (let i = 0; i < instructions.length; i++) {
      if (instructionMatches(instructions[i], q, field)) out.push(i);
    }
    return out;
  });

  let currentMatchPos = $derived.by(() => {
    if (pendingJumpIndex === null) return -1;
    return matchIndices.indexOf(pendingJumpIndex);
  });

  /** Filter pass yields BOTH the matching instructions and their original
   *  indices in one walk. The indices are needed by the virtualized row
   *  renderer so click-to-jump lands on the real instruction position (not
   *  the position within the filtered subset). `indices === null` means the
   *  filter is empty, so `filteredIdx === instructionIdx` trivially. */
  let filteredView = $derived.by<{ items: BinInstruction[]; indices: number[] | null }>(() => {
    if (!searchQuery.trim()) {
      return { items: instructions, indices: null };
    }
    const q = searchQuery.trim().toLowerCase();
    const items: BinInstruction[] = [];
    const indices: number[] = [];
    for (let i = 0; i < instructions.length; i++) {
      if (instructionMatches(instructions[i], q, searchField)) {
        items.push(instructions[i]);
        indices.push(i);
      }
    }
    return { items, indices };
  });
  let filtered = $derived(filteredView.items);

  /** Translate a position within `filtered` back to a real instruction index. */
  function toInstrIdx(filteredIdx: number): number {
    return filteredView.indices === null ? filteredIdx : filteredView.indices[filteredIdx];
  }

  let totalPages = $derived(Math.max(1, Math.ceil(filtered.length / PAGE_SIZE)));

  /** Inclusive-start, exclusive-end range of `filtered` to actually mount in
   *  the DOM. In paged mode it's the current page; in scroll mode it's the
   *  viewport ± a small over-render buffer. */
  let visibleRange = $derived.by(() => {
    if (store.binViewerMode === "paged") {
      const start = currentPage * PAGE_SIZE;
      return { start, end: Math.min(start + PAGE_SIZE, filtered.length) };
    }
    const visibleRows = Math.ceil(containerHeight / ROW_HEIGHT) + VIEWPORT_BUFFER * 2;
    const firstVisible = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - VIEWPORT_BUFFER);
    return { start: firstVisible, end: Math.min(firstVisible + visibleRows, filtered.length) };
  });

  let visibleItems = $derived(filtered.slice(visibleRange.start, visibleRange.end));

  /** Heights for the top and bottom sentinel rows that reserve scroll space
   *  for the rows that aren't currently mounted. Together they make the
   *  scrollbar behave as if every row were present. */
  let topSpacerHeight = $derived(
    store.binViewerMode === "scroll" ? visibleRange.start * ROW_HEIGHT : 0
  );
  let bottomSpacerHeight = $derived(
    store.binViewerMode === "scroll"
      ? Math.max(0, filtered.length - visibleRange.end) * ROW_HEIGHT
      : 0
  );

  // (Page-reset on search was previously a reactive effect, but it fired
  // whenever `searchQuery` changed for ANY reason — including the programmatic
  // clear inside jumpToInstruction — which overwrote the just-set target page
  // back to 0. We now reset currentPage explicitly from the search input's
  // oninput handler instead, so programmatic search changes don't clobber it.)

  /** Keep `containerHeight` in sync with the scroll container's actual size
   *  so virtual-scroll math doesn't drift when the user resizes the window
   *  or toggles side panels that take horizontal/vertical room. */
  $effect(() => {
    if (!tableContainerEl) return;
    const measure = () => {
      containerHeight = tableContainerEl?.clientHeight ?? 600;
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(tableContainerEl);
    return () => ro.disconnect();
  });

  function handleScroll(e: Event) {
    if (store.binViewerMode !== "scroll") return;
    scrollTop = (e.target as HTMLDivElement).scrollTop;
  }

  $effect(() => {
    loadInstructions();
  });

  /** Consume any pending "Show in BIN Viewer" jump request from the editor
   *  context menu. Waits until instructions have loaded, then performs the
   *  same stash+scroll+flash flow used by row-click jumps. */
  $effect(() => {
    const req = store.pendingBinViewerJump;
    if (!req || loading || instructions.length === 0) return;
    const tab = store.activeTab;
    if (!tab || tab.id !== req.tabId) return;
    if (req.instructionIndex < 0 || req.instructionIndex >= instructions.length) {
      store.pendingBinViewerJump = null;
      return;
    }
    jumpToInstruction(req.instructionIndex);
    if (store.pendingBinViewerJump === req) {
      store.pendingBinViewerJump = null;
    }
  });

  async function loadInstructions() {
    const tab = store.activeTab;
    if (!tab) return;
    try {
      loading = true;
      instructions = await getAllInstructions(tab.id);
      loading = false;
    } catch (err) {
      error = String(err);
      loading = false;
    }
  }

  function formatArg(arg: { arg_type: number; raw_data: number; decoded_string?: string }): string {
    const typeNames: Record<number, string> = {
      0: "lit", 1: "flt", 2: "str", 3: "g-int", 4: "g-flt",
      5: "g-str", 6: "g-int2", 9: "cmp", 0xB: "l-str",
      0xE: "tmp", 0xF: "tmp-f", 0x14: "t-str",
    };
    const typeName = typeNames[arg.arg_type] ?? `t${arg.arg_type}`;
    if (arg.decoded_string !== undefined) {
      return `${typeName}:"${arg.decoded_string}"`;
    }
    return `${typeName}:0x${arg.raw_data.toString(16).toUpperCase()}`;
  }

  /** Click an arg.
   *  - In filtered mode: treat as the same intent as a row click — jump to
   *    the result. Otherwise the most-clickable area of the row (the args
   *    column) silently hijacks the navigation and the user can't reach the
   *    result they searched for.
   *  - In unfiltered mode: filter the list by the arg's value (text for
   *    decoded strings, hex for everything else).
   */
  function handleArgClick(e: MouseEvent, arg: { arg_type: number; raw_data: number; decoded_string?: string }, globalIdx: number) {
    e.stopPropagation();
    if (searchQuery.trim()) {
      jumpToInstruction(globalIdx);
      return;
    }
    // If we're in stash-restored state with no active filter, clear the stash
    // first so the user's chip-click query isn't masked.
    if (stashedQuery) {
      stashedQuery = "";
      pendingJumpIndex = null;
    }
    if (arg.decoded_string !== undefined && arg.decoded_string.length > 0) {
      searchQuery = arg.decoded_string;
      searchField = "string";
    } else {
      searchQuery = "0x" + arg.raw_data.toString(16).toUpperCase();
      searchField = "all";
    }
  }

  /** Jump to instruction at globalIdx: stash current filter (if any), clear
   *  search, ensure the row is on the visible page (paged mode), scroll-into-
   *  view, and flash it. */
  function jumpToInstruction(globalIdx: number) {
    // Set the jump target FIRST so the progressive-rendering $effect (which
    // re-runs when searchQuery clears) sees it and extends renderLimit to
    // cover the target row instead of resetting to the default CHUNK. Without
    // this ordering, the effect would overwrite any later renderLimit bump
    // and the scrollIntoView would miss because the target row hadn't been
    // rendered into the DOM yet.
    pendingJumpIndex = globalIdx;
    if (searchQuery.trim()) {
      stashedQuery = searchQuery;
      stashedField = searchField;
      searchQuery = "";
    }
    if (store.binViewerMode === "paged") {
      currentPage = Math.floor(globalIdx / PAGE_SIZE);
    }
    // Defer to next frame so the render-effect can run and the DOM can update
    // before we try to scroll.
    queueMicrotask(() => scrollToAndFlash(globalIdx));
  }

  function restoreFilter() {
    if (!stashedQuery) return;
    searchQuery = stashedQuery;
    searchField = stashedField;
    stashedQuery = "";
    pendingJumpIndex = null;
  }

  function clearStash() {
    stashedQuery = "";
    pendingJumpIndex = null;
  }

  /** Scroll the target instruction into view and briefly highlight it.
   *  In scroll mode we set `scrollTop` directly — virtual scrolling means the
   *  row is guaranteed to mount as soon as the viewport range covers it,
   *  removing the polling/race the progressive renderer needed. In paged mode
   *  the page is already correct (set in jumpToInstruction); we just wait one
   *  tick for the DOM to settle, then flash. */
  async function scrollToAndFlash(globalIdx: number) {
    if (store.binViewerMode === "scroll" && tableContainerEl) {
      // Center the target row in the visible area.
      const target = Math.max(0, globalIdx * ROW_HEIGHT - containerHeight / 2);
      tableContainerEl.scrollTo({ top: target, behavior: "auto" });
      // Update local state immediately so visibleRange recomputes this turn
      // rather than waiting for the scroll event's next firing.
      scrollTop = target;
    }
    await tick();
    // One RAF gives Svelte time to render the now-mounted row.
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    const row = (tableContainerEl ?? document)
      .querySelector(`[data-global-idx="${globalIdx}"]`) as HTMLElement | null;
    if (!row) return;
    row.classList.remove("jump-flash");
    void row.offsetWidth; // force reflow so flash re-triggers if same row
    row.classList.add("jump-flash");
    setTimeout(() => row?.classList.remove("jump-flash"), 1600);
  }

  function stepMatch(direction: 1 | -1) {
    if (matchIndices.length === 0) return;
    const pos = currentMatchPos;
    let nextPos: number;
    if (pos < 0) nextPos = direction === 1 ? 0 : matchIndices.length - 1;
    else nextPos = (pos + direction + matchIndices.length) % matchIndices.length;
    const targetIdx = matchIndices[nextPos];
    // If we're currently filtered, treat this as a jump (stash + clear).
    // If we're already stashed, just re-scroll without touching the stash.
    if (searchQuery.trim()) {
      jumpToInstruction(targetIdx);
    } else {
      pendingJumpIndex = targetIdx;
      if (store.binViewerMode === "paged") {
        currentPage = Math.floor(targetIdx / PAGE_SIZE);
      }
      queueMicrotask(() => scrollToAndFlash(targetIdx));
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    // Don't hijack arrow keys when typing in the search box (let cursor move).
    const target = e.target as HTMLElement | null;
    const inSearch = target?.tagName === "INPUT" && (target as HTMLInputElement).type === "text";
    if (e.key === "Escape") {
      onClose();
      return;
    }
    if ((e.key === "ArrowDown" || e.key === "ArrowUp") && matchIndices.length > 0) {
      // Allow ↑/↓ in search only with modifier (avoid stealing caret motion)
      if (inSearch && !e.altKey) return;
      e.preventDefault();
      stepMatch(e.key === "ArrowDown" ? 1 : -1);
    }
  }

  let backdropMouseDown = false;
  function handleBackdropMouseDown(e: MouseEvent) {
    backdropMouseDown = (e.target as HTMLElement).classList.contains("dialog-backdrop");
  }
  function handleBackdropMouseUp(e: MouseEvent) {
    if (backdropMouseDown && (e.target as HTMLElement).classList.contains("dialog-backdrop")) {
      onClose();
    }
    backdropMouseDown = false;
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="dialog-backdrop" onmousedown={handleBackdropMouseDown} onmouseup={handleBackdropMouseUp}>
  <div class="dialog">
    <div class="dialog-header">
      <h2>BIN Viewer — {store.activeTab?.sourceBinFilename}</h2>
      <span class="count">{filtered.length.toLocaleString()} instructions</span>
      <div class="mode-toggle" title="Display mode — Scroll uses virtualized rendering for big BINs; Paged splits into fixed pages.">
        <button
          class="mode-btn"
          class:active={store.binViewerMode === "scroll"}
          onclick={() => { store.binViewerMode = "scroll"; void persistAllSettings(); }}
        >Scroll</button>
        <button
          class="mode-btn"
          class:active={store.binViewerMode === "paged"}
          onclick={() => { store.binViewerMode = "paged"; void persistAllSettings(); }}
        >Paged</button>
      </div>
      <button class="close-btn" onclick={onClose}>&times;</button>
    </div>

    <div class="search-bar">
      <input
        type="text"
        class="search-input"
        placeholder="Search instructions..."
        value={searchQuery}
        oninput={(e) => {
          searchQuery = (e.target as HTMLInputElement).value;
          currentPage = 0;
          // A fresh user-typed query also invalidates any pending jump state.
          if (stashedQuery) { stashedQuery = ""; }
          pendingJumpIndex = null;
        }}
      />
      <select
        class="search-field"
        value={searchField}
        onchange={(e) => {
          searchField = (e.target as HTMLSelectElement).value as "all" | "label" | "string";
          currentPage = 0;
          if (stashedQuery) { stashedQuery = ""; }
          pendingJumpIndex = null;
        }}
      >
        <option value="all">All</option>
        <option value="label">Label/Opcode</option>
        <option value="string">Strings</option>
      </select>
    </div>

    {#if stashedQuery || (searchQuery.trim() && matchIndices.length > 0)}
      <div class="match-bar">
        {#if stashedQuery}
          <span class="stash-label">
            Showing all — filter stashed: <strong>"{stashedQuery}"</strong>
          </span>
          <button class="link-btn" onclick={restoreFilter} title="Re-apply the stashed search">
            Restore filter
          </button>
          <button class="link-btn dim" onclick={clearStash} title="Clear stashed filter">×</button>
        {:else}
          <span class="stash-label">{matchIndices.length} matches</span>
        {/if}
        <span class="step-controls">
          <button class="step-btn" onclick={() => stepMatch(-1)} disabled={matchIndices.length === 0} title="Previous match (↑)">↑</button>
          <span class="step-pos">
            {currentMatchPos >= 0 ? currentMatchPos + 1 : "–"} / {matchIndices.length}
          </span>
          <button class="step-btn" onclick={() => stepMatch(1)} disabled={matchIndices.length === 0} title="Next match (↓)">↓</button>
        </span>
      </div>
    {/if}

    {#if store.binViewerMode === "paged" && !loading && !error}
      <div class="page-controls">
        <button class="page-btn" disabled={currentPage === 0} onclick={() => currentPage = 0}>&laquo;</button>
        <button class="page-btn" disabled={currentPage === 0} onclick={() => currentPage--}>&lsaquo;</button>
        <span class="page-info">Page {currentPage + 1} / {totalPages}</span>
        <button class="page-btn" disabled={currentPage >= totalPages - 1} onclick={() => currentPage++}>&rsaquo;</button>
        <button class="page-btn" disabled={currentPage >= totalPages - 1} onclick={() => currentPage = totalPages - 1}>&raquo;</button>
      </div>
    {/if}

    <div class="table-container" bind:this={tableContainerEl} onscroll={handleScroll}>
      {#if loading}
        <div class="loading">Loading instructions...</div>
      {:else if error}
        <div class="error">{error}</div>
      {:else}
        <table class="instr-table">
          <thead>
            <tr>
              <th class="th-idx">#</th>
              <th class="th-offset">Offset</th>
              <th class="th-opcode">Opcode</th>
              <th class="th-label">Label</th>
              <th class="th-args">Arguments</th>
            </tr>
          </thead>
          <tbody>
            {#if topSpacerHeight > 0}
              <tr class="spacer" style="height: {topSpacerHeight}px"><td colspan="5"></td></tr>
            {/if}
            {#each visibleItems as instr, i (visibleRange.start + i)}
              {@const globalIdx = toInstrIdx(visibleRange.start + i)}
              <tr
                data-global-idx={globalIdx}
                class:clickable={searchQuery.trim().length > 0}
                onclick={() => searchQuery.trim() && jumpToInstruction(globalIdx)}
                title={searchQuery.trim() ? "Click to jump to this instruction in context" : ""}
              >
                <td class="cell-idx">{globalIdx}</td>
                <td class="cell-offset">0x{instr.offset.toString(16).toUpperCase()}</td>
                <td class="cell-opcode">0x{instr.opcode.toString(16).toUpperCase()}</td>
                <td class="cell-label">{instr.label}</td>
                <td class="cell-args">
                  {#each instr.arguments as arg, ai}
                    <button
                      type="button"
                      class="arg arg-btn"
                      class:arg-string={arg.decoded_string !== undefined}
                      onclick={(e) => handleArgClick(e, arg, globalIdx)}
                      title="Click to find all instructions touching this value"
                    >{formatArg(arg)}</button>{ai < instr.arguments.length - 1 ? ", " : ""}
                  {/each}
                </td>
              </tr>
            {/each}
            {#if bottomSpacerHeight > 0}
              <tr class="spacer" style="height: {bottomSpacerHeight}px"><td colspan="5"></td></tr>
            {/if}
          </tbody>
        </table>
      {/if}
    </div>

  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 200;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .dialog {
    width: 90vw;
    max-width: 1200px;
    height: 85vh;
    background: #1a1a2e;
    border: 1px solid #3a3a5a;
    border-radius: 8px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .dialog-header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    border-bottom: 1px solid #2a2a4a;
    background: #16213e;
  }

  .dialog-header h2 {
    margin: 0;
    font-size: 1rem;
    color: #8ab4f8;
    font-weight: 600;
  }

  .count {
    font-size: 0.8rem;
    color: #888;
  }

  /* Inline display-mode toggle (replaces the old Settings>Display tab).
   * Two-button segmented control; selected mode persisted on click. */
  .mode-toggle {
    margin-left: auto;
    display: inline-flex;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    overflow: hidden;
  }
  .mode-btn {
    background: #1a1a3e;
    border: none;
    color: #8a8aaa;
    padding: 4px 10px;
    font-size: 0.78rem;
    cursor: pointer;
  }
  .mode-btn:hover { background: #2a2a5e; color: #c0c0e0; }
  .mode-btn.active {
    background: #2a3a6e;
    color: #fff;
  }
  .mode-btn + .mode-btn { border-left: 1px solid #3a3a5a; }

  .close-btn {
    background: none;
    border: none;
    color: #888;
    font-size: 1.4rem;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }

  .close-btn:hover {
    color: #e0e0e0;
  }

  .search-bar {
    display: flex;
    gap: 8px;
    padding: 8px 16px;
    border-bottom: 1px solid #2a2a4a;
  }

  .search-input {
    flex: 1;
    padding: 6px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #12122a;
    color: #c0c0e0;
    font-size: 0.85rem;
    outline: none;
  }

  .search-input:focus {
    border-color: #6c9bd2;
  }

  .search-field {
    padding: 6px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #12122a;
    color: #c0c0e0;
    font-size: 0.8rem;
    outline: none;
  }

  .table-container {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    /* contain layout/paint so the virtual-scroll spacer rows can grow tall
     * (up to total_instructions * ROW_HEIGHT, easily 1M+ px) without the
     * browser repainting the entire dialog on every scroll tick. */
    contain: strict;
  }

  .loading, .error {
    padding: 40px;
    text-align: center;
    color: #888;
    font-size: 0.9rem;
  }

  .error {
    color: #f06060;
  }

  .instr-table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
  }

  .instr-table tr {
    /* Fixed row height is REQUIRED for virtual-scroll math (scrollTop is
     * mapped to row index via ROW_HEIGHT). Don't change without updating
     * the JS constant. */
    height: 28px;
  }
  .instr-table tr.spacer {
    /* Spacer rows reserve scrollable space for the rows we haven't mounted.
     * Inline height (variable) overrides the .tr 28px rule. */
    height: auto;
  }
  .instr-table tr.spacer td {
    border: none;
    padding: 0;
  }

  thead {
    position: sticky;
    top: 0;
    z-index: 5;
  }

  th {
    padding: 6px 10px;
    text-align: left;
    font-size: 0.75rem;
    font-weight: 600;
    color: #8ab4f8;
    background: #16213e;
    border-bottom: 2px solid #3a3a5a;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .th-idx { width: 60px; }
  .th-offset { width: 80px; }
  .th-opcode { width: 70px; }
  .th-label { width: 150px; }

  td {
    padding: 3px 10px;
    border-bottom: 1px solid #222240;
    font-size: 0.8rem;
    vertical-align: middle;
    /* Clip overflowing cells so all rows stay at the fixed 28px height the
     * virtualizer assumes. The args column gets `title=` for the truncated
     * content so users can still inspect long arg lists on hover. */
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  tr.clickable { cursor: pointer; }
  tr.clickable:hover { background: #1d1d36; }

  tr.jump-flash {
    animation: jumpFlash 1.5s ease-out;
  }
  @keyframes jumpFlash {
    0%   { background-color: rgba(255, 200, 90, 0.55); }
    100% { background-color: transparent; }
  }

  .match-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 16px;
    border-bottom: 1px solid #2a2a4a;
    background: #1a2240;
    font-size: 0.78rem;
    color: #c0c0e0;
  }
  .stash-label { flex: 0 0 auto; }
  .stash-label strong { color: #ffd060; font-weight: 600; }
  .link-btn {
    background: transparent;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    color: #8aa0ff;
    padding: 2px 8px;
    font-size: 0.75rem;
    cursor: pointer;
  }
  .link-btn:hover { background: #2a2a5e; color: #b0c0ff; }
  .link-btn.dim { color: #777; padding: 2px 6px; }

  .step-controls {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .step-btn {
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    color: #c0c0e0;
    padding: 2px 8px;
    font-size: 0.8rem;
    cursor: pointer;
    font-family: monospace;
  }
  .step-btn:hover:not(:disabled) { background: #2a2a5e; }
  .step-btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .step-pos {
    font-family: "Consolas", monospace;
    font-size: 0.75rem;
    color: #8a8aaa;
    min-width: 50px;
    text-align: center;
  }

  .cell-idx {
    color: #555;
    font-family: "Consolas", "Courier New", monospace;
    font-size: 0.75rem;
  }

  .cell-offset, .cell-opcode {
    font-family: "Consolas", "Courier New", monospace;
    color: #8ab4f8;
    font-size: 0.75rem;
  }

  .cell-label {
    color: #6cc070;
    font-family: "Consolas", "Courier New", monospace;
    font-size: 0.8rem;
  }

  .cell-args {
    color: #c0c0e0;
    font-family: "Consolas", "Courier New", monospace;
    font-size: 0.75rem;
    word-break: break-word;
  }

  .arg-btn {
    background: transparent;
    border: 1px solid transparent;
    color: inherit;
    font: inherit;
    padding: 0 2px;
    border-radius: 2px;
    cursor: pointer;
  }
  .arg-btn:hover {
    background: #2a2a5e;
    border-color: #5a5a8a;
    color: #fff;
  }
  .arg-string {
    color: #e8a838;
  }

  tr:hover {
    background: #1e1e42;
  }

  .page-controls {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 6px 16px;
    border-bottom: 1px solid #2a2a4a;
    background: #16213e;
  }

  .page-btn {
    padding: 3px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #12122a;
    color: #c0c0e0;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .page-btn:hover:not(:disabled) {
    background: #1e1e42;
    border-color: #6c9bd2;
  }

  .page-btn:disabled {
    opacity: 0.3;
    cursor: default;
  }

  .page-info {
    font-size: 0.8rem;
    color: #888;
    min-width: 100px;
    text-align: center;
  }

</style>
