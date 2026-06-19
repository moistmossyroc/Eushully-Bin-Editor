<script lang="ts">
  /**
   * Browser for the rotating project autosave backups (.bak1 .. .bak3).
   * Left panel: list of backups newest-first with timestamps + entry counts.
   * Right panel: per-backup diff against the current project, plus a Revert
   * button that swaps the picked backup in over the current JSON.
   *
   * The autosave throttles rotation to once per 5 minutes, so the 3-slot
   * window typically spans 5–15 minutes of edit history — enough to undo a
   * "wait what did I just do" mistake without keeping a long history.
   */
  import {
    listProjectBackups,
    diffProjectBackup,
    restoreProjectBackup,
    type ProjectBackupInfo,
    type BackupDiff,
  } from "../lib/ipc";
  import { store } from "../lib/stores.svelte";

  interface Props {
    /** Path to the live project JSON (the autosave target). */
    projectPath: string;
    binFilename: string;
    onClose: () => void;
    /** Called after a successful revert so the parent can reload the tab. */
    onReverted: () => void;
  }

  let { projectPath, binFilename, onClose, onReverted }: Props = $props();

  let backups = $state<ProjectBackupInfo[]>([]);
  let loadingList = $state(true);
  let listError = $state<string | null>(null);

  let selectedBackup = $state<ProjectBackupInfo | null>(null);
  let diff = $state<BackupDiff | null>(null);
  let loadingDiff = $state(false);
  let diffError = $state<string | null>(null);

  let restoring = $state(false);
  let restoreError = $state<string | null>(null);

  async function loadList() {
    try {
      loadingList = true;
      listError = null;
      backups = await listProjectBackups(projectPath);
      if (backups.length > 0) {
        await selectBackup(backups[0]);
      }
    } catch (e) {
      listError = String(e);
    } finally {
      loadingList = false;
    }
  }

  async function selectBackup(b: ProjectBackupInfo) {
    selectedBackup = b;
    diff = null;
    diffError = null;
    loadingDiff = true;
    try {
      diff = await diffProjectBackup(projectPath, b.path);
    } catch (e) {
      diffError = String(e);
    } finally {
      loadingDiff = false;
    }
  }

  async function handleRestore() {
    if (!selectedBackup) return;
    const ok = confirm(
      `Revert ${binFilename} to backup #${selectedBackup.depth} (${formatTime(selectedBackup.modified_ms)})?\n\n` +
      `This overwrites your current project JSON. ${diff ? `${diff.translated_diff_count + diff.omit_diff_count} entries will change.` : ""}\n\n` +
      `Your current state will be lost (the next autosave will create a fresh .bak1, but the version before that revert won't be in the chain).`,
    );
    if (!ok) return;
    restoring = true;
    restoreError = null;
    try {
      await restoreProjectBackup(projectPath, selectedBackup.path);
      onReverted();
      onClose();
    } catch (e) {
      restoreError = String(e);
    } finally {
      restoring = false;
    }
  }

  function formatTime(ms: number): string {
    if (!ms) return "(unknown)";
    const d = new Date(ms);
    const now = new Date();
    const sameDay = d.toDateString() === now.toDateString();
    const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    return sameDay ? `Today ${time}` : `${d.toLocaleDateString()} ${time}`;
  }

  function ageFromNow(ms: number): string {
    if (!ms) return "";
    const diffMs = Date.now() - ms;
    const mins = Math.floor(diffMs / 60_000);
    if (mins < 1) return "just now";
    if (mins < 60) return `${mins} min ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    return `${Math.floor(hours / 24)}d ago`;
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
  }

  function truncate(s: string, n: number): string {
    return s.length > n ? s.slice(0, n) + "…" : s;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
  }

  $effect(() => { void loadList(); });
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="backdrop" onclick={onClose}></div>

<div class="dialog" role="dialog" aria-label="Backup history">
  <header class="dialog-header">
    <h3>Backup history — {binFilename}</h3>
    <button class="close-btn" onclick={onClose} title="Close (Esc)">×</button>
  </header>

  <div class="body">
    <aside class="backup-list">
      {#if loadingList}
        <p class="empty">Loading…</p>
      {:else if listError}
        <p class="empty error">{listError}</p>
      {:else if backups.length === 0}
        <p class="empty">No backups yet. Autosaves create a backup at most once every 5 minutes.</p>
      {:else}
        {#each backups as b (b.path)}
          <button
            type="button"
            class="backup-row"
            class:active={selectedBackup?.path === b.path}
            onclick={() => selectBackup(b)}
          >
            <div class="row-line1">
              <span class="depth-tag">#{b.depth}</span>
              <span class="age">{ageFromNow(b.modified_ms)}</span>
            </div>
            <div class="row-line2">{formatTime(b.modified_ms)}</div>
            <div class="row-line3">
              {b.entry_count ?? "?"} entries · {formatSize(b.size_bytes)}
            </div>
          </button>
        {/each}
        <p class="hint">
          #1 is most recent. Newest backup may be up to 5 minutes older than your last edit.
        </p>
      {/if}
    </aside>

    <section class="diff-pane">
      {#if !selectedBackup}
        <p class="empty">Pick a backup to see what changed.</p>
      {:else if loadingDiff}
        <p class="empty">Computing diff…</p>
      {:else if diffError}
        <p class="empty error">{diffError}</p>
      {:else if diff}
        <div class="stat-row">
          <div class="stat">
            <div class="stat-value">{diff.translated_diff_count}</div>
            <div class="stat-label">translation diffs</div>
          </div>
          <div class="stat">
            <div class="stat-value">{diff.omit_diff_count}</div>
            <div class="stat-label">omit-flag diffs</div>
          </div>
          <div class="stat">
            <div class="stat-value">{diff.total_current} → {diff.total_backup}</div>
            <div class="stat-label">entry count</div>
          </div>
        </div>

        {#if diff.rows.length === 0}
          <p class="empty">This backup is identical to the current project.</p>
        {:else}
          <table class="diff-table">
            <thead>
              <tr>
                <th class="col-id">#</th>
                <th class="col-orig">Original</th>
                <th class="col-cur">Current</th>
                <th class="col-bak">Backup</th>
              </tr>
            </thead>
            <tbody>
              {#each diff.rows as r}
                <tr>
                  <td class="col-id">{r.instruction_index}</td>
                  <td class="col-orig" title={r.original_text}>{truncate(r.original_text, 80)}</td>
                  <td class="col-cur" title={r.current_translated}>{truncate(r.current_translated, 80) || "—"}</td>
                  <td class="col-bak" title={r.backup_translated}>{truncate(r.backup_translated, 80) || "—"}</td>
                </tr>
              {/each}
            </tbody>
          </table>
          {#if diff.rows.length >= 200}
            <p class="hint">(showing first 200 of {diff.translated_diff_count + diff.omit_diff_count} differing rows)</p>
          {/if}
        {/if}
      {/if}

      {#if restoreError}
        <p class="empty error">{restoreError}</p>
      {/if}
    </section>
  </div>

  <footer>
    {#if selectedBackup && diff && (diff.translated_diff_count + diff.omit_diff_count) > 0}
      <span class="footer-hint">
        Reverting overwrites the current project with backup #{selectedBackup.depth}.
      </span>
    {/if}
    <button class="btn cancel" onclick={onClose}>Close</button>
    <button
      class="btn primary"
      disabled={!selectedBackup || restoring || loadingDiff}
      onclick={handleRestore}
    >
      {restoring ? "Reverting…" : "Revert to selected"}
    </button>
  </footer>
</div>

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
    width: 900px;
    max-width: 95vw;
    height: 80vh;
    max-height: 720px;
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
  }
  .close-btn {
    background: transparent;
    border: none;
    color: #8a8aaa;
    font-size: 1.3rem;
    cursor: pointer;
  }
  .close-btn:hover { color: #fff; }

  .body {
    flex: 1;
    display: grid;
    grid-template-columns: 240px 1fr;
    min-height: 0;
    overflow: hidden;
  }

  .backup-list {
    border-right: 1px solid #2a2a4a;
    background: #1a1a30;
    overflow-y: auto;
    padding: 6px 0;
  }
  .backup-row {
    display: block;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    border-bottom: 1px solid #20203a;
    padding: 8px 12px;
    color: #c0c0e0;
    cursor: pointer;
    transition: background 0.1s;
  }
  .backup-row:hover { background: #20203a; }
  .backup-row.active { background: #2a2a5e; }
  .row-line1 {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin-bottom: 2px;
  }
  .depth-tag {
    background: #2a3a6e;
    color: #fff;
    font-size: 0.7rem;
    padding: 1px 6px;
    border-radius: 8px;
    font-family: monospace;
  }
  .age {
    color: #8aa0ff;
    font-size: 0.75rem;
    font-weight: 500;
  }
  .row-line2 {
    color: #888;
    font-size: 0.72rem;
    font-family: monospace;
  }
  .row-line3 {
    color: #6a6a8a;
    font-size: 0.7rem;
    margin-top: 2px;
  }
  .hint {
    padding: 10px 12px;
    font-size: 0.7rem;
    color: #6a6a8a;
    font-style: italic;
    line-height: 1.4;
  }

  .diff-pane {
    overflow-y: auto;
    padding: 14px 16px;
    background: #14142a;
  }
  .stat-row {
    display: flex;
    gap: 10px;
    margin-bottom: 12px;
  }
  .stat {
    flex: 1;
    background: #1a1a3e;
    border: 1px solid #2a2a4a;
    border-radius: 4px;
    padding: 8px;
    text-align: center;
  }
  .stat-value {
    font-size: 1.2rem;
    font-weight: 600;
    color: #8ab4f8;
    line-height: 1;
    margin-bottom: 3px;
  }
  .stat-label {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #888;
  }

  .diff-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.78rem;
    table-layout: fixed;
  }
  .diff-table th {
    text-align: left;
    color: #888;
    font-size: 0.7rem;
    padding: 5px 6px;
    border-bottom: 1px solid #2a2a4a;
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }
  .diff-table td {
    padding: 5px 6px;
    border-bottom: 1px solid #1a1a30;
    vertical-align: top;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .col-id { width: 60px; color: #8aa0ff; font-family: monospace; font-size: 0.72rem; }
  .col-orig { width: 30%; color: #d8a86f; }
  .col-cur { color: #c0c0e0; }
  .col-bak { color: #6fd8a4; }

  .empty {
    padding: 20px 16px;
    text-align: center;
    color: #6a6a8a;
    font-size: 0.85rem;
    line-height: 1.5;
  }
  .empty.error { color: #f08080; }

  footer {
    padding: 10px 16px;
    border-top: 1px solid #2a2a4a;
    display: flex;
    align-items: center;
    gap: 10px;
    justify-content: flex-end;
  }
  .footer-hint {
    flex: 1;
    color: #888;
    font-size: 0.75rem;
    text-align: left;
  }
  .btn {
    padding: 6px 16px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.85rem;
  }
  .btn.cancel:hover:not(:disabled) { background: #2a2a5e; }
  .btn.primary {
    background: #2a3a6e;
    border-color: #5a7aff;
    color: #fff;
  }
  .btn.primary:hover:not(:disabled) { background: #3a4a7e; }
  .btn:disabled { opacity: 0.45; cursor: not-allowed; }

</style>
