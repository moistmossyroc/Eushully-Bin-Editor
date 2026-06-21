<script lang="ts">
  /**
   * Reviewable glossary auto-apply — ACROSS ALL TABS. Proposes SAFE, deterministic
   * substitutions (fill an untranslated exact-term row; replace a Japanese term
   * that leaked verbatim into the English), grouped by tab. Deselect anything you
   * don't want, then apply — the whole batch (spanning tabs) is captured for
   * one-click bulk undo.
   */
  import { store } from "../lib/stores.svelte";
  import { findGlossaryAutoApplyAllTabs, type GlossaryAutoChange } from "../lib/glossaryAutoApply";
  import { toastInfo } from "../lib/toast";

  interface Props { onClose: () => void; }
  let { onClose }: Props = $props();

  let hasGlossary = $derived(
    store.globalGlossary.some((g) => g.english.trim()) ||
      store.tabs.some((t) => t.glossary.some((g) => g.english.trim())),
  );

  let changes = $derived.by(() =>
    findGlossaryAutoApplyAllTabs(
      store.tabs.map((t, i) => ({
        id: t.id,
        name: t.sourceBinFilename || "Untitled",
        index: i,
        entries: t.entries,
        glossary: t.glossary,
      })),
      store.globalGlossary,
    ),
  );

  interface TabGroup { tabIndex: number; tabId: string; tabName: string; items: GlossaryAutoChange[]; }
  let groups = $derived.by<TabGroup[]>(() => {
    const byTab = new Map<number, TabGroup>();
    for (const c of changes) {
      const ti = c.tabIndex ?? 0;
      let tg = byTab.get(ti);
      if (!tg) { tg = { tabIndex: ti, tabId: c.tabId ?? "", tabName: c.tabName ?? "Untitled", items: [] }; byTab.set(ti, tg); }
      tg.items.push(c);
    }
    return [...byTab.values()].sort((a, b) => a.tabIndex - b.tabIndex);
  });

  const keyOf = (c: GlossaryAutoChange) => `${c.tabIndex}|${c.rowId}`;

  /** keys the user has unchecked (default = everything selected). */
  let excluded = $state(new Set<string>());
  let selectedCount = $derived(changes.filter((c) => !excluded.has(keyOf(c))).length);

  function toggle(c: GlossaryAutoChange) {
    const next = new Set(excluded);
    const k = keyOf(c);
    if (next.has(k)) next.delete(k); else next.add(k);
    excluded = next;
  }
  function selectAll() { excluded = new Set(); }
  function selectNone() { excluded = new Set(changes.map(keyOf)); }

  function truncate(s: string, n: number): string {
    return s.length <= n ? s : s.slice(0, n) + "…";
  }

  function applySelected() {
    const toApply = changes.filter((c) => !excluded.has(keyOf(c)));
    if (toApply.length === 0) return;

    const affectedIdx = [...new Set(toApply.map((c) => c.tabIndex ?? 0))];
    const affectedTabs = affectedIdx.map((i) => store.tabs[i]).filter(Boolean);
    // One snapshot covering every touched tab → single-click undo of the batch.
    store.captureBulkUndo("Glossary auto-apply", affectedTabs);

    for (const ti of affectedIdx) {
      const tab = store.tabs[ti];
      if (!tab) continue;
      const byIndex = new Map(
        toApply.filter((c) => (c.tabIndex ?? 0) === ti).map((c) => [c.rowIndex, c.after]),
      );
      tab.entries = tab.entries.map((e, i) =>
        byIndex.has(i) ? { ...e, translated_text: byIndex.get(i)! } : e,
      );
      tab.isDirty = true;
    }

    toastInfo(`Applied ${toApply.length} glossary change${toApply.length === 1 ? "" : "s"} across ${affectedIdx.length} tab${affectedIdx.length === 1 ? "" : "s"} — undo available`);
    onClose();
  }
</script>

