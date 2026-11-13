<script lang="ts">
  // Text box profile for the active game (engine/box_profile.rs): move the body text up,
  // grow its surface, tighten line spacing — same font size, more lines per box. Stored
  // per installed game folder; when enabled the .fanpatch export patches SYSTEM4 and every
  // dialogue scene (age-sys.md §7b) and the game view previews it.
  import { store } from "../lib/stores.svelte";
  import type { BoxProfile } from "../lib/ipc";
  import { applyBoxProfile, visibleLineCount } from "../lib/gameLayout";
  import type { LayoutGeometry } from "../lib/gameLayout";

  interface Props {
    /** Body box of the base game (no .fanpatch applied). */
    base: LayoutGeometry;
    gameDir: string;
  }

  let { base, gameDir }: Props = $props();

  let entry = $derived(store.boxProfiles[gameDir] ?? { profile: { liftPx: 0, growPx: 0, lineSpacing: base.lineSpacing }, enabled: false });
  let nativeLines = $derived(visibleLineCount(base));
  let profileLines = $derived(visibleLineCount(applyBoxProfile(base, entry.profile)));

  function update(change: Partial<BoxProfile> & { enabled?: boolean }) {
    const { enabled, ...profileChange } = change;
    store.boxProfiles = {
      ...store.boxProfiles,
      [gameDir]: { profile: { ...entry.profile, ...profileChange }, enabled: enabled ?? entry.enabled },
    };
    void import("../lib/settings").then((m) => m.persistAllSettingsDebounced());
  }

  function numberFrom(event: Event): number {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    return Number.isFinite(value) ? Math.max(0, Math.round(value)) : 0;
  }
</script>

<div class="box-profile">
  <label class="toggle" title="Patch SYSTEM4 and every dialogue scene's line spacing in the exported .fanpatch">
    <input type="checkbox" checked={entry.enabled} onchange={(e) => update({ enabled: (e.currentTarget as HTMLInputElement).checked })} />
    Text box profile
  </label>
  <label title="Move the text up inside its area (px)">
    up <input type="number" min="0" max="60" value={entry.profile.liftPx} onchange={(e) => update({ liftPx: numberFrom(e) })} />
  </label>
  <label title="Make the text area taller (px)">
    taller <input type="number" min="0" max="80" value={entry.profile.growPx} onchange={(e) => update({ growPx: numberFrom(e) })} />
  </label>
  <label title="Line spacing (px between lines; native {base.lineSpacing})">
    spacing <input type="number" min="0" max="40" value={entry.profile.lineSpacing} onchange={(e) => update({ lineSpacing: numberFrom(e) })} />
  </label>
  <span class="lines" class:gain={profileLines > nativeLines}>{nativeLines} → {profileLines} lines</span>
  {#if entry.enabled && profileLines !== store.maxLinesPerBox}
    <button
      class="use-btn"
      onclick={() => {
        store.maxLinesPerBox = profileLines;
        void import("../lib/settings").then((m) => m.persistAllSettingsDebounced());
      }}
      title="Set the re-flow / pagination limit to the profile's line count"
    >Use {profileLines} lines per box</button>
  {/if}
</div>

<style>
  .box-profile {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    font-size: 11px;
    color: var(--text-secondary, #aaa);
  }
  .box-profile input[type="number"] {
    width: 44px;
    font-size: 11px;
    background: var(--bg-secondary, #222);
    color: inherit;
    border: 1px solid var(--border, #444);
    border-radius: 3px;
    padding: 0 3px;
  }
  .toggle {
    font-weight: 600;
  }
  .lines.gain {
    color: #7fd48a;
  }
  .use-btn {
    font-size: 11px;
    padding: 0 6px;
    margin-left: auto;
  }
</style>
