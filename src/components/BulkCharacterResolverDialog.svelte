<script lang="ts">
  /**
   * Single dialog listing every unresolved character (empty name) across all
   * open tabs, with the existing CharacterNamePickerDialog as the per-row
   * resolution UI. Lets the user knock out dozens of name resolutions in one
   * focused session instead of clicking 🔍 on each row separately.
   *
   * Pointer values are scene-local in SYS4, so we do NOT cluster by pointer
   * across tabs — each (tab, character) is its own row.
   */
  import { store } from "../lib/stores.svelte";
  import CharacterNamePickerDialog from "./CharacterNamePickerDialog.svelte";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  interface UnresolvedRow {
    tabIndex: number;
    tabName: string;
    charIndex: number;
    pointer: number;
    pointerLabel: string;
    binPath: string;
    /** Set after the user resolves this row in the picker. */
    resolvedName?: string;
  }

  function formatPointer(pointer: number, sysVersion: number | undefined): string {
    if (sysVersion === 3 || sysVersion === 5) return "#" + pointer;
    return "0x" + pointer.toString(16).toUpperCase();
  }

  let rows = $derived.by<UnresolvedRow[]>(() => {
    const out: UnresolvedRow[] = [];
    for (let ti = 0; ti < store.tabs.length; ti++) {
      const tab = store.tabs[ti];
      if (!tab.binPath) continue;
      const tabName = tab.sourceBinFilename || `Tab ${ti + 1}`;
      for (let ci = 0; ci < tab.characters.length; ci++) {
        const c = tab.characters[ci];
        if (c.name) continue;
        out.push({
          tabIndex: ti,
          tabName,
          charIndex: ci,
          pointer: c.pointer,
          pointerLabel: formatPointer(c.pointer, tab.sysVersion),
          binPath: tab.binPath,
        });
      }
    }
    return out;
  });

  let filter = $state("");
  let filtered = $derived.by(() => {
    if (!filter.trim()) return rows;
    const q = filter.trim().toLowerCase();
    return rows.filter((r) =>
      r.tabName.toLowerCase().includes(q) ||
      r.pointerLabel.toLowerCase().includes(q),
    );
  });

  /** Index into `rows` that's currently being resolved (picker open), if any. */
  let pickerRowIdx = $state<number | null>(null);

  function pickFor(rowIdx: number) {
    pickerRowIdx = rowIdx;
  }

  function applyPick(name: string) {
    if (pickerRowIdx === null) return;
    const row = rows[pickerRowIdx];
    const tab = store.tabs[row.tabIndex];
    if (tab && tab.characters[row.charIndex]) {
      tab.characters[row.charIndex].name = name;
      tab.isDirty = true;
    }
  }

  function closePicker() {
    pickerRowIdx = null;
  }

  /** Number of rows resolved during this session (UI feedback only). */
  let sessionResolved = $derived(
    rows.filter((r) => {
      const tab = store.tabs[r.tabIndex];
      return !!tab?.characters[r.charIndex]?.name;
    }).length,
  );

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && pickerRowIdx === null) onClose();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="backdrop" onclick={onClose}></div>

