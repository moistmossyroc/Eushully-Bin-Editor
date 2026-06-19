<script lang="ts">
  /**
   * Cross-tab translation dashboard. Aggregates progress, character
   * resolution, and cap-violation stats across every open tab so the user
   * doesn't have to scan each tab's dot individually to gauge overall
   * project state.
   *
   * Numbers update reactively as the user translates — closing untranslated
   * rows or fixing over-cap entries shrinks the relevant figures live.
   */
  import { store } from "../lib/stores.svelte";
  import { hasTranslatableContent } from "../lib/exportOmit";
  import { findOverflows, findBoxOverflows } from "../lib/overflow";
  import ResizeHandle from "./ResizeHandle.svelte";
  import { persistAllSettingsDebounced } from "../lib/settings";

  const EDITABLE = new Set(["show-text", "display-furigana", "comment"]);

  interface PerTab {
    tabIndex: number;
    name: string;
    translated: number;
    total: number;
    pct: number;
    charactersResolved: number;
    charactersTotal: number;
    overCap: number;
    boxOver: number;
    isDirty: boolean;
  }

  let perTab = $derived.by<PerTab[]>(() => {
    const lineCap = store.translatedLineCharCap;
    return store.tabs.map((tab, ti) => {
      let total = 0, done = 0;
      for (const e of tab.entries) {
        if (e.deleted || e.is_context_row || e.omit_export) continue;
        if (!EDITABLE.has(e.label)) continue;
        if (!hasTranslatableContent(e.original_text)) continue;
        total++;
        if (e.translated_text.trim().length > 0) done++;
      }
      // Use findOverflows to match the "Review overflows" toolbar count
      // exactly. It measures LINE-GROUP totals (show-text + display-furigana
      // siblings between end-text-line opcodes), strips quote brackets and
      // furigana annotations — same calc the in-game engine uses for width.
      // Counting raw `translated_text.length` (the bug) over-reported by
      // ~10–50× because every furigana annotation and bracket inflated it.
      const overCap = lineCap > 0 ? findOverflows(tab.entries, lineCap).length : 0;
      // Box overflows: a text box exceeding maxLinesPerBox or the per-box char
      // cap (often a side effect of auto-split) — won't render fully in-game.
      const boxOver = findBoxOverflows(tab.entries, store.translatedCharCap, store.maxLinesPerBox).length;
      const resolved = tab.characters.filter((c) => c.name.length > 0).length;
      return {
        tabIndex: ti,
        name: tab.sourceBinFilename || `Tab ${ti + 1}`,
        translated: done,
        total,
        pct: total === 0 ? 0 : Math.round((done / total) * 100),
        charactersResolved: resolved,
        charactersTotal: tab.characters.length,
        overCap,
        boxOver,
        isDirty: tab.isDirty,
      };
    });
  });

  let overall = $derived.by(() => {
    let translated = 0, total = 0, resolved = 0, characters = 0, overCap = 0, boxOver = 0;
    for (const t of perTab) {
      translated += t.translated;
      total += t.total;
      resolved += t.charactersResolved;
      characters += t.charactersTotal;
      overCap += t.overCap;
      boxOver += t.boxOver;
    }
    return {
      translated, total,
      pct: total === 0 ? 0 : Math.round((translated * 100) / total),
      resolved, characters,
      overCap,
      boxOver,
      tabsComplete: perTab.filter((t) => t.total > 0 && t.translated === t.total).length,
      tabsInProgress: perTab.filter((t) => t.translated > 0 && t.translated < t.total).length,
      tabsPending: perTab.filter((t) => t.total > 0 && t.translated === 0).length,
    };
  });

  function tabClick(tabIndex: number) {
    store.switchTab(tabIndex);
  }
</script>

