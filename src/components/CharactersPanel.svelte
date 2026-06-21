<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { store } from "../lib/stores.svelte";
  import { confirmDialog } from "../lib/confirm";
  import { scanBinForCharacters } from "../lib/ipc";
  import type { CharacterEntry } from "../lib/types";
  import ResizeHandle from "./ResizeHandle.svelte";
  import { persistAllSettingsDebounced } from "../lib/settings";
  import CharacterNamePickerDialog from "./CharacterNamePickerDialog.svelte";
  import BulkCharacterResolverDialog from "./BulkCharacterResolverDialog.svelte";

  let pickerOpenFor = $state<number | null>(null); // index into characters[]
  let bulkResolverOpen = $state(false);

  let resolvedCount = $derived(
    store.activeTab!.characters.filter((c) => c.name.length > 0).length
  );

  function handleNameInput(index: number, e: Event) {
    const target = e.target as HTMLInputElement;
    store.activeTab!.characters[index].name = target.value;
    store.activeTab!.isDirty = true;
  }

  /** Apply scanned character names to a single tab's character list */
  function applyCharactersToTab(tab: typeof store.activeTab, found: CharacterEntry[]): number {
    if (!tab) return 0;
    let newCount = 0;
    for (const entry of found) {
      const existing = tab.characters.find((c) => c.pointer === entry.pointer);
      if (existing && existing.name.length === 0 && entry.name.length > 0) {
        existing.name = entry.name;
        newCount++;
      }
    }
    if (newCount > 0) {
      tab.characters = [...tab.characters].sort((a, b) => a.pointer - b.pointer);
      tab.isDirty = true;
    }
    return newCount;
  }

  async function handleScanBin() {
    const selected = await open({
      multiple: true,
      title: "Select BIN files to scan for character names",
      filters: [{ name: "BIN Files", extensions: ["BIN", "bin"] }],
    });

    if (!selected) return;

    const paths = Array.isArray(selected) ? selected : [selected];

    // Ask whether to apply globally when multiple tabs are open
    let applyGlobally = false;
    if (store.tabs.length > 1) {
      applyGlobally = await confirmDialog({
        title: "Apply to all tabs?",
        message: "Apply scanned character names to all open tabs, or only the current tab?",
        confirmLabel: "All Tabs",
        cancelLabel: "Current Only",
      });
    }

    // Scan all selected BIN files and collect results
    const allFound: CharacterEntry[] = [];
    for (const path of paths) {
      try {
        const filename = path.replace(/\\/g, "/").split("/").pop();
        store.statusMessage = `Scanning ${filename}...`;
        const found = await scanBinForCharacters(path);
        allFound.push(...found);
      } catch (err) {
        store.statusMessage = `Error scanning: ${err}`;
      }
    }

    let totalNew = 0;
    if (applyGlobally) {
      for (const tab of store.tabs) {
        totalNew += applyCharactersToTab(tab, allFound);
      }
    } else {
      totalNew = applyCharactersToTab(store.activeTab, allFound);
    }

    const scope = applyGlobally ? ` across ${store.tabs.length} tabs` : "";
    store.statusMessage = `Scan complete — resolved ${totalNew} new character${totalNew !== 1 ? "s" : ""}${scope}`;
  }

  let isSys5 = $derived((store.activeTab?.sysVersion ?? 4) >= 5);
  let isSys3 = $derived((store.activeTab?.sysVersion ?? 4) <= 3);

  function formatPointer(pointer: number): string {
    // SYS3/SYS5 use sequential numeric IDs; SYS4 uses hex register addresses
    if (isSys5 || isSys3) return "#" + pointer;
    return "0x" + pointer.toString(16).toUpperCase();
  }
</script>