<div class="dialog" role="dialog" aria-label="Bulk resolve characters">
  <header class="dialog-header">
    <h3>
      Unresolved Characters
      <span class="counter">{rows.length - sessionResolved} remaining</span>
    </h3>
    <button class="close-btn" onclick={onClose} title="Close (Esc)">×</button>
  </header>

  <div class="filter-row">
    <input
      type="text"
      class="filter-input"
      placeholder="Filter by tab name or pointer..."
      bind:value={filter}
      autofocus
    />
    <span class="count">{filtered.length} / {rows.length}</span>
  </div>

  <div class="list">
    {#if rows.length === 0}
      <p class="empty">No unresolved characters across any open tab. 🎉</p>
    {:else if filtered.length === 0}
      <p class="empty">No matches.</p>
    {:else}
      {#each filtered as r, i}
        {@const realIdx = rows.indexOf(r)}
        {@const tab = store.tabs[r.tabIndex]}
        {@const currentName = tab?.characters[r.charIndex]?.name}
        <div class="row" class:resolved={!!currentName}>
          <span class="tab-name" title={r.binPath}>{r.tabName}</span>
          <span class="pointer">{r.pointerLabel}</span>
          {#if currentName}
            <span class="resolved-name" title="Resolved in this session">✓ {currentName}</span>
          {:else}
            <button class="pick-btn" onclick={() => pickFor(realIdx)}>🔍 Pick name</button>
          {/if}
        </div>
      {/each}
    {/if}
  </div>

  <footer>
    <span class="footer-hint">
      {sessionResolved > 0
        ? `Resolved ${sessionResolved} character${sessionResolved === 1 ? "" : "s"} this session — autosave will persist on next row change.`
        : "Pick a name to resolve each row. Names persist to the project JSON on autosave."}
    </span>
    <button class="btn" onclick={onClose}>Done</button>
  </footer>
</div>

{#if pickerRowIdx !== null}
  {@const row = rows[pickerRowIdx]}
  <CharacterNamePickerDialog
    binPath={row.binPath}
    pointer={row.pointer}
    pointerLabel={`${row.tabName} → ${row.pointerLabel}`}
    onPick={applyPick}
    onClose={closePicker}
  />
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 1000;
  }
  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 1001;
    width: 620px;
    max-width: 92vw;
    height: 75vh;
    max-height: 700px;
    background: #15152a;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
  }
  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 16px;
    border-bottom: 1px solid #2a2a4a;
  }
  .dialog-header h3 {
    margin: 0;
    font-size: 1rem;
    color: #c0c0e0;
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .counter {
    color: #f4c87a;
    font-size: 0.78rem;
    font-weight: 500;
  }
  .close-btn {
    background: transparent;
    border: none;
    color: #8a8aaa;
    font-size: 1.3rem;
    cursor: pointer;
  }
  .close-btn:hover { color: #fff; }

  .filter-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-bottom: 1px solid #2a2a4a;
  }
  .filter-input {
    flex: 1;
    padding: 6px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    font-size: 0.85rem;
    outline: none;
  }
  .filter-input:focus { border-color: #5a7aff; }
  .count {
    font-size: 0.75rem;
    color: #777799;
    min-width: 70px;
    text-align: right;
  }

  .list {
    flex: 1;
    overflow-y: auto;
    padding: 6px 0;
  }
  .row {
    display: grid;
    grid-template-columns: 1fr 100px 1fr;
    gap: 12px;
    align-items: center;
    padding: 6px 14px;
    border-bottom: 1px solid #1a1a30;
  }
  .row.resolved { background: rgba(74, 208, 128, 0.06); }
  .tab-name {
    color: #6c9bd2;
    font-family: monospace;
    font-size: 0.82rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pointer {
    color: #8a8aaa;
    font-family: monospace;
    font-size: 0.78rem;
  }
  .pick-btn {
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    color: #c0c0e0;
    font-size: 0.78rem;
    padding: 4px 12px;
    cursor: pointer;
    justify-self: end;
  }
  .pick-btn:hover { background: #2a2a5e; border-color: #5a7aff; }
  .resolved-name {
    color: #6fd886;
    font-size: 0.82rem;
    justify-self: end;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .empty {
    padding: 24px 14px;
    text-align: center;
    color: #6a6a8a;
    font-size: 0.85rem;
  }

  footer {
    padding: 10px 16px;
    border-top: 1px solid #2a2a4a;
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .footer-hint {
    flex: 1;
    color: #888;
    font-size: 0.75rem;
  }
  .btn {
    padding: 6px 16px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #2a3a6e;
    color: #fff;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .btn:hover { background: #3a4a7e; }

</style>
