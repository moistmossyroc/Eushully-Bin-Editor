<script lang="ts">
  import type { TranslationFolderStatus } from "../lib/ipc";
  import { openFolderInExplorer, relaunchTranslationFolder } from "../lib/ipc";

  interface Props {
    parentDir: string;
    folders: TranslationFolderStatus[];
    onClose: () => void;
    /** Optional callback to refresh the folder list after a relaunch finishes. */
    onRefresh?: () => void;
    /** Import a single folder's translation.json. Resolves with applied count.
     *  Throws on no-match/cancel/error (message surfaced inline). */
    onImport?: (folder: TranslationFolderStatus) => Promise<number>;
  }

  let { parentDir, folders, onClose, onRefresh, onImport }: Props = $props();

  let actionError = $state<string | null>(null);
  /** Folder names that have a pending relaunch in flight — used for a
   *  per-row spinner so the user doesn't double-click. */
  let busyFolders = $state<Set<string>>(new Set());

  async function handleOpenFolder(folderName: string) {
    actionError = null;
    try {
      await openFolderInExplorer(`${parentDir}/${folderName}`);
    } catch (e) {
      actionError = `Open folder failed: ${e}`;
    }
  }

  /** Folders with an import in flight — per-row spinner. */
  let importingFolders = $state<Set<string>>(new Set());

  async function handleImport(f: TranslationFolderStatus) {
    if (!onImport) return;
    actionError = null;
    importingFolders.add(f.folder);
    importingFolders = new Set(importingFolders);
    try {
      const applied = await onImport(f);
      // Reflect the new progress immediately without making the user hit Refresh.
      onRefresh?.();
      void applied;
    } catch (e) {
      // "canceled" is the user backing out of the mode prompt — not an error.
      if (String(e).replace(/^Error:\s*/, "") !== "canceled") {
        actionError = `Import failed: ${String(e).replace(/^Error:\s*/, "")}`;
      }
    } finally {
      importingFolders.delete(f.folder);
      importingFolders = new Set(importingFolders);
    }
  }

  async function handleRelaunch(folderName: string) {
    actionError = null;
    busyFolders.add(folderName);
    busyFolders = new Set(busyFolders);
    try {
      await relaunchTranslationFolder(`${parentDir}/${folderName}`);
      // The launched process runs in a separate console. We can't await its
      // completion here — just notify the parent that the user may want to
      // refresh after they observe it finishing.
    } catch (e) {
      actionError = `Relaunch failed: ${e}`;
    } finally {
      // Drop the busy flag after a short delay so the spinner registers but
      // doesn't stick around forever (the actual translate run takes minutes).
      setTimeout(() => {
        busyFolders.delete(folderName);
        busyFolders = new Set(busyFolders);
      }, 1500);
    }
  }

  // Must be $derived, not plain const: the dialog instance persists across a
  // Refresh (the `folders` prop is swapped in place). A const would compute once
  // at init and the header would go stale while the per-row table re-rendered.
  const totalEntries = $derived(folders.reduce((s, f) => s + f.total, 0));
  const totalTranslated = $derived(folders.reduce((s, f) => s + f.translated, 0));
  const overallPct = $derived(totalEntries === 0 ? 0 : Math.floor((totalTranslated * 100) / totalEntries));

  function statusLabel(f: TranslationFolderStatus): string {
    if (f.error) return "ERROR";
    if (f.total === 0) return "EMPTY";
    if (f.translated === f.total) return "COMPLETE";
    if (f.translated === 0) return "NOT STARTED";
    return "IN PROGRESS";
  }

  function statusClass(f: TranslationFolderStatus): string {
    if (f.error) return "status-error";
    if (f.total === 0) return "status-empty";
    if (f.translated === f.total) return "status-complete";
    if (f.translated === 0) return "status-pending";
    return "status-progress";
  }

  function pct(f: TranslationFolderStatus): number {
    return f.total === 0 ? 0 : Math.floor((f.translated * 100) / f.total);
  }
</script>