<div class="characters-panel" style="width: {store.panelWidths.characters}px">
  <ResizeHandle
    width={store.panelWidths.characters}
    minWidth={240}
    maxWidth={800}
    onResize={(w) => (store.panelWidths.characters = w)}
    onResizeEnd={() => persistAllSettingsDebounced()}
  />
  <div class="panel-header">
    <h3>Characters / Speakers</h3>
    <span class="resolve-count">
      {resolvedCount}/{store.activeTab!.characters.length} resolved
    </span>
  </div>

  <div class="panel-actions">
    <button class="scan-btn" onclick={handleScanBin}>
      Scan BIN for Names
    </button>
    {#if store.tabs.length > 1}
      <button
        class="scan-btn"
        onclick={() => (bulkResolverOpen = true)}
        title="Resolve unknown character names across ALL open tabs in one dialog"
      >
        Bulk Resolve (all tabs)
      </button>
    {/if}
    {#if isSys3}
      <p class="sys5-hint">
        SYS3 speakers are detected from inline text. Type translations manually or scan a CNINIT2.BIN file.
      </p>
    {:else if isSys5 && resolvedCount < store.activeTab!.characters.length}
      <p class="sys5-hint">
        Some SYS5 speaker names were not resolved. Type names manually or scan a CVINIT.BIN file.
      </p>
    {/if}
  </div>

  <div class="characters-table-container">
    <table class="characters-table">
      <thead>
        <tr>
          <th class="th-pointer">Pointer</th>
          <th class="th-name">Name</th>
        </tr>
      </thead>
      <tbody>
        {#each store.activeTab!.characters as char, index (char.pointer)}
          <tr class={char.name.length > 0 ? "resolved" : "unresolved"}>
            <td class="cell-pointer">{formatPointer(char.pointer)}</td>
            <td class="cell-name">
              <div class="name-cell">
                <input
                  type="text"
                  class="name-input"
                  value={char.name}
                  oninput={(e) => handleNameInput(index, e)}
                  placeholder="Unknown"
                />
                {#if store.activeTab?.binPath && !char.name}
                  <button
                    type="button"
                    class="pick-btn"
                    title="Pick from init BIN names"
                    onclick={() => (pickerOpenFor = index)}
                  >🔍</button>
                {/if}
              </div>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

{#if bulkResolverOpen}
  <BulkCharacterResolverDialog onClose={() => (bulkResolverOpen = false)} />
{/if}

{#if pickerOpenFor !== null && store.activeTab?.binPath}
  {@const idx = pickerOpenFor}
  {@const char = store.activeTab.characters[idx]}
  <CharacterNamePickerDialog
    binPath={store.activeTab.binPath}
    pointer={char.pointer}
    pointerLabel={formatPointer(char.pointer)}
    onPick={(name) => {
      store.activeTab!.characters[idx].name = name;
      store.activeTab!.isDirty = true;
    }}
    onClose={() => (pickerOpenFor = null)}
  />
{/if}

<style>
  .characters-panel {
    display: flex;
    flex-direction: column;
    border-left: 1px solid #2a2a4a;
    background: #1a1a2e;
    overflow: hidden;
    position: relative;
    flex-shrink: 0;
  }

  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border-bottom: 1px solid #2a2a4a;
    background: #16213e;
  }

  .panel-header h3 {
    margin: 0;
    font-size: 0.9rem;
    color: #8ab4f8;
    font-weight: 600;
  }

  .resolve-count {
    font-size: 0.75rem;
    color: #888;
  }

  .panel-actions {
    padding: 8px 12px;
    border-bottom: 1px solid #2a2a4a;
  }

  .scan-btn {
    width: 100%;
    padding: 6px 12px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.8rem;
    transition: all 0.15s;
  }

  .scan-btn:hover {
    background: #2a2a5e;
    border-color: #5a5a8a;
  }

  .sys5-hint {
    margin: 6px 0 0;
    font-size: 0.72rem;
    color: #888;
    line-height: 1.4;
  }

  .characters-table-container {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
  }

  .characters-table {
    width: 100%;
    border-collapse: collapse;
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

  .th-pointer {
    width: 100px;
  }

  tr.resolved {
    background: rgba(40, 120, 60, 0.15);
  }

  tr.unresolved {
    background: rgba(140, 40, 40, 0.15);
  }

  tr.resolved:hover {
    background: rgba(40, 120, 60, 0.25);
  }

  tr.unresolved:hover {
    background: rgba(140, 40, 40, 0.25);
  }

  td {
    padding: 4px 10px;
    border-bottom: 1px solid #222240;
    font-size: 0.8rem;
  }

  .cell-pointer {
    font-family: "Consolas", "Courier New", monospace;
    color: #8ab4f8;
    font-size: 0.8rem;
    user-select: all;
  }

  .name-cell {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .pick-btn {
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 3px;
    background: transparent;
    color: #8a8aaa;
    cursor: pointer;
    font-size: 0.8rem;
    line-height: 1;
    opacity: 0.5;
  }
  tr:hover .pick-btn { opacity: 1; }
  .pick-btn:hover {
    background: #2a2a5e;
    border-color: #5a5a8a;
    color: #fff;
  }

  .name-input {
    width: 100%;
    padding: 2px 4px;
    border: 1px solid transparent;
    border-radius: 3px;
    background: transparent;
    color: #c0c0e0;
    font-family: inherit;
    font-size: 0.8rem;
    outline: none;
  }

  .name-input::placeholder {
    color: #555;
    font-style: italic;
  }

  .name-input:hover {
    border-color: #3a3a5a;
  }

  .name-input:focus {
    border-color: #6c9bd2;
    background: rgba(108, 155, 210, 0.06);
  }
</style>
