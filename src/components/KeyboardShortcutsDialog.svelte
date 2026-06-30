<script lang="ts">
  /**
   * Cheat-sheet of every global keyboard shortcut. Opened with `?` (or
   * Shift+/) from anywhere in the app — the trigger is bound in App.svelte
   * so this component doesn't have to listen for it.
   *
   * Single source of truth for documented shortcuts — when adding a new
   * binding elsewhere, add it here too. The list is rendered statically so
   * shipping/reorganizing it stays trivial.
   */
  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  interface Shortcut {
    keys: string[];     // each entry rendered as a <kbd>
    description: string;
  }
  interface Section {
    title: string;
    shortcuts: Shortcut[];
  }

  const sections: Section[] = [
    {
      title: "Global",
      shortcuts: [
        { keys: ["F1"], description: "Open the Help Center" },
        { keys: ["?"], description: "Open this shortcuts dialog" },
        { keys: ["Ctrl", "Shift", "F"], description: "Toggle Global Search panel" },
        { keys: ["Esc"], description: "Close active dialog / overlay" },
      ],
    },
    {
      title: "Editor table",
      shortcuts: [
        { keys: ["↑"], description: "Previous dialogue row" },
        { keys: ["↓"], description: "Next dialogue row" },
      ],
    },
    {
      title: "BIN Viewer",
      shortcuts: [
        { keys: ["↑"], description: "Step to previous search match" },
        { keys: ["↓"], description: "Step to next search match" },
        { keys: ["Alt", "↑/↓"], description: "Step matches while typing in the search box" },
        { keys: ["Click row"], description: "Jump to that instruction in context (when filtered)" },
        { keys: ["Click arg"], description: "Filter by that arg value (when unfiltered)" },
      ],
    },
    {
      title: "Character panel",
      shortcuts: [
        { keys: ["🔍 button"], description: "Pick character name from init BINs (only shown for unresolved IDs)" },
        { keys: ["Bulk Resolve"], description: "Resolve unresolved character names across ALL open tabs in one dialog (shown when ≥2 tabs open)" },
      ],
    },
    {
      title: "Editor row",
      shortcuts: [
        { keys: ["💡"], description: "Block-level translation memory: fills every empty row in the current dialog block from a matching block in another tab (only appears when the whole block's content is identical)" },
      ],
    },
    {
      title: "Tab bar",
      shortcuts: [
        { keys: ["Right-click tab"], description: "Restore from autosave backup (last 3 versions kept, rotated every ~5 min)" },
        { keys: ["Scroll wheel"], description: "Cycle between tabs" },
        { keys: ["Drag tab"], description: "Reorder tabs by dragging" },
        { keys: ["Hover tab"], description: "Close button (×) appears — hidden by default to prevent accidental closes" },
        { keys: ["status dot"], description: "Translation progress: red = none, amber = in progress, green = complete. Amber ring around any color = some translated rows exceed the per-line cap (open Stats panel or use Review overflows)." },
        { keys: ["click status dot"], description: "When yellow/red, opens a list of untranslated rows — click any row to jump to it in the editor" },
      ],
    },
    {
      title: "Glossary panel",
      shortcuts: [
        { keys: ["Suggest"], description: "Find katakana names not yet in glossary, one-click add" },
        { keys: ["Category pills"], description: "Filter to a single category (Character/Location/Item/Skill/Title/General)" },
        { keys: ["Save to folder"], description: "Write current entries to glossary.json next to the BIN — auto-loaded on next BIN open from the same folder" },
      ],
    },
  ];

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="backdrop" onclick={onClose}></div>

<div class="dialog" role="dialog" aria-label="Keyboard shortcuts">
  <header class="dialog-header">
    <h3>Keyboard Shortcuts</h3>
    <button class="close-btn" onclick={onClose} title="Close (Esc)">×</button>
  </header>

  <div class="content">
    {#each sections as section}
      <section class="section">
        <h4>{section.title}</h4>
        <table>
          <tbody>
            {#each section.shortcuts as s}
              <tr>
                <td class="keys-cell">
                  {#each s.keys as key, i}
                    <kbd>{key}</kbd>{#if i < s.keys.length - 1}<span class="plus">+</span>{/if}
                  {/each}
                </td>
                <td class="desc-cell">{s.description}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </section>
    {/each}
  </div>
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
    width: 560px;
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
    padding: 0 4px;
  }
  .close-btn:hover { color: #fff; }

  .content {
    padding: 14px 16px;
    overflow-y: auto;
  }
  .section + .section { margin-top: 18px; }
  .section h4 {
    margin: 0 0 8px 0;
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #8aa0ff;
    border-bottom: 1px solid #2a2a4a;
    padding-bottom: 4px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  td {
    padding: 5px 8px;
    font-size: 0.85rem;
    vertical-align: middle;
  }
  .keys-cell {
    width: 200px;
    white-space: nowrap;
  }
  .desc-cell {
    color: #c0c0e0;
  }
  kbd {
    display: inline-block;
    padding: 2px 6px;
    margin: 0 1px;
    font-family: "Consolas", monospace;
    font-size: 0.78rem;
    color: #e0e0e8;
    background: #25254a;
    border: 1px solid #4a4a7a;
    border-bottom-width: 2px;
    border-radius: 3px;
    line-height: 1;
  }
  .plus {
    color: #777;
    padding: 0 2px;
  }
</style>
