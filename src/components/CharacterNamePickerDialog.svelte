<script lang="ts">
  /**
   * Manual character-name picker. Surfaces every set-string-defined name
   * across the project's init BINs (CVINIT / CIINIT / CNINIT / CHINIT) so
   * the user can resolve scene-local lookup-array IDs that static analysis
   * can't reach (SYS4 builds runtime speaker tables per scene).
   *
   * The user filters the list, picks a name, optionally edits it (in case
   * they want a translated form), and confirms. The chosen string is fed
   * back via `onPick` — the parent stores it on `tab.characters[i].name`.
   */
  import { fileExists } from "../lib/ipc";
  import { listInitBinNames, type InitNameCandidate } from "../lib/ipc";

  interface Props {
    /** Source BIN path — used to derive the init-BIN directory. */
    binPath: string;
    /** Pointer being resolved, just for display in the dialog header. */
    pointer: number;
    /** Pretty-printed pointer label (matches what the panel shows). */
    pointerLabel: string;
    onPick: (name: string) => void;
    onClose: () => void;
  }

  let { binPath, pointer: _pointer, pointerLabel, onPick, onClose }: Props = $props();

  let candidates = $state<InitNameCandidate[]>([]);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let filter = $state("");
  let selectedKey = $state<string | null>(null);
  let editedName = $state("");

  const BASE_INIT_FILENAMES = ["CVINIT.BIN", "CIINIT.BIN", "CNINIT.BIN", "CNINIT2.BIN", "CHINIT.BIN"];

  /** Eushully append/DLC convention: `$N$<NAME>.BIN`. When the scene BIN
   *  has an `$N$` prefix, include the matching `$N$<INIT>.BIN` files in
   *  the candidate list — those are where append-introduced characters
   *  live and the user would otherwise have to hand-type names. */
  function candidateFilenames(scenePath: string): string[] {
    const fname = scenePath.replace(/\\/g, "/").split("/").pop() ?? "";
    const m = fname.match(/^(\$\d+\$)/);
    if (!m) return BASE_INIT_FILENAMES;
    const prefix = m[1];
    return [...BASE_INIT_FILENAMES.map((n) => prefix + n), ...BASE_INIT_FILENAMES];
  }

  async function load() {
    try {
      const normalized = binPath.replace(/\\/g, "/");
      const dir = normalized.substring(0, normalized.lastIndexOf("/"));
      const paths: string[] = [];
      for (const fn of candidateFilenames(binPath)) {
        const p = `${dir}/${fn}`;
        if (await fileExists(p)) paths.push(p);
      }
      if (paths.length === 0) {
        loadError = "No init BINs (CVINIT/CIINIT/CNINIT/CHINIT) found next to the scene BIN.";
        loading = false;
        return;
      }
      candidates = await listInitBinNames(paths);
    } catch (e) {
      loadError = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => { void load(); });

  /** Filtered + ranked candidates. Matches against name or hex address. */
  let filtered = $derived.by(() => {
    if (!filter.trim()) return candidates;
    const q = filter.trim().toLowerCase();
    return candidates.filter((c) =>
      c.name.toLowerCase().includes(q) ||
      ("0x" + c.address.toString(16)).includes(q) ||
      c.source_file.toLowerCase().includes(q),
    );
  });

  function rowKey(c: InitNameCandidate): string {
    return `${c.source_file}:${c.address}:${c.name}`;
  }

  function selectRow(c: InitNameCandidate) {
    selectedKey = rowKey(c);
    editedName = c.name;
  }

  function confirm() {
    const name = editedName.trim();
    if (!name) return;
    onPick(name);
    onClose();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
    else if (e.key === "Enter" && selectedKey && editedName.trim()) confirm();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div
  class="backdrop"
  role="presentation"
  onclick={onClose}
></div>

<div class="dialog" role="dialog" aria-label="Pick character name">
  <header class="dialog-header">
    <h3>Pick name for {pointerLabel}</h3>
    <button class="close-btn" onclick={onClose} title="Close (Esc)">×</button>
  </header>

  <div class="filter-row">
    <input
      type="text"
      class="filter-input"
      placeholder="Filter names, addresses (0x...), or source file..."
      bind:value={filter}
      autofocus
    />
    <span class="count">
      {#if loading}loading…{:else}{filtered.length} / {candidates.length}{/if}
    </span>
  </div>

  <div class="list-container">
    {#if loading}
      <p class="empty">Scanning init BINs…</p>
    {:else if loadError}
      <p class="empty error">{loadError}</p>
    {:else if filtered.length === 0}
      <p class="empty">No matches.</p>
    {:else}
      {#each filtered as c (rowKey(c))}
        <button
          type="button"
          class="row"
          class:active={selectedKey === rowKey(c)}
          onclick={() => selectRow(c)}
          ondblclick={() => { selectRow(c); confirm(); }}
        >
          <span class="row-source">{c.source_file}</span>
          <span class="row-addr">0x{c.address.toString(16).toUpperCase()}</span>
          <span class="row-name">{c.name}</span>
        </button>
      {/each}
    {/if}
  </div>

  <div class="pick-row">
    <label class="label">Use as name:</label>
    <input
      type="text"
      class="name-input"
      bind:value={editedName}
      placeholder="Pick a row above (or type a custom name)"
    />
    <button class="cancel-btn" onclick={onClose}>Cancel</button>
    <button class="ok-btn" disabled={!editedName.trim()} onclick={confirm}>OK</button>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 999;
  }
  .dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 1000;
    width: 640px;
    max-width: 92vw;
    max-height: 80vh;
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
    padding: 10px 14px;
    border-bottom: 1px solid #2a2a4a;
  }
  .dialog-header h3 {
    margin: 0;
    font-size: 0.95rem;
    color: #c0c0e0;
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
    gap: 8px;
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
    min-width: 80px;
    text-align: right;
  }

  .list-container {
    flex: 1;
    overflow-y: auto;
    min-height: 200px;
  }
  .row {
    display: grid;
    grid-template-columns: 110px 90px 1fr;
    gap: 10px;
    width: 100%;
    text-align: left;
    padding: 5px 14px;
    background: transparent;
    border: none;
    border-bottom: 1px solid #1a1a30;
    color: #c0c0e0;
    font-size: 0.82rem;
    cursor: pointer;
  }
  .row:hover { background: #1d1d36; }
  .row.active { background: #2a2a5e; }
  .row-source { color: #6c9bd2; font-family: monospace; }
  .row-addr { color: #8a8aaa; font-family: monospace; }
  .row-name { color: #c0c0e0; }

  .pick-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid #2a2a4a;
  }
  .label {
    font-size: 0.8rem;
    color: #8a8aaa;
  }
  .name-input {
    flex: 1;
    padding: 6px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    font-size: 0.85rem;
    outline: none;
  }
  .name-input:focus { border-color: #5a7aff; }
  .ok-btn, .cancel-btn {
    padding: 6px 14px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .ok-btn { background: #2a3a6e; border-color: #5a7aff; color: #fff; }
  .ok-btn:hover:not(:disabled) { background: #3a4a7e; }
  .ok-btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .cancel-btn:hover { background: #2a2a5e; }

  .empty {
    padding: 24px 14px;
    text-align: center;
    color: #555577;
    font-size: 0.85rem;
  }
  .empty.error { color: #d88080; }

</style>
