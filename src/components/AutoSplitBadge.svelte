<script lang="ts">
  import { store } from "../lib/stores.svelte";

  const qualityConfig = {
    clean: { className: "ready", label: "Auto-split: ready" },
    degraded: {
      className: "degraded",
      label: "Auto-split: ready (degraded)",
    },
    unavailable: {
      className: "lineonly",
      label: "Auto-split: line-only",
    },
  } as const;

  let config = $derived(() => {
    const q = store.activeTab?.bridgeTemplateQuality;
    if (!q) return null;
    return qualityConfig[q] ?? null;
  });
</script>

{#if store.autoSplitEnabled && store.activeTab}
  {@const c = config()}
  {#if c}
    <span
      class="auto-split-badge {c.className}"
      title={c.className === "degraded"
        ? "Bridge template was derived by stripping branches -- cloning may produce minor rendering quirks"
        : c.className === "lineonly"
          ? "No textbox bridge in this BIN -- long lines still wrap within one box (up to max lines), but text that exceeds one box can't spill into a new box"
          : "Branch-free bridge template found -- safe to clone"}
    >
      {c.label}
    </span>
  {/if}
{/if}

<style>
  .auto-split-badge {
    font-size: 0.7rem;
    padding: 2px 8px;
    border-radius: 3px;
    white-space: nowrap;
    user-select: none;
  }

  .auto-split-badge.ready {
    background: #1a3a2e;
    border: 1px solid #2a5a3e;
    color: #6fd886;
  }

  .auto-split-badge.degraded {
    background: #3a3a1a;
    border: 1px solid #5a5a2a;
    color: #d8d86f;
  }

  .auto-split-badge.lineonly {
    background: #3a3a1a;
    border: 1px solid #5a5a2a;
    color: #d8d86f;
  }
</style>
