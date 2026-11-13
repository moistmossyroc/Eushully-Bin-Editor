<script lang="ts">
  // Name search, type chips with counts, Mods, Effective only, counts + selection.
  import { assetsStore as assets } from "./assetsStore.svelte";
  import { clearSelection, selectAllShown, updateFilter } from "./assetsActions";
  import { KIND_LABELS, type EntryKind } from "./types";

  const SEARCH_DEBOUNCE_MS = 150;
  const KINDS: EntryKind[] = ["image", "audio", "script", "video", "other"];

  let searchText = $state(assets.filter.search);
  let searchTimer: ReturnType<typeof setTimeout> | undefined;

  function onSearchInput() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => updateFilter({ search: searchText }), SEARCH_DEBOUNCE_MS);
  }

  function toggleKind(kind: EntryKind) {
    updateFilter({ kind: assets.filter.kind === kind ? null : kind });
  }
</script>

{#if assets.game}
  {@const game = assets.game}
  <div class="filter-bar">
    <input
      class="search"
      type="search"
      placeholder="Filter names…"
      bind:value={searchText}
      oninput={onSearchInput}
    />
    <button class="chip" class:active={assets.filter.kind === null} onclick={() => updateFilter({ kind: null })}>
      All {game.entryCount.toLocaleString()}
    </button>
    {#each KINDS as kind (kind)}
      {#if game.kindCounts[kind]}
        <button class="chip" class:active={assets.filter.kind === kind} onclick={() => toggleKind(kind)}>
          {KIND_LABELS[kind]} {game.kindCounts[kind]?.toLocaleString()}
        </button>
      {/if}
    {/each}
    {#if game.modCount > 0}
      <button
        class="chip mods"
        class:active={assets.filter.modsOnly}
        title="Only files from mod archives (.fanpatch, translation patch)"
        onclick={() => updateFilter({ modsOnly: !assets.filter.modsOnly })}
      >
        Mods {game.modCount.toLocaleString()}
      </button>
    {/if}
    {#if game.notEffectiveCount > 0}
      <label class="toggle" title="Hide {game.notEffectiveCount.toLocaleString()} copies the game does not load (shadowed by a newer copy)">
        <input
          type="checkbox"
          checked={assets.filter.effectiveOnly}
          onchange={(event) => updateFilter({ effectiveOnly: event.currentTarget.checked })}
        />
        Effective only
      </label>
    {/if}
    <span class="spacer"></span>
    <span class="counts">
      {assets.listing.length.toLocaleString()} shown{assets.selected.size > 0 ? ` · ${assets.selected.size.toLocaleString()} selected` : ""}
    </span>
    <button class="small" onclick={selectAllShown} title="Ctrl+A in the list">Select shown</button>
    {#if assets.selected.size > 0}
      <button class="small" onclick={clearSelection} title="Esc in the list">Clear</button>
    {/if}
  </div>
{/if}

<style>
  .filter-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-bottom: 1px solid #2a2a4a;
    font-size: 0.82rem;
  }
  .search {
    width: 220px;
    padding: 5px 8px;
    background: #12122a;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    color: #e0e0e0;
  }
  .search:focus {
    outline: none;
    border-color: #6c9bd2;
  }
  .chip {
    padding: 4px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 999px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.8rem;
  }
  .chip:hover {
    background: #2a2a5e;
  }
  .chip.active {
    background: #2a3a6e;
    border-color: #6c9bd2;
    color: #fff;
  }
  .chip.mods {
    color: #f07a90;
    border-color: #6a3040;
  }
  .chip.mods.active {
    background: #4a1f2c;
    border-color: #f07a90;
  }
  .toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    color: #c0c0e0;
    margin-left: 6px;
  }
  .spacer {
    flex: 1;
  }
  .counts {
    color: #8a8aa8;
  }
  .small {
    padding: 3px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.78rem;
  }
  .small:hover {
    background: #2a2a5e;
  }
</style>
