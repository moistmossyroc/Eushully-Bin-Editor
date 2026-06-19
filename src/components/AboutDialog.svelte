<script lang="ts">
  /**
   * Minimal About modal: app version + project info + quick "open project
   * folder" / "open settings folder" shortcuts so the user can find their
   * Projects/, settings.json, and config dirs without hunting through the
   * filesystem. Triggered from the Toolbar's Settings dropdown.
   */
  import { getVersion } from "@tauri-apps/api/app";
  import { path } from "@tauri-apps/api";
  import { openFolderInExplorer } from "../lib/ipc";
  import { toastError } from "../lib/toast";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  let version = $state("…");
  let configDir = $state<string | null>(null);

  $effect(() => {
    void (async () => {
      try { version = await getVersion(); } catch {/* stay as … */}
      try { configDir = await path.appConfigDir(); } catch {/* skip */}
    })();
  });

  async function revealConfigDir() {
    if (!configDir) return;
    try { await openFolderInExplorer(configDir); }
    catch (e) { toastError(`Open failed: ${e}`); }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="backdrop" onclick={onClose}></div>

<div class="dialog" role="dialog" aria-label="About">
  <header>
    <h3>Eushully BIN Editor</h3>
    <button class="close-btn" onclick={onClose} title="Close (Esc)">×</button>
  </header>

  <div class="body">
    <p class="tagline">
      Translation editor for the Eushully AGE engine.<br />
      Tauri + Svelte + Rust.
    </p>

    <table class="info">
      <tbody>
        <tr><td>Version</td><td>{version}</td></tr>
        <tr><td>SYS support</td><td>SYS3 · SYS4 · SYS5</td></tr>
        <tr><td>Settings folder</td>
          <td class="path-cell" title={configDir ?? ""}>
            {#if configDir}
              <span class="path">{configDir}</span>
              <button class="reveal-btn" onclick={revealConfigDir}>Reveal</button>
            {:else}—{/if}
          </td>
        </tr>
      </tbody>
    </table>

    <p class="note">
      Settings, font-warning dismissal state, and recent-files lists live
      in the settings folder above. Project edit state and rolling backups
      live next to each loaded BIN in a <code>Projects/</code> subfolder.
    </p>
  </div>

  <footer>
    <button class="btn" onclick={onClose}>Close</button>
  </footer>
</div>

<style>
  .backdrop {
    position: fixed; inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 1000;
  }
  .dialog {
    position: fixed;
    top: 50%; left: 50%;
    transform: translate(-50%, -50%);
    z-index: 1001;
    width: 540px;
    max-width: 92vw;
    background: #15152a;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 16px;
    border-bottom: 1px solid #2a2a4a;
  }
  header h3 {
    margin: 0;
    font-size: 1rem;
    color: #8ab4f8;
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
    padding: 16px;
  }
  .tagline {
    margin: 0 0 14px 0;
    color: #c0c0e0;
    font-size: 0.88rem;
    line-height: 1.5;
  }
  .info {
    width: 100%;
    font-size: 0.82rem;
    border-collapse: collapse;
    margin-bottom: 12px;
  }
  .info td {
    padding: 6px 0;
    border-bottom: 1px solid #2a2a4a;
    vertical-align: top;
  }
  .info td:first-child {
    color: #8aa0ff;
    width: 130px;
    font-weight: 500;
  }
  .info td:last-child { color: #c0c0e0; }
  .path-cell {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .path {
    font-family: monospace;
    font-size: 0.74rem;
    color: #8a8aaa;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }
  .reveal-btn {
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    color: #c0c0e0;
    border-radius: 3px;
    padding: 3px 10px;
    font-size: 0.72rem;
    cursor: pointer;
    flex-shrink: 0;
  }
  .reveal-btn:hover { background: #2a2a5e; }

  .note {
    margin: 12px 0 0 0;
    padding: 10px 12px;
    background: #1a1a30;
    border-left: 3px solid #5a7aff;
    border-radius: 3px;
    font-size: 0.75rem;
    color: #8a8aaa;
    line-height: 1.5;
  }
  .note code {
    background: #0e0e22;
    padding: 1px 4px;
    border-radius: 2px;
    color: #e8a838;
    font-size: 0.7rem;
  }

  footer {
    padding: 10px 16px;
    border-top: 1px solid #2a2a4a;
    display: flex;
    justify-content: flex-end;
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
