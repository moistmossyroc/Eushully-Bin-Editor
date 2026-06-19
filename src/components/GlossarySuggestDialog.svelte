<script lang="ts">
  /**
   * Scan all entries' original text for Katakana sequences not yet in the
   * glossary and surface them as one-click "Add" suggestions. Speeds up the
   * early-pass workflow where the user typically adds dozens of character /
   * place / item names manually.
   *
   * Katakana is the right signal: native loanwords and proper nouns are
   * almost always katakana-only sequences. We skip kanji and hiragana to
   * avoid noise from common-vocabulary substrings.
   */
  import { store } from "../lib/stores.svelte";
  import { persistAllSettingsDebounced } from "../lib/settings";
  import { translateSingleEntry } from "../lib/ipc";
  import { toastError } from "../lib/toast";
  import type { GlossaryEntry, DialogueEntry } from "../lib/types";

  interface Props {
    onClose: () => void;
    /** Mirrors the panel's Global/Project toggle. "global" scans every open
     *  tab and adds to the global glossary; "project" scans the active tab and
     *  adds to its glossary. Defaults to project for back-compat. */
    activeView?: "project" | "global";
  }
  let { onClose, activeView = "project" }: Props = $props();

  /** Which tabs to scan, per the panel view: all open tabs for global, just the
   *  active tab for project. */
  function scanTabs() {
    if (activeView === "global") return store.tabs;
    return store.activeTab ? [store.activeTab] : store.tabs;
  }

  /** Where added entries land — matches scan scope. */
  function getTargetGlossary(): GlossaryEntry[] {
    if (activeView === "global") return store.globalGlossary;
    return store.activeTab?.glossary ?? store.globalGlossary;
  }

  /** Match runs of katakana/kanji, optionally bridged by `の` (possessive
   *  particle). This catches compound names like "折玄の森" (Origen's Forest)
   *  where the proper-noun is glued by a particle, without admitting full
   *  hiragana words (which would flood the list with common vocabulary).
   *   - Katakana (U+30A0..U+30FF incl. ・ and ー; halfwidth U+FF66..U+FF9F)
   *   - Kanji   (U+4E00..U+9FFF CJK Unified Ideographs)
   *   - Bridge  (の only — by far the dominant particle in named compounds) */
  const TERM_RE = /[゠-ヿ一-鿿ｦ-ﾟ]+(?:の[゠-ヿ一-鿿ｦ-ﾟ]+)*/g;

  /** Junk filter: stretched-char runs and middle-dot-only strings. */
  function isWorthSuggesting(term: string): boolean {
    if (/^[ー・]+$/.test(term)) return false;
    return true;
  }

  /** A term has to carry enough non-hiragana signal to be a name worth
   *  suggesting. Pure kana/kanji needs ≥2 chars (e.g. "折玄"). Bridged
   *  compounds need ≥3 non-hiragana chars (e.g. "折玄の森" passes;
   *  "神の力" fails — too short to be a recurring proper-noun). */
  function passesLengthCheck(term: string): boolean {
    let hasHira = false;
    let nonHira = 0;
    for (const ch of term) {
      const cp = ch.codePointAt(0)!;
      if (cp >= 0x3040 && cp <= 0x309F) hasHira = true;
      else nonHira++;
    }
    return hasHira ? nonHira >= 3 : term.length >= 2;
  }

  /** Category drives the colored badge and the noise floor:
   *   - "katakana" pure: ≥1 occurrence (high precision)
   *   - "mixed" (kanji+katakana, or bridged with hiragana): ≥1 (likely proper noun)
   *   - "kanji" pure: needs higher count (compound vocab is noisy) */
  type TermClass = "katakana" | "mixed" | "kanji";
  function classify(term: string): TermClass {
    let hasKana = false, hasKanji = false, hasHira = false;
    for (const ch of term) {
      const cp = ch.codePointAt(0)!;
      if ((cp >= 0x30A0 && cp <= 0x30FF) || (cp >= 0xFF66 && cp <= 0xFF9F)) hasKana = true;
      else if (cp >= 0x4E00 && cp <= 0x9FFF) hasKanji = true;
      else if (cp >= 0x3040 && cp <= 0x309F) hasHira = true;
    }
    if (hasHira || (hasKana && hasKanji)) return "mixed";
    return hasKana ? "katakana" : "kanji";
  }

  /** User-controlled minimum-occurrence floor for kanji-only matches.
   *  Default 3 keeps the noise manageable; the user can dial it down to 1
   *  for small corpora where every occurrence matters. Katakana/mixed
   *  always show even at count=1 since they're high-precision signals. */
  let kanjiMinCount = $state(3);

  interface RawCandidate {
    term: string;
    count: number;
    cls: TermClass;
    /** Up to 2 example source sentences the term appears in. */
    examples: string[];
    /** Source BIN filenames the term was found in. */
    bins: string[];
  }

  /** The expensive scan — runs once when the dialog opens (or the view/tab set
   *  changes), NOT on every `kanjiMinCount` tweak. Yields a frame first so the
   *  dialog paints with a "Scanning…" state before a large all-tabs sweep. */
  let rawCandidates = $state<RawCandidate[]>([]);
  /** Exact-match translation memory built during the scan: original→translation
   *  across scanned tabs. Used for free offline suggestions. */
  let tmMap = $state<Map<string, string>>(new Map());
  let scanning = $state(true);

  function runScan(): { cands: RawCandidate[]; tm: Map<string, string> } {
    const counts = new Map<string, number>();
    const examples = new Map<string, string[]>();
    const bins = new Map<string, Set<string>>();
    const tm = new Map<string, string>();
    const existing = new Set(getTargetGlossary().map((g) => g.japanese));
    for (const tab of scanTabs()) {
      const binName = tab.sourceBinFilename || "(unsaved)";
      for (const e of tab.entries) {
        if (!e.original_text) continue;
        // Build TM from any translated row (exact original → translation).
        if (e.translated_text && e.translated_text.trim() && !tm.has(e.original_text)) {
          tm.set(e.original_text, e.translated_text.trim());
        }
        const matches = e.original_text.match(TERM_RE);
        if (!matches) continue;
        for (const m of matches) {
          if (existing.has(m)) continue;
          if (!isWorthSuggesting(m)) continue;
          if (!passesLengthCheck(m)) continue;
          counts.set(m, (counts.get(m) ?? 0) + 1);
          const ex = examples.get(m) ?? [];
          if (ex.length < 2 && !ex.includes(e.original_text)) ex.push(e.original_text);
          examples.set(m, ex);
          const bs = bins.get(m) ?? new Set<string>();
          bs.add(binName);
          bins.set(m, bs);
        }
      }
    }
    const cands = [...counts.entries()]
      .map(([term, count]) => ({
        term, count, cls: classify(term),
        examples: examples.get(term) ?? [],
        bins: [...(bins.get(term) ?? [])],
      }))
      .sort((a, b) => b.count - a.count);
    return { cands, tm };
  }

  // Re-scan when the dialog opens or the scan scope changes. Deps are read
  // explicitly so threshold/filter edits don't trigger a costly rescan.
  $effect(() => {
    void activeView;
    void store.tabs.map((t) => t.id); // track tab set identity
    scanning = true;
    const id = setTimeout(() => {
      const { cands, tm } = runScan();
      rawCandidates = cands;
      tmMap = tm;
      scanning = false;
    }, 0);
    return () => clearTimeout(id);
  });

  /** Cheap: apply the kanji-noise floor on top of the cached raw scan. */
  let candidates = $derived(
    rawCandidates.filter((c) => c.cls !== "kanji" || c.count >= kanjiMinCount),
  );

  let filter = $state("");
  let filtered = $derived.by(() => {
    if (!filter.trim()) return candidates;
    const q = filter.trim().toLowerCase();
    return candidates.filter((c) => c.term.toLowerCase().includes(q));
  });

  // ---- Per-term expansion: context + suggested translation ------------------
  let expandedTerm = $state<string | null>(null);
  /** term → suggested english (offline TM hit or provider result). */
  let suggestions = $state<Map<string, string>>(new Map());
  /** terms with an in-flight provider call. */
  let suggesting = $state<Set<string>>(new Set());

  function toggleExpand(term: string) {
    expandedTerm = expandedTerm === term ? null : term;
    // Pre-fill an offline suggestion the moment the row opens (free).
    if (expandedTerm === term && !suggestions.has(term)) {
      const hit = tmMap.get(term);
      if (hit) {
        suggestions.set(term, hit);
        suggestions = new Map(suggestions);
      }
    }
  }

  /** Provider-agnostic on-demand suggestion: routes through whatever LLM the
   *  user configured in Settings (Anthropic or OpenAI-compatible) via the same
   *  single-entry translate path the editor uses. */
  async function suggestTranslation(c: RawCandidate) {
    if (suggesting.has(c.term)) return;
    if (!store.apiKeyConfigured) {
      store.statusMessage = "Configure a translation provider/API key in Settings first";
      return;
    }
    suggesting.add(c.term);
    suggesting = new Set(suggesting);
    try {
      // Synthetic entry for the term; feed the example lines as context so the
      // model can localize the name from how it's actually used.
      const entry: DialogueEntry = {
        instruction_index: -1, opcode: 0, label: "show-text",
        original_text: c.term, translated_text: "", notes: "",
        is_context_row: false, string_arg_indices: [], deleted: false,
      };
      const context: DialogueEntry[] = c.examples.map((line, i) => ({
        instruction_index: -100 - i, opcode: 0, label: "show-text",
        original_text: line, translated_text: "", notes: "",
        is_context_row: true, string_arg_indices: [], deleted: false,
      }));
      // Resolve a model that works even in global view with no active tab.
      const model = store.activeTab?.activeModel
        ?? (store.apiProvider === "openai-compatible"
          ? store.lastOpenAiCompatModel
          : store.lastAnthropicModel);
      const result = await translateSingleEntry(
        entry, context, store.mergedGlossary, model,
        {
          blockCharCap: store.translatedCharCap,
          lineCharCap: store.translatedLineCharCap,
          enforceCharLimits: false,
          crossBinContext: null,
          minimalNotes: true,
          fastMode: store.fastMode,
          blockMode: false,
          contextBlocks: 0,
          scopedGlossary: store.scopedGlossary,
        },
      );
      suggestions.set(c.term, result.translation.trim());
      suggestions = new Map(suggestions);
    } catch (err) {
      toastError(`Suggestion failed: ${err}`);
    } finally {
      suggesting.delete(c.term);
      suggesting = new Set(suggesting);
    }
  }

  /** Track which suggestions the user already added so they don't add twice
   *  in one session. Cleared when the dialog closes. */
  let added = $state<Set<string>>(new Set());

  function addOne(term: string) {
    if (added.has(term)) return;
    const target = getTargetGlossary();
    if (target.some((g) => g.japanese === term)) {
      // Race: someone else just added it. Treat as success.
      added.add(term);
      added = new Set(added);
      return;
    }
    // Carry any suggested translation (offline hit or provider result).
    const english = suggestions.get(term) ?? "";
    target.push({ japanese: term, english, notes: "", category: "" });
    // Global glossary persists to settings; project rides tab.isDirty.
    if (activeView === "global") persistAllSettingsDebounced();
    else if (store.activeTab) store.activeTab.isDirty = true;
    added.add(term);
    added = new Set(added);
  }

  function addAllVisible() {
    for (const c of filtered) {
      if (!added.has(c.term)) addOne(c.term);
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="backdrop" onclick={onClose}></div>

<div class="dialog" role="dialog" aria-label="Glossary suggestions">
  <header class="dialog-header">
    <div>
      <h3>Suggest glossary entries</h3>
      <p class="scope">
        {#if activeView === "global"}
          Scanning all {store.tabs.length} open tab{store.tabs.length === 1 ? "" : "s"} → global glossary
        {:else}
          Scanning {store.activeTab?.sourceBinFilename ?? "active tab"} → project glossary
        {/if}
      </p>
    </div>
    <button class="close-btn" onclick={onClose} title="Close (Esc)">×</button>
  </header>

  <div class="filter-row">
    <input
      type="text"
      class="filter-input"
      placeholder="Filter suggestions..."
      bind:value={filter}
      autofocus
    />
    <span class="count">{scanning ? "scanning…" : `${filtered.length} / ${candidates.length}`}</span>
    {#if filtered.length > 0}
      <button class="add-all" onclick={addAllVisible}>Add all visible ({filtered.length})</button>
    {/if}
  </div>

  <div class="threshold-row">
    <label class="threshold-label" title="Kanji-only terms must appear at least this many times to be suggested. Katakana and mixed kanji+katakana terms always show.">
      Kanji min occurrences:
      <input
        type="number"
        class="threshold-input"
        min="1"
        max="20"
        bind:value={kanjiMinCount}
      />
    </label>
    <span class="threshold-hint">Lower = more kanji noise; higher = only common terms</span>
  </div>

  <div class="list">
    {#if scanning}
      <p class="empty">Scanning {activeView === "global" ? "all open tabs" : "the active tab"}…</p>
    {:else if candidates.length === 0}
      <p class="empty">No proper-noun terms found that aren't already in the glossary.</p>
    {:else if filtered.length === 0}
      <p class="empty">No matches.</p>
    {:else}
      {#each filtered as c (c.term)}
        {@const isOpen = expandedTerm === c.term}
        <div class="row" class:open={isOpen}>
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <button class="term" type="button" onclick={() => toggleExpand(c.term)} title="Show context + suggested translation">
            <span class="caret" class:rot={isOpen}>▸</span>{c.term}
          </button>
          <span
            class="class-badge class-{c.cls}"
            title={c.cls === "katakana"
              ? "Katakana — foreign words / loanwords / non-Japanese names"
              : c.cls === "mixed"
                ? "Mixed scripts — proper-noun compounds (kanji+katakana or kanji+particle+kanji like 折玄の森)"
                : "Kanji — kanji-only compound (filtered by min-count to reduce common-vocab noise)"}
          >
            {c.cls === "katakana" ? "Katakana" : c.cls === "mixed" ? "Mixed" : "Kanji"}
          </span>
          <span class="count-pill">×{c.count}</span>
          {#if added.has(c.term)}
            <span class="added">✓ added</span>
          {:else}
            <button class="add-btn" onclick={() => addOne(c.term)}>Add</button>
          {/if}
        </div>
        {#if isOpen}
          <div class="detail">
            <div class="detail-meta">
              Found in {c.bins.join(", ")} · {c.count} occurrence{c.count === 1 ? "" : "s"}
            </div>
            {#each c.examples as ex}
              <div class="example" title={ex}>{ex}</div>
            {/each}
            <div class="suggest-row">
              {#if suggestions.has(c.term)}
                <span class="suggest-label">Suggested:</span>
                <span class="suggest-value">{suggestions.get(c.term)}</span>
                <button class="suggest-btn" disabled={suggesting.has(c.term)} onclick={() => suggestTranslation(c)}>
                  {suggesting.has(c.term) ? "…" : "Re-suggest"}
                </button>
              {:else}
                <button
                  class="suggest-btn"
                  disabled={suggesting.has(c.term) || !store.apiKeyConfigured}
                  title={store.apiKeyConfigured
                    ? "Suggest a translation using your configured provider"
                    : "Configure a translation provider/API key in Settings"}
                  onclick={() => suggestTranslation(c)}
                >
                  {suggesting.has(c.term) ? "Suggesting…" : `Suggest translation (${store.apiProvider === "openai-compatible" ? "configured LLM" : "Claude"})`}
                </button>
              {/if}
            </div>
          </div>
        {/if}
      {/each}
    {/if}
  </div>

  <footer>
    <p class="hint">
      Added entries land in the {activeView === "global" ? "global" : "project"} glossary.
      Click a term for context + a suggested translation.
    </p>
    <button class="btn" onclick={onClose}>Done</button>
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
    width: 540px;
    max-width: 92vw;
    max-height: 82vh;
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
  .scope {
    margin: 2px 0 0 0;
    font-size: 0.72rem;
    color: #777799;
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
    min-width: 70px;
    text-align: right;
  }
  .add-all {
    background: #1a3a2e;
    border: 1px solid #2a5a3e;
    border-radius: 3px;
    color: #6fd886;
    font-size: 0.75rem;
    padding: 4px 10px;
    cursor: pointer;
  }
  .add-all:hover { background: #2a4a3a; }

  .list {
    flex: 1;
    overflow-y: auto;
    padding: 6px 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 14px;
    border-bottom: 1px solid #1a1a30;
  }
  .term {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    text-align: left;
    background: none;
    border: none;
    padding: 0;
    color: #c0c0e0;
    font-size: 0.9rem;
    cursor: pointer;
  }
  .term:hover { color: #8ab4f8; }
  .caret {
    display: inline-block;
    font-size: 0.7rem;
    color: #6a6a8a;
    transition: transform 0.1s;
  }
  .caret.rot { transform: rotate(90deg); }

  .row.open { background: #1a1a34; }
  .detail {
    padding: 8px 14px 10px 28px;
    background: #14142a;
    border-bottom: 1px solid #1a1a30;
  }
  .detail-meta {
    font-size: 0.72rem;
    color: #777799;
    margin-bottom: 6px;
  }
  .example {
    font-size: 0.8rem;
    color: #b0b0d0;
    padding: 3px 8px;
    margin-bottom: 4px;
    background: #1a1a3e;
    border-left: 2px solid #3a3a5a;
    border-radius: 0 3px 3px 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .suggest-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
  }
  .suggest-label { font-size: 0.72rem; color: #777799; }
  .suggest-value {
    flex: 1;
    font-size: 0.85rem;
    color: #6fd8a4;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .suggest-btn {
    background: #2a3a6e;
    border: 1px solid #4a5a9a;
    color: #c0d0ff;
    border-radius: 3px;
    font-size: 0.72rem;
    padding: 3px 10px;
    cursor: pointer;
  }
  .suggest-btn:hover:not(:disabled) { background: #3a4a8e; }
  .suggest-btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .count-pill {
    background: #2a2a4a;
    color: #8aa0ff;
    font-size: 0.72rem;
    padding: 1px 6px;
    border-radius: 8px;
    font-family: monospace;
  }
  .class-badge {
    font-size: 0.65rem;
    padding: 1px 7px;
    border-radius: 8px;
    font-weight: 600;
    min-width: 60px;
    text-align: center;
    text-transform: uppercase;
    letter-spacing: 0.3px;
  }
  .class-katakana { background: #2a3a5e; color: #8aa0ff; }
  .class-mixed { background: #3a2a5e; color: #c08aff; }
  .class-kanji { background: #3a2a1a; color: #e8a838; }

  .threshold-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 14px;
    border-bottom: 1px solid #2a2a4a;
    background: #1a1a30;
  }
  .threshold-label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 0.78rem;
    color: #c0c0e0;
    cursor: help;
  }
  .threshold-input {
    width: 50px;
    padding: 3px 6px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #1a1a3e;
    color: #c0c0e0;
    font-size: 0.8rem;
  }
  .threshold-hint {
    font-size: 0.7rem;
    color: #6a6a8a;
    font-style: italic;
  }
  .add-btn {
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    color: #c0c0e0;
    border-radius: 3px;
    font-size: 0.75rem;
    padding: 3px 10px;
    cursor: pointer;
    min-width: 50px;
  }
  .add-btn:hover { background: #2a2a5e; border-color: #5a7aff; }
  .added {
    color: #6fd886;
    font-size: 0.75rem;
    min-width: 50px;
    text-align: center;
  }

  .empty {
    padding: 24px 14px;
    text-align: center;
    color: #555577;
    font-size: 0.85rem;
  }

  footer {
    padding: 10px 16px;
    border-top: 1px solid #2a2a4a;
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .hint {
    flex: 1;
    margin: 0;
    color: #777799;
    font-size: 0.75rem;
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