{#if store.statsPanelOpen}
  <aside class="stats-panel" style="width: {store.panelWidths.stats}px">
    <ResizeHandle
      width={store.panelWidths.stats}
      minWidth={240}
      maxWidth={600}
      onResize={(w) => (store.panelWidths.stats = w)}
      onResizeEnd={() => persistAllSettingsDebounced()}
    />
    <header class="panel-header">
      <h3>Project Stats</h3>
      <button class="close-btn" onclick={() => (store.statsPanelOpen = false)} title="Close">×</button>
    </header>

    {#if store.tabs.length === 0}
      <p class="empty">No tabs open.</p>
    {:else}
      <section class="overall">
        <h4>Overall</h4>
        <div class="big-stat">
          <span class="big-num">{overall.pct}<span class="pct">%</span></span>
          <span class="big-label">{overall.translated.toLocaleString()} / {overall.total.toLocaleString()} translatable rows</span>
        </div>
        <div class="bar"><div class="bar-fill" style="width: {overall.pct}%"></div></div>

        <div class="mini-grid">
          <div class="mini">
            <div class="mini-val complete">{overall.tabsComplete}</div>
            <div class="mini-label">complete</div>
          </div>
          <div class="mini">
            <div class="mini-val progress">{overall.tabsInProgress}</div>
            <div class="mini-label">in progress</div>
          </div>
          <div class="mini">
            <div class="mini-val pending">{overall.tabsPending}</div>
            <div class="mini-label">untouched</div>
          </div>
        </div>

        <div class="extra-stats">
          <div class="extra-row">
            <span class="extra-label">Characters resolved</span>
            <span class="extra-val">{overall.resolved} / {overall.characters}</span>
          </div>
          {#if store.translatedLineCharCap > 0}
            <div class="extra-row" class:over-warn={overall.overCap > 0}>
              <span class="extra-label">Rows over line cap</span>
              <span class="extra-val">{overall.overCap}</span>
            </div>
          {/if}
          {#if overall.boxOver > 0}
            <div class="extra-row over-warn" title="Text boxes exceeding the per-box line/char limits (won't render fully in-game)">
              <span class="extra-label">Boxes over limit</span>
              <span class="extra-val">{overall.boxOver}</span>
            </div>
          {/if}
        </div>
      </section>

      <section class="per-tab">
        <h4>Per tab</h4>
        {#each perTab as t (t.tabIndex)}
          <button
            class="tab-row"
            class:active={store.activeTabIndex === t.tabIndex}
            onclick={() => tabClick(t.tabIndex)}
            title="Click to switch to {t.name}"
          >
            <div class="tab-row-header">
              <span class="tab-row-name">{t.name}{t.isDirty ? " •" : ""}</span>
              <span class="tab-row-pct">{t.pct}%</span>
            </div>
            <div class="mini-bar"><div class="mini-bar-fill" style="width: {t.pct}%"></div></div>
            <div class="tab-row-meta">
              <span>{t.translated}/{t.total} rows</span>
              {#if t.charactersTotal > 0}
                <span class="dot-sep">·</span>
                <span>{t.charactersResolved}/{t.charactersTotal} chars</span>
              {/if}
              {#if t.overCap > 0}
                <span class="dot-sep">·</span>
                <span class="over-warn">{t.overCap} over cap</span>
              {/if}
              {#if t.boxOver > 0}
                <span class="dot-sep">·</span>
                <span class="over-warn">{t.boxOver} box{t.boxOver === 1 ? "" : "es"}</span>
              {/if}
            </div>
          </button>
        {/each}
      </section>
    {/if}
  </aside>
{/if}

<style>
  .stats-panel {
    position: relative;
    flex-shrink: 0;
    background: #15152a;
    border-left: 1px solid #2a2a4a;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid #2a2a4a;
  }
  .panel-header h3 {
    margin: 0;
    font-size: 0.95rem;
    color: #c0c0e0;
  }
  .close-btn {
    background: transparent;
    border: none;
    color: #8a8aaa;
    font-size: 1.2rem;
    cursor: pointer;
    padding: 0 4px;
  }
  .close-btn:hover { color: #fff; }

  .empty {
    padding: 24px 14px;
    text-align: center;
    color: #6a6a8a;
    font-size: 0.85rem;
  }

  h4 {
    margin: 14px 14px 6px 14px;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.6px;
    color: #8aa0ff;
  }

  .overall { padding-bottom: 6px; border-bottom: 1px solid #2a2a4a; }
  .big-stat {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 6px 14px 4px 14px;
  }
  .big-num {
    font-size: 2.4rem;
    font-weight: 600;
    color: #8ab4f8;
    line-height: 1;
  }
  .big-num .pct {
    font-size: 1.1rem;
    color: #6a7aa0;
    margin-left: 2px;
  }
  .big-label {
    color: #888;
    font-size: 0.75rem;
    margin-top: 2px;
  }
  .bar {
    margin: 8px 14px 10px 14px;
    height: 6px;
    background: #2a2a4a;
    border-radius: 3px;
    overflow: hidden;
  }
  .bar-fill {
    height: 100%;
    background: linear-gradient(90deg, #6c9bd2, #8ab4f8);
    transition: width 0.3s;
  }

  .mini-grid {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 6px;
    padding: 0 14px 8px 14px;
  }
  .mini {
    background: #1a1a3e;
    border: 1px solid #2a2a4a;
    border-radius: 3px;
    text-align: center;
    padding: 6px 4px;
  }
  .mini-val {
    font-size: 1.1rem;
    font-weight: 600;
    line-height: 1;
  }
  .mini-val.complete { color: #4ad080; }
  .mini-val.progress { color: #e0a030; }
  .mini-val.pending { color: #d04444; }
  .mini-label {
    font-size: 0.65rem;
    color: #8a8aaa;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    margin-top: 2px;
  }

  .extra-stats {
    padding: 0 14px 8px 14px;
  }
  .extra-row {
    display: flex;
    justify-content: space-between;
    font-size: 0.78rem;
    color: #c0c0e0;
    padding: 4px 0;
  }
  .extra-label { color: #8a8aaa; }
  .extra-row.over-warn .extra-val { color: #e0a030; }
  .over-warn { color: #e0a030; }

  .per-tab {
    flex: 1;
    overflow-y: auto;
    padding-bottom: 12px;
  }
  .tab-row {
    display: block;
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    border-bottom: 1px solid #1a1a30;
    padding: 7px 14px;
    cursor: pointer;
    transition: background 0.1s;
  }
  .tab-row:hover { background: #1d1d3a; }
  .tab-row.active {
    background: #2a2a5e;
    border-left: 3px solid #6c9bd2;
    padding-left: 11px;
  }
  .tab-row-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: 3px;
  }
  .tab-row-name {
    color: #c0c0e0;
    font-size: 0.82rem;
    font-family: monospace;
  }
  .tab-row-pct {
    color: #8aa0ff;
    font-size: 0.78rem;
    font-weight: 600;
  }
  .mini-bar {
    height: 3px;
    background: #2a2a4a;
    border-radius: 2px;
    overflow: hidden;
    margin-bottom: 4px;
  }
  .mini-bar-fill {
    height: 100%;
    background: #6c9bd2;
  }
  .tab-row-meta {
    display: flex;
    gap: 4px;
    font-size: 0.7rem;
    color: #8a8aaa;
  }
  .dot-sep { color: #4a4a6a; }

</style>