<div class="dialog-backdrop" onclick={onClose} onkeydown={(e) => e.key === "Escape" && onClose()} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <header>
      <div>
        <h2>Translation Project Status</h2>
        <p class="subtitle" title={parentDir}>{parentDir}</p>
      </div>
      <button class="close-btn" onclick={onClose}>×</button>
    </header>

    <div class="overall">
      <div class="overall-row">
        <span class="overall-label">Overall:</span>
        <span class="overall-value"><strong>{totalTranslated}</strong> / {totalEntries} entries ({overallPct}%)</span>
        <span class="overall-folders">across {folders.length} folder{folders.length === 1 ? "" : "s"}</span>
      </div>
      <div class="overall-bar">
        <div class="overall-bar-fill" style="width: {overallPct}%"></div>
      </div>
    </div>

    {#if folders.length === 0}
      <div class="empty">No translation subfolders found.</div>
    {:else}
      <div class="table-scroll">
        <table>
          <thead>
            <tr>
              <th class="col-folder">Folder</th>
              <th class="col-bin">Source BIN</th>
              <th class="col-progress">Progress</th>
              <th class="col-pct">%</th>
              <th class="col-status">Status</th>
              <th class="col-actions">Actions</th>
            </tr>
          </thead>
          <tbody>
            {#each folders as f}
              {@const isComplete = !f.error && f.total > 0 && f.translated === f.total}
              {@const isBusy = busyFolders.has(f.folder)}
              {@const isImporting = importingFolders.has(f.folder)}
              <tr>
                <td class="col-folder" title={f.folder}>{f.folder}</td>
                <td class="col-bin" title={f.error ?? f.source_bin}>{f.error ? "(error)" : f.source_bin || "—"}</td>
                <td class="col-progress">
                  {#if f.error}
                    —
                  {:else}
                    {f.translated} / {f.total}
                  {/if}
                </td>
                <td class="col-pct">
                  {#if !f.error}
                    <div class="pct-bar">
                      <div class="pct-bar-fill" style="width: {pct(f)}%"></div>
                      <span class="pct-text">{pct(f)}%</span>
                    </div>
                  {/if}
                </td>
                <td class="col-status">
                  <span class="status-badge {statusClass(f)}">{statusLabel(f)}</span>
                </td>
                <td class="col-actions">
                  <button
                    class="row-action"
                    title="Open folder in file explorer"
                    onclick={() => handleOpenFolder(f.folder)}
                  >📁</button>
                  {#if onImport && !f.error && f.total > 0}
                    <button
                      class="row-action"
                      title={isImporting ? "Importing…" : "Import this folder's translation.json into the matching open tab"}
                      disabled={isImporting}
                      onclick={() => handleImport(f)}
                    >{isImporting ? "…" : "📥"}</button>
                  {/if}
                  {#if !isComplete && f.total > 0}
                    <button
                      class="row-action"
                      title={isBusy ? "Launching…" : "Re-run translation in this folder (opens a new terminal window)"}
                      disabled={isBusy}
                      onclick={() => handleRelaunch(f.folder)}
                    >{isBusy ? "…" : "▶"}</button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}

    {#if actionError}
      <div class="action-error">{actionError}</div>
    {/if}

    <footer>
      {#if onRefresh}
        <button class="btn refresh" onclick={onRefresh} title="Re-scan folders for updated progress">Refresh</button>
      {/if}
      <button class="btn cancel" onclick={onClose}>Close</button>
    </footer>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed; inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex; align-items: center; justify-content: center;
    z-index: 1000;
  }
  .dialog {
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    width: 920px; max-width: 95vw; max-height: 85vh;
    display: flex; flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  header {
    padding: 14px 20px;
    border-bottom: 1px solid #2a2a4a;
    display: flex; justify-content: space-between; align-items: flex-start;
  }
  h2 {
    margin: 0; color: #8ab4f8; font-size: 1.05rem;
  }
  .subtitle {
    margin: 4px 0 0 0;
    color: #888;
    font-size: 0.75rem;
    font-family: monospace;
    max-width: 800px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close-btn {
    background: none; border: none; color: #a0a0c0;
    font-size: 1.4rem; cursor: pointer; padding: 0 6px; line-height: 1;
  }
  .close-btn:hover { color: #fff; }
  .overall {
    padding: 12px 20px;
    border-bottom: 1px solid #2a2a4a;
    background: #16213e;
  }
  .overall-row {
    display: flex; align-items: baseline; gap: 12px;
    font-size: 0.85rem;
    color: #c0c0e0;
    margin-bottom: 8px;
  }
  .overall-label { color: #8ab4f8; }
  .overall-value strong { color: #8ab4f8; }
  .overall-folders { color: #888; margin-left: auto; font-size: 0.75rem; }
  .overall-bar {
    height: 6px;
    background: #2a2a4a;
    border-radius: 3px;
    overflow: hidden;
  }
  .overall-bar-fill {
    height: 100%;
    background: linear-gradient(90deg, #6c9bd2, #8ab4f8);
    transition: width 0.3s;
  }
  .empty {
    padding: 32px 24px;
    text-align: center;
    color: #a0a0c0;
    font-size: 0.9rem;
  }
  .table-scroll { flex: 1; overflow-y: auto; min-height: 0; }
  table { width: 100%; border-collapse: collapse; font-size: 0.82rem; }
  th {
    position: sticky; top: 0;
    /* Above the per-row .pct-bar (position: relative) cells — without an
     * explicit z-index the positioned progress bars paint over the sticky
     * header as rows scroll under it. */
    z-index: 2;
    background: #16213e;
    color: #8ab4f8;
    padding: 8px 10px;
    text-align: left; font-weight: 600;
    border-bottom: 1px solid #2a2a4a;
  }
  td {
    padding: 6px 10px;
    color: #c0c0e0;
    border-bottom: 1px solid #20204a;
    vertical-align: middle;
  }
  .col-folder { width: 160px; font-family: monospace; color: #6c9bd2; }
  .col-bin { width: 180px; font-family: monospace; }
  .col-progress { width: 110px; font-family: monospace; text-align: right; }
  .col-pct { width: 160px; }
  .col-status { width: 130px; }
  .col-actions { width: 110px; text-align: right; white-space: nowrap; }
  .row-action {
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    color: #c0c0e0;
    font-size: 0.85rem;
    padding: 2px 7px;
    margin-left: 2px;
    cursor: pointer;
  }
  .row-action:hover:not(:disabled) { background: #2a2a5e; border-color: #5a5a8a; }
  .row-action:disabled { opacity: 0.5; cursor: wait; }
  .action-error {
    padding: 8px 16px;
    background: #3a1a1a;
    border-top: 1px solid #5a2a2a;
    color: #f08080;
    font-size: 0.8rem;
  }
  .btn.refresh {
    background: #1a3a2a;
    border-color: #2a5a3a;
    color: #8af0a0;
  }
  .btn.refresh:hover { background: #2a4a3a; }
  .pct-bar {
    position: relative;
    height: 16px;
    background: #2a2a4a;
    border-radius: 3px;
    overflow: hidden;
  }
  .pct-bar-fill {
    height: 100%;
    background: linear-gradient(90deg, #4a7a9a, #6c9bd2);
    transition: width 0.3s;
  }
  .pct-text {
    position: absolute; inset: 0;
    display: flex; align-items: center; justify-content: center;
    font-size: 0.7rem;
    color: #fff;
    text-shadow: 0 0 2px rgba(0,0,0,0.6);
    font-family: monospace;
  }
  .status-badge {
    display: inline-block;
    padding: 2px 8px;
    border-radius: 3px;
    font-size: 0.7rem;
    font-weight: 600;
  }
  .status-complete { background: #1a3a2e; color: #6fd886; border: 1px solid #2a5a3e; }
  .status-progress { background: #2a3a5e; color: #8ab4f8; border: 1px solid #4a6a8a; }
  .status-pending  { background: #3a2a1a; color: #e0a070; border: 1px solid #5a3a2a; }
  .status-empty    { background: #2a2a3a; color: #888;    border: 1px solid #3a3a4a; }
  .status-error    { background: #3a1a1a; color: #e06060; border: 1px solid #5a2a2a; }
  footer {
    padding: 12px 20px;
    border-top: 1px solid #2a2a4a;
    display: flex; justify-content: flex-end;
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
  .btn:hover { background: #2a2a5e; }
</style>