<div class="dialog-backdrop" onclick={onClose} onkeydown={(e) => e.key === "Escape" && onClose()} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <header>
      <h2>Glossary Auto-Apply — all tabs</h2>
      <button class="close-btn" onclick={onClose}>×</button>
    </header>

    {#if !hasGlossary}
      <div class="empty">No glossary entries with English renderings yet — add some first.</div>
    {:else if changes.length === 0}
      <div class="empty">✓ Nothing to auto-apply — no exact-term fills or leaked Japanese terms found in any tab.</div>
    {:else}
      <div class="summary">
        <strong>{changes.length}</strong> safe change{changes.length === 1 ? "" : "s"} across
        <strong>{groups.length}</strong> tab{groups.length === 1 ? "" : "s"}
        (<span class="kind fill">fill</span> = exact-term row, <span class="kind replace">replace</span> = Japanese leaked into the English).
        <span class="select-actions">
          <button class="link" onclick={selectAll}>all</button> /
          <button class="link" onclick={selectNone}>none</button>
        </span>
      </div>
      <div class="scroll">
        {#each groups as g (g.tabId)}
          <div class="tab-section">
            <div class="tab-head">{g.tabName} <span class="tab-count">{g.items.length}</span></div>
            {#each g.items as c (keyOf(c))}
              <div class="row">
                <input type="checkbox" checked={!excluded.has(keyOf(c))} onchange={() => toggle(c)} />
                <div class="body">
                  <div class="term-line">
                    <span class="kind {c.kind}">{c.kind}</span>
                    <span class="jp">{truncate(c.term, 40)}</span>
                    <span class="arrow">→</span>
                    <span class="en">{truncate(c.glossaryEn, 40)}</span>
                  </div>
                  <div class="before">{c.before.trim() === "" ? "(empty)" : truncate(c.before, 100)}</div>
                  <div class="after"><span class="aarrow">→</span> {truncate(c.after, 100)}</div>
                </div>
              </div>
            {/each}
          </div>
        {/each}
      </div>
    {/if}

    <footer>
      <button class="btn" onclick={onClose}>Cancel</button>
      <button class="btn primary" disabled={selectedCount === 0} onclick={applySelected}>
        Apply {selectedCount} selected
      </button>
    </footer>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed; inset: 0; background: rgba(0, 0, 0, 0.6);
    display: flex; align-items: center; justify-content: center; z-index: 1000;
  }
  .dialog {
    background: #1a1a3e; border: 1px solid #3a3a5a; border-radius: 6px;
    width: 920px; max-width: 92vw; max-height: 85vh;
    display: flex; flex-direction: column; box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  header {
    padding: 14px 20px; border-bottom: 1px solid #2a2a4a;
    display: flex; justify-content: space-between; align-items: center;
  }
  h2 { margin: 0; color: #8ab4f8; font-size: 1.05rem; }
  .close-btn { background: none; border: none; color: #a0a0c0; font-size: 1.4rem; cursor: pointer; padding: 0 6px; line-height: 1; }
  .close-btn:hover { color: #fff; }
  .empty { padding: 32px 24px; text-align: center; color: #a0a0c0; font-size: 0.9rem; }
  .summary { padding: 12px 20px; font-size: 0.85rem; color: #c0c0e0; border-bottom: 1px solid #2a2a4a; }
  .summary strong { color: #6ad08a; }
  .select-actions { margin-left: 8px; }
  .link { background: none; border: none; color: #8ab4f8; cursor: pointer; font-size: 0.82rem; padding: 0 2px; text-decoration: underline; }
  .scroll { flex: 1; overflow-y: auto; min-height: 0; padding: 4px 0; }
  .tab-section { border-bottom: 1px solid #20204a; }
  .tab-head {
    position: sticky; top: 0; z-index: 1; background: #16213e; color: #8ab4f8;
    padding: 6px 20px; font-size: 0.82rem; font-weight: 600; border-bottom: 1px solid #2a2a4a;
  }
  .tab-count { color: #6a6a8a; font-weight: 400; }
  .row { display: flex; gap: 10px; padding: 8px 20px; border-bottom: 1px solid #20204a; align-items: flex-start; }
  .row input[type="checkbox"] { margin-top: 3px; }
  .body { flex: 1; min-width: 0; }
  .term-line { font-size: 0.8rem; margin-bottom: 3px; }
  .term-line .jp { color: #e0c0a0; }
  .term-line .arrow { color: #6a6a8a; margin: 0 6px; }
  .term-line .en { color: #8ab4f8; font-weight: 600; }
  .kind { display: inline-block; font-size: 0.68rem; padding: 1px 6px; border-radius: 3px; margin-right: 6px; text-transform: uppercase; letter-spacing: 0.03em; }
  .kind.fill { background: #1e3a2e; color: #6ad08a; border: 1px solid #2e5a44; }
  .kind.replace { background: #3a2e1e; color: #f0a070; border: 1px solid #5a442e; }
  .before { color: #9a7a7a; font-size: 0.78rem; text-decoration: line-through; }
  .after { color: #d0e8d0; font-size: 0.82rem; line-height: 1.4; }
  .after .aarrow { color: #6ad08a; }
  footer { padding: 12px 20px; border-top: 1px solid #2a2a4a; display: flex; justify-content: flex-end; gap: 8px; }
  .btn { padding: 6px 14px; border: 1px solid #3a3a5a; border-radius: 4px; background: #1a1a3e; color: #c0c0e0; cursor: pointer; font-size: 0.85rem; }
  .btn:hover { background: #2a2a5e; }
  .btn.primary { background: #2a4a3e; border-color: #3e6a54; color: #c0f0d0; }
  .btn.primary:hover:not(:disabled) { background: #3a5a4e; }
  .btn:disabled { opacity: 0.5; cursor: default; }
</style>
