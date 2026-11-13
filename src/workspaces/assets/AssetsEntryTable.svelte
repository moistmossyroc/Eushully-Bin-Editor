<script lang="ts">
  // Virtualised file list: only visible rows are rendered; row details are
  // paged in from the backend as the user scrolls (games have 15k-36k entries).
  import { assetsStore as assets } from "./assetsStore.svelte";
  import { clickRow, ensureRows, moveCursorTo, selectAllShown, clearSelection, toggleSelected, updateFilter } from "./assetsActions";
  import type { SortColumn } from "./types";
  import { formatOffset, formatSize, nextSort } from "./utils";

  const ROW_HEIGHT = 26;
  const OVERSCAN_ROWS = 40;
  const COLUMNS: { id: SortColumn; label: string }[] = [
    { id: "name", label: "Name" },
    { id: "kind", label: "Type" },
    { id: "archive", label: "Archive" },
    { id: "offset", label: "Offset" },
    { id: "size", label: "Size" },
  ];

  let viewport: HTMLDivElement | undefined = $state();
  let viewportHeight = $state(400);
  let scrollTop = $state(0);

  let firstVisible = $derived(Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN_ROWS));
  let lastVisible = $derived(Math.min(assets.listing.length - 1, Math.ceil((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN_ROWS));
  let visiblePositions = $derived(
    assets.listing.length === 0 ? [] : Array.from({ length: lastVisible - firstVisible + 1 }, (_, i) => firstVisible + i),
  );

  $effect(() => {
    void ensureRows(firstVisible, lastVisible);
  });

  // Reset scroll when the listing changes (new filter / game).
  let lastListing: number[] | null = null;
  $effect(() => {
    if (assets.listing !== lastListing) {
      lastListing = assets.listing;
      if (viewport && assets.cursor === null) viewport.scrollTop = 0;
    }
  });

  function scrollCursorIntoView(position: number) {
    if (!viewport) return;
    const top = position * ROW_HEIGHT;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (top + ROW_HEIGHT > viewport.scrollTop + viewport.clientHeight) viewport.scrollTop = top + ROW_HEIGHT - viewport.clientHeight;
  }

  // Keep the cursor row visible however it moved (keys, --show, filter change).
  $effect(() => {
    const cursor = assets.cursor;
    if (cursor !== null) scrollCursorIntoView(cursor);
  });

  function handleKeydown(event: KeyboardEvent) {
    const pageRows = Math.max(1, Math.floor(viewportHeight / ROW_HEIGHT) - 1);
    const cursor = assets.cursor ?? -1;
    const moves: Record<string, number> = {
      ArrowDown: cursor + 1,
      ArrowUp: Math.max(0, cursor - 1),
      PageDown: cursor + pageRows,
      PageUp: Math.max(0, cursor - pageRows),
      Home: 0,
      End: assets.listing.length - 1,
    };
    if (event.key in moves) {
      event.preventDefault();
      moveCursorTo(Math.min(moves[event.key], assets.listing.length - 1));
    } else if (event.key === " " && assets.cursor !== null) {
      event.preventDefault();
      toggleSelected(assets.listing[assets.cursor]);
    } else if ((event.ctrlKey || event.metaKey) && (event.key === "a" || event.key === "A")) {
      event.preventDefault();
      selectAllShown();
    } else if (event.key === "Escape") {
      clearSelection();
    }
  }

  function sortBy(column: SortColumn) {
    updateFilter(nextSort(assets.filter, column));
  }
</script>

<div class="table">
  <div class="header-row">
    <span class="cell check"></span>
    {#each COLUMNS as column (column.id)}
      <button class="cell header {column.id}" onclick={() => sortBy(column.id)}>
        {column.label}{assets.filter.sort === column.id ? (assets.filter.descending ? " ▼" : " ▲") : ""}
      </button>
    {/each}
  </div>
  <div
    class="viewport"
    bind:this={viewport}
    bind:clientHeight={viewportHeight}
    onscroll={() => (scrollTop = viewport?.scrollTop ?? 0)}
    tabindex="0"
    role="grid"
    aria-rowcount={assets.listing.length}
    onkeydown={handleKeydown}
  >
    <div class="spacer" style:height="{assets.listing.length * ROW_HEIGHT}px">
      {#each visiblePositions as position (assets.listing[position])}
        {@const entryIndex = assets.listing[position]}
        {@const row = assets.rows.get(entryIndex)}
        <div
          class="row"
          class:cursor={assets.cursor === position}
          class:selected={assets.selected.has(entryIndex)}
          class:not-effective={row && !row.effective}
          class:mod={row?.tier === "mod"}
          class:update={row?.tier === "officialUpdate"}
          style:top="{position * ROW_HEIGHT}px"
          role="row"
          tabindex="-1"
          title={row ? `${row.name}${row.note ? `\n${row.note}` : ""}` : ""}
          onclick={(event) => clickRow(position, { ctrl: event.ctrlKey || event.metaKey, shift: event.shiftKey })}
          onkeydown={() => {}}
        >
          <span class="cell check">
            <input
              type="checkbox"
              checked={assets.selected.has(entryIndex)}
              onclick={(event) => { event.stopPropagation(); toggleSelected(entryIndex); }}
            />
          </span>
          {#if row}
            <span class="cell name">{row.name}</span>
            <span class="cell kind">{row.kind}</span>
            <span class="cell archive">{row.archive}</span>
            <span class="cell offset">{formatOffset(row.offset)}</span>
            <span class="cell size">{formatSize(row.size)}</span>
          {:else}
            <span class="cell name loading">…</span>
          {/if}
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .table {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    font-size: 0.82rem;
  }
  .header-row,
  .row {
    display: grid;
    grid-template-columns: 28px minmax(200px, 1fr) 70px 150px 100px 80px;
    align-items: center;
  }
  .header-row {
    background: #16213e;
    border-bottom: 1px solid #2a2a4a;
  }
  .cell {
    padding: 0 8px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .cell.check {
    padding: 0 0 0 6px;
  }
  .header {
    border: none;
    background: none;
    color: #9ab;
    text-align: left;
    font-weight: 600;
    font-size: 0.8rem;
    padding: 6px 8px;
    cursor: pointer;
  }
  .header:hover {
    color: #e0e0e0;
  }
  .viewport {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    outline: none;
  }
  .viewport:focus-visible {
    box-shadow: inset 0 0 0 1px #6c9bd2;
  }
  .spacer {
    position: relative;
  }
  .row {
    position: absolute;
    left: 0;
    right: 0;
    height: 26px;
    cursor: default;
    color: #d8d8ec;
    border-bottom: 1px solid #1f1f38;
  }
  .row:hover {
    background: #22223e;
  }
  .row.selected {
    background: #26365e;
  }
  .row.cursor {
    background: #2e4478;
    box-shadow: inset 2px 0 0 #6c9bd2;
  }
  .row.mod .name {
    color: #f07a90;
  }
  .row.update .name {
    color: #78aae6;
  }
  .row.not-effective .name {
    color: #6a6a84;
    text-decoration: line-through;
  }
  .kind,
  .archive,
  .size {
    color: #9a9ab8;
  }
  .offset {
    font-family: "Cascadia Mono", Consolas, monospace;
    color: #8a8aa8;
    font-size: 0.78rem;
  }
  .loading {
    color: #5a5a7a;
  }
</style>
