<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import {
    startTranslation,
    pauseTranslation,
    resumeTranslation,
    stopTranslation,
    scanCrossBinContext,
    autoSplitEntry,
  } from "../lib/ipc";
  import { store } from "../lib/stores.svelte";
  import { applySplitPlan, isAutoSplitContinuation } from "../lib/insertions";
  import { findOverflows, findBoxOverflows, EDITABLE_OVERFLOW_LABELS } from "../lib/overflow";
  import { toastWarn } from "../lib/toast";
  import { computeEmptyStats, bulkSetOmitExport, isStructuralOnly } from "../lib/exportOmit";
  import { furiganaDisplayLength, stripQuoteBrackets } from "../lib/types";
  import type { TranslationProgress, DialogueEntry, GlossaryEntry } from "../lib/types";
  import OverwriteConfirmDialog from "./OverwriteConfirmDialog.svelte";

  let isRunning = $derived(store.activeTab?.translationProgress.status === "Running");
  let isPaused = $derived(store.activeTab?.translationProgress.status === "Paused");
  let isIdle = $derived(
    !store.activeTab ||
      store.activeTab.translationProgress.status === "Idle" ||
      store.activeTab.translationProgress.status === "Stopped"
  );

  /** Mapping from custom endpoint URL → friendly label, used in the toolbar
   *  badge so the user sees "Mistral · model" instead of a bare URL. Kept in
   *  sync with PROVIDER_PRESETS in SettingsDialog. */
  const ENDPOINT_LABELS: Record<string, string> = {
    "https://openrouter.ai/api": "OpenRouter",
    "https://api.openai.com": "OpenAI",
    "https://api.mistral.ai": "Mistral",
    "https://api.groq.com/openai": "Groq",
    "http://localhost:11434": "Ollama",
    "http://localhost:1234": "LM Studio",
  };
  let openAiCompatLabel = $derived(
    ENDPOINT_LABELS[store.customEndpoint?.replace(/\/$/, "") ?? ""] ?? "OpenAI-compat"
  );

  const EDITABLE_LABELS = new Set(["show-text", "display-furigana", "comment", "set-string", "draw-string"]);
  /** Labels safe to auto-omit on translation completion. Excludes set-string
   *  and draw-string — those populate game-state slots (e.g. speaker name
   *  inside the engine's 【】 template) where empty produces breakage. */
  const AUTO_OMIT_LABELS = new Set(["show-text", "display-furigana", "comment"]);

  /** Count of untranslated dialogue entries */
  let untranslatedCount = $derived(
    store.activeTab?.entries.filter(
      (e) =>
        !e.is_context_row && !e.deleted && e.translated_text.trim() === ""
    ).length ?? 0
  );

  /** Entries that have translations */
  function countExistingTranslations() {
    const tab = store.activeTab;
    if (!tab) return { user: 0, claude: 0, empty: 0 };
    let user = 0, claude = 0, empty = 0;
    for (const e of tab.entries) {
      if (e.is_context_row || e.deleted || !EDITABLE_LABELS.has(e.label)) continue;
      if (e.translated_text.trim() === "") {
        empty++;
      } else if (e.notes.includes("[LLM:")) {
        claude++;
      } else {
        user++;
      }
    }
    return { user, claude, empty };
  }

  /** Compute display length for a single entry, accounting for furigana and 「」 */
  function entryDisplayLength(e: DialogueEntry): number {
    const baseLen = e.label === "display-furigana"
      ? furiganaDisplayLength(e.translated_text)
      : e.translated_text.length;
    return stripQuoteBrackets(baseLen, e.translated_text);
  }

  /** Count over-limit entries for the review-overflows button badge.
   *  Uses the same line-group logic as the visual indicator and the dialog. */
  let overLimitCount = $derived.by(() => {
    const tab = store.activeTab;
    if (!tab) return 0;
    return findOverflows(tab.entries, store.translatedLineCharCap).length;
  });

  /** Boxes over the per-box limits (line count / total chars) — usually a
   *  by-product of auto-split. Surfaced in the same Review overflows button. */
  let boxOverLimitCount = $derived.by(() => {
    const tab = store.activeTab;
    if (!tab) return 0;
    return findBoxOverflows(tab.entries, store.translatedCharCap, store.maxLinesPerBox).length;
  });

  /** Count of entries whose original_text matches a glossary japanese key and have no translation yet */
  let glossaryMatchCount = $derived.by(() => {
    const tab = store.activeTab;
    if (!tab) return 0;
    const glossary = store.mergedGlossary;
    if (glossary.length === 0) return 0;

    const glossaryMap = new Map<string, string>();
    for (const g of glossary) {
      if (g.japanese && g.english) glossaryMap.set(g.japanese, g.english);
    }

    let count = 0;
    for (const e of tab.entries) {
      if (e.is_context_row || e.deleted || !EDITABLE_LABELS.has(e.label)) continue;
      if (e.translated_text.trim() !== "") continue;
      if (glossaryMap.has(e.original_text)) count++;
    }
    return count;
  });

  /** Apply glossary matches: fill translated_text from glossary for exact original_text matches */
  function handleApplyGlossary() {
    const tab = store.activeTab;
    if (!tab) return;

    const glossaryMap = new Map<string, string>();
    for (const g of store.mergedGlossary) {
      if (g.japanese && g.english) glossaryMap.set(g.japanese, g.english);
    }

    let appliedCount = 0;
    for (let i = 0; i < tab.entries.length; i++) {
      const e = tab.entries[i];
      if (e.is_context_row || e.deleted || !EDITABLE_LABELS.has(e.label)) continue;
      if (e.translated_text.trim() !== "") continue;

      const match = glossaryMap.get(e.original_text);
      if (match) {
        tab.entries[i].translated_text = match;
        tab.entries[i].notes = "[Glossary] Auto-filled from glossary match";
        appliedCount++;
      }
    }

    if (appliedCount > 0) {
      tab.isDirty = true;
    }
    store.statusMessage = appliedCount > 0
      ? `Applied ${appliedCount} glossary matches`
      : "No glossary matches found for untranslated entries";
  }

  let showOverwriteDialog = $state(false);
  let overwriteCounts = $state({ user: 0, claude: 0, empty: 0 });

  /** Stats for eligible empty cells, used to drive the toggle button. */
  let emptyStats = $derived(computeEmptyStats(store.activeTab));

  /** Toggle button state — true when ALL eligible empties are omitted. */
  let allEmptiesOmitted = $derived(
    emptyStats.eligible > 0 && emptyStats.omitted === emptyStats.eligible,
  );

  /** Single-click toggle: flips all eligible empty cells to the opposite state.
   *  Bias toward omit — if currently mixed or all included, click → omit all. */
  function handleToggleEmpties() {
    const value = !allEmptiesOmitted;
    const changed = bulkSetOmitExport(store.activeTab, value);
    store.statusMessage = value
      ? `Omitted ${changed} empty cell(s) from export.`
      : `Re-included ${changed} empty cell(s) in export.`;
  }

  /** Format token count with K/M suffix */
  function formatTokens(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
    if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
    return n.toString();
  }

  /** Check for existing translations before starting batch */
  function handleTranslateClick() {
    const tab = store.activeTab;
    if (!tab) return;
    if (!store.apiKeyConfigured) {
      store.statusMessage = "Set your API key in Settings first";
      return;
    }

    const counts = countExistingTranslations();

    // If all entries are empty, go straight to translation
    if (counts.user === 0 && counts.claude === 0) {
      doStartTranslation();
      return;
    }

    // Show the overwrite confirmation dialog
    overwriteCounts = counts;
    showOverwriteDialog = true;
  }

  function handleOverwriteConfirm(mode: "skip" | "overwrite-claude" | "overwrite-all") {
    showOverwriteDialog = false;
    const tab = store.activeTab;
    if (!tab) return;

    if (mode === "overwrite-claude") {
      // Clear Claude-translated entries so they get re-translated
      for (let i = 0; i < tab.entries.length; i++) {
        const e = tab.entries[i];
        if (e.is_context_row || e.deleted || !EDITABLE_LABELS.has(e.label)) continue;
        if (e.translated_text.trim() !== "" && e.notes.includes("[LLM:")) {
          tab.entries[i].translated_text = "";
          tab.entries[i].notes = "";
        }
      }
      tab.isDirty = true;
    } else if (mode === "overwrite-all") {
      // Clear all translated entries
      for (let i = 0; i < tab.entries.length; i++) {
        const e = tab.entries[i];
        if (e.is_context_row || e.deleted || !EDITABLE_LABELS.has(e.label)) continue;
        if (e.translated_text.trim() !== "") {
          tab.entries[i].translated_text = "";
          tab.entries[i].notes = "";
        }
      }
      tab.isDirty = true;
    }
    // mode === "skip" — leave entries as-is, the batch skips non-empty entries

    doStartTranslation();
  }

  async function doStartTranslation() {
    const tab = store.activeTab;
    if (!tab) return;
    if (!store.apiKeyConfigured) {
      store.statusMessage = "Set your API key in Settings first";
      return;
    }
    // Clear stale error from a previous run — fresh start, fresh slate.
    tab.stickyTranslationError = null;
    try {
      // Scan sibling BINs for narrative context if enabled
      let crossBinCtx: string | null = null;
      if (store.crossBinContext && tab.binPath) {
        store.statusMessage = "Scanning sibling BIN files for context...";
        try {
          const ctx = await scanCrossBinContext(tab.binPath);
          if (ctx.length > 0) crossBinCtx = ctx;
        } catch {
          // Non-critical — proceed without cross-bin context
        }
      }

      store.statusMessage = "Starting translation...";
      await startTranslation(tab.id, tab.entries, store.promptGlossary, tab.claudeModel, {
        blockCharCap: store.translatedCharCap,
        lineCharCap: store.translatedLineCharCap,
        enforceCharLimits: store.enforceCharLimits,
        crossBinContext: crossBinCtx,
        minimalNotes: store.minimalNotes,
        fastMode: store.fastMode,
        blockMode: store.blockMode,
        contextBlocks: store.contextBlocks,
        scopedGlossary: store.scopedGlossary,
      });
    } catch (err) {
      store.statusMessage = `Error: ${err}`;
    }
  }

  async function handlePause() {
    const tab = store.activeTab;
    if (!tab) return;
    // Optimistic update: flip status to Paused immediately so the Resume button appears
    // even if the current API request is mid-flight (backend confirms at next loop iteration)
    tab.translationProgress = { ...tab.translationProgress, status: "Paused" };
    try {
      await pauseTranslation(tab.id);
    } catch (err) {
      store.statusMessage = `Error: ${err}`;
    }
  }

  async function handleResume() {
    const tab = store.activeTab;
    if (!tab) return;
    try {
      await resumeTranslation(tab.id);
    } catch (err) {
      store.statusMessage = `Error: ${err}`;
    }
  }

  async function handleStop() {
    const tab = store.activeTab;
    if (!tab) return;
    try {
      await stopTranslation(tab.id);
    } catch (err) {
      store.statusMessage = `Error: ${err}`;
    }
  }

  /** Find tab by ID across all open tabs */
  function findTab(tabId: string) {
    return store.tabs.find((t) => t.id === tabId);
  }

  onMount(() => {
    // Listen for progress updates from the backend
    const unlistenProgress = listen<TranslationProgress & { tab_id: string }>(
      "translation-progress",
      (event) => {
        const tab = findTab(event.payload.tab_id);
        if (!tab) return;
        tab.translationProgress = event.payload;

        if (tab === store.activeTab) {
          const fastSuffix = store.fastMode ? " (Fast mode)" : "";
          const p = tab.translationProgress;
          const blockSuffix =
            p.total_blocks != null && p.block_index != null
              ? ` — block ${p.block_index + 1}/${p.total_blocks}`
              : "";
          if (p.status === "Running") {
            store.statusMessage = `Translating ${p.current_index + 1}/${p.total_entries}${blockSuffix}${fastSuffix}...`;
          } else if (p.status === "Paused") {
            store.statusMessage = `Translation paused${blockSuffix}${fastSuffix}`;
          } else if (tab.translationProgress.status === "Stopped") {
            store.statusMessage = "Translation stopped";
          }
        }
      }
    );

    // Listen for individual entry translations
    const unlistenEntry = listen<{
      tab_id: string;
      index: number;
      entry: DialogueEntry;
    }>("translation-entry-updated", (event) => {
      const tab = findTab(event.payload.tab_id);
      if (!tab) return;
      const { index, entry } = event.payload;
      if (index >= 0 && index < tab.entries.length) {
        tab.entries[index] = entry;
        tab.isDirty = true;

        // Post-translation auto-split: if enabled and the row overflows, split it.
        // Restricted to rendered dialogue labels (show-text, display-furigana) —
        // comments/set-string/draw-string don't render in-game so they have no
        // line cap concept.
        if (
          store.autoSplitEnabled &&
          store.translatedLineCharCap > 0 &&
          entry.translated_text.trim() !== "" &&
          !entry.is_context_row &&
          !entry.deleted &&
          EDITABLE_OVERFLOW_LABELS.has(entry.label) &&
          // Line-wrapping within a box needs no bridge; "unavailable" only
          // blocks spilling into an extra box, handled at split time.
          tab.bridgeTemplateQuality
        ) {
          const len = entryDisplayLength(entry);
          const lineCap = store.translatedLineCharCap;

          // Only split if actually over the line cap
          if (len > lineCap) {
            // Idempotency: skip if this entry already has split children
            const alreadySplit = tab.entries.some(
              (e) =>
                isAutoSplitContinuation(e) &&
                e.notes.includes(`row #${entry.instruction_index},`),
            );
            if (!alreadySplit) {
              // Snapshot box-overflow count so we can tell if THIS split
              // produced a box that exceeds the per-box limits (too many
              // lines / over total cap) — e.g. a degraded bridge template
              // can't open a new box, so the split overfills one.
              const boxBefore = findBoxOverflows(
                tab.entries,
                store.translatedCharCap,
                store.maxLinesPerBox,
              ).length;
              autoSplitEntry(
                tab.id,
                entry.translated_text,
                entry.instruction_index,
                lineCap,
                store.translatedCharCap,
                store.maxLinesPerBox,
              )
                .then((plan) => {
                  applySplitPlan(plan);
                  const boxAfter = findBoxOverflows(
                    tab.entries,
                    store.translatedCharCap,
                    store.maxLinesPerBox,
                  ).length;
                  if (boxAfter > boxBefore) {
                    toastWarn(
                      `Auto-split overfilled a text box (over ${store.maxLinesPerBox} lines${store.translatedCharCap > 0 ? ` / ${store.translatedCharCap} chars` : ""}). Open “Review overflows” to fix it.`,
                    );
                  }
                })
                .catch((err) => {
                  console.warn("Auto-split failed for entry", index, err);
                });
            }
          }
        }
      }
    });

    // Listen for translation completion
    const unlistenComplete = listen<{ tab_id: string; total_entries: number }>(
      "translation-complete",
      (event) => {
        const tab = findTab(event.payload.tab_id);
        if (!tab) return;
        tab.translationProgress.status = "Idle";

        // Auto-omit any editable row that's still untranslated. Mirrors the
        // CC-import behavior so the preview/export are clean by default.
        // Excludes set-string / draw-string — those populate game-state
        // slots where empty produces visible breakage (e.g. speaker name
        // disappearing inside the engine's 【】 bracket template).
        // Build a fresh array so {#each} child components re-render.
        let autoOmitted = 0;
        const next = tab.entries.map((e) => {
          if (e.deleted || e.is_context_row) return e;
          if (!AUTO_OMIT_LABELS.has(e.label)) return e;
          if (e.translated_text.trim() !== "") return e;
          // Don't auto-omit structural-only rows (brackets, empty refs) —
          // they're engine markup that must keep their original text.
          if (isStructuralOnly(e.original_text ?? "")) return e;
          if (!e.original_text || e.original_text.trim() === "") return e;
          if (e.omit_export === true) return e;
          autoOmitted++;
          return { ...e, omit_export: true };
        });
        if (autoOmitted > 0) {
          tab.entries = next;
          tab.isDirty = true;
        }

        if (tab === store.activeTab) {
          const tail = autoOmitted > 0 ? ` (${autoOmitted} empty rows auto-omitted)` : "";
          store.statusMessage = `Translation complete — ${event.payload.total_entries} entries processed${tail}`;
        }
      }
    );

    // Listen for translation errors
    const unlistenError = listen<{ tab_id: string; error: string }>(
      "translation-error",
      (event) => {
        const tab = findTab(event.payload.tab_id);
        if (!tab) return;
        // Sticky error survives subsequent progress emits so the user can read
        // the failure reason even after a few more rows roll past. Cleared
        // when they dismiss it or start a fresh translation.
        tab.stickyTranslationError = event.payload.error;
        if (tab === store.activeTab) {
          store.statusMessage = `Translation error: ${event.payload.error}`;
        }
      }
    );

    // Listen for new glossary terms auto-added by Claude
    const unlistenNewTerm = listen<{ tab_id: string; entry: GlossaryEntry }>(
      "glossary-new-term",
      (event) => {
        const tab = findTab(event.payload.tab_id);
        if (!tab) return;
        // Auto-add to tab-local glossary if not already present
        const exists = tab.glossary.some(
          (g) => g.japanese === event.payload.entry.japanese
        );
        if (!exists) {
          tab.glossary = [...tab.glossary, event.payload.entry];
          tab.isDirty = true;
        }
      }
    );

    // Listen for glossary update proposals that need user review
    const unlistenReview = listen<{
      tab_id: string;
      existing: GlossaryEntry;
      proposed: GlossaryEntry;
      source_entry_index: number;
    }>("glossary-review-pending", (event) => {
      store.glossaryReviewQueue = [
        ...store.glossaryReviewQueue,
        {
          tabId: event.payload.tab_id,
          existing: event.payload.existing,
          proposed: event.payload.proposed,
          sourceEntryIndex: event.payload.source_entry_index,
        },
      ];
      // Auto-open the review dialog when items arrive
      if (!store.glossaryReviewDialogOpen) {
        store.glossaryReviewDialogOpen = true;
      }
    });

    return () => {
      unlistenProgress.then((fn) => fn());
      unlistenEntry.then((fn) => fn());
      unlistenComplete.then((fn) => fn());
      unlistenError.then((fn) => fn());
      unlistenNewTerm.then((fn) => fn());
      unlistenReview.then((fn) => fn());
    };
  });
</script>

<div class="translation-controls">
  <div class="controls-left">
    {#if store.apiProvider === "openai-compatible"}
      <button
        type="button"
        class="model-badge clickable"
        title="Click to change provider or model in Settings."
        onclick={() => (store.settingsDialogOpen = true)}
      >
        <span class="model-badge-provider">{openAiCompatLabel}</span>
        <span class="model-badge-sep">·</span>
        <span class="model-badge-model">
          {store.activeTab?.openAiCompatModel || "no model set — click to configure"}
        </span>
      </button>
    {:else if !store.apiKeyConfigured || store.availableModels.length === 0}
      <button
        type="button"
        class="model-badge clickable placeholder"
        title="Click to open Settings and add your Anthropic API key."
        onclick={() => (store.settingsDialogOpen = true)}
      >
        Configure API Key to load Claude models →
      </button>
    {:else}
      <select
        class="model-select"
        bind:value={store.activeTab!.anthropicModel}
        disabled={!isIdle}
      >
        {#each store.availableModels as model}
          <option value={model.id}>{model.label}</option>
        {/each}
      </select>
    {/if}

    {#if isIdle}
      <button
        class="ctrl-btn start"
        onclick={handleTranslateClick}
        title="Translate entries using Claude"
      >
        Translate ({untranslatedCount})
      </button>

      {#if glossaryMatchCount > 0}
        <button
          class="ctrl-btn apply-glossary"
          onclick={handleApplyGlossary}
          title={`${glossaryMatchCount} untranslated entries match glossary terms — auto-fill translations`}
        >
          Apply Glossary ({glossaryMatchCount})
        </button>
      {/if}

      {#if overLimitCount > 0 || boxOverLimitCount > 0}
        <button
          class="ctrl-btn optimize"
          class:has-box-overflow={boxOverLimitCount > 0}
          onclick={() => (store.overflowReviewDialogOpen = true)}
          title={[
            overLimitCount > 0 ? `${overLimitCount} line(s) over the per-line cap` : "",
            boxOverLimitCount > 0 ? `${boxOverLimitCount} text box(es) over the per-box limits (won't render fully in-game)` : "",
          ].filter(Boolean).join(" · ") + " — review and fix"}
        >
          Review overflows ({overLimitCount + boxOverLimitCount})
        </button>
      {/if}

      {#if store.mergedGlossary.some((g) => g.english.trim())}
        <button
          class="ctrl-btn"
          onclick={() => (store.glossaryConsistencyOpen = true)}
          title="Flag translated rows where a glossary term's Japanese appears but its English rendering is missing (drift check)"
        >
          Glossary check
        </button>
        <button
          class="ctrl-btn"
          onclick={() => (store.glossaryAutoApplyOpen = true)}
          title="Review safe glossary substitutions: fill exact-term rows + replace Japanese terms that leaked into the English. Nothing applies until you confirm."
        >
          Glossary auto-apply
        </button>
      {/if}

      {#if emptyStats.eligible > 0}
        <button
          class="ctrl-btn empty-cells"
          class:active={allEmptiesOmitted}
          onclick={handleToggleEmpties}
          oncontextmenu={(e) => { e.preventDefault(); store.omittedCellsDialogOpen = true; }}
          title={(allEmptiesOmitted
            ? `All ${emptyStats.eligible} empty cell(s) are currently OMITTED from export. Click to re-include (original Japanese will render in-game).`
            : `${emptyStats.eligible - emptyStats.omitted} of ${emptyStats.eligible} empty cell(s) will export their original Japanese. Click to OMIT all (BIN gets empty strings — game renders nothing).`)
            + ` — Right-click to view omitted cells & rescan.`}
        >
          {#if allEmptiesOmitted}
            Empty cells omitted ({emptyStats.eligible})
          {:else}
            Omit empty cells ({emptyStats.eligible - emptyStats.omitted})
          {/if}
        </button>
      {/if}
    {:else if isRunning}
      <button class="ctrl-btn pause" onclick={handlePause}>
        Pause
      </button>
      <button class="ctrl-btn stop" onclick={handleStop}>
        Stop
      </button>
    {:else if isPaused}
      <button class="ctrl-btn resume" onclick={handleResume}>
        Resume
      </button>
      <button class="ctrl-btn stop" onclick={handleStop}>
        Stop
      </button>
    {/if}
  </div>

  <div class="controls-right">
    {#if !isIdle}
      <div class="progress-info">
        <span class="progress-text">
          {store.activeTab!.translationProgress.current_index}/{store.activeTab!.translationProgress.total_entries}
        </span>
        <div class="progress-bar">
          <div
            class="progress-fill"
            style="width: {store.activeTab!.translationProgress.total_entries > 0
              ? (store.activeTab!.translationProgress.current_index / store.activeTab!.translationProgress.total_entries) * 100
              : 0}%"
          ></div>
        </div>
      </div>
    {/if}

    <div class="token-display" title="Input / Output tokens used">
      <span class="token-label">Tokens:</span>
      <span class="token-in">{formatTokens(store.activeTab!.translationProgress.total_input_tokens)} in</span>
      <span class="token-sep">/</span>
      <span class="token-out">{formatTokens(store.activeTab!.translationProgress.total_output_tokens)} out</span>
    </div>

    {#if store.activeTab!.translationProgress.last_error}
      {@const msg = store.activeTab!.translationProgress.last_error}
      {@const kind = msg.startsWith("Rate limited") ? "waiting"
        : msg.startsWith("Transient error") ? "retrying"
        : "error"}
      {@const label = kind === "waiting" ? "Waiting"
        : kind === "retrying" ? "Retrying"
        : "Error"}
      <span class="status-indicator {kind}" title={msg}>
        {label}
      </span>
    {/if}

    {#if store.activeTab!.stickyTranslationError}
      <div class="sticky-error" title={store.activeTab!.stickyTranslationError}>
        <span class="sticky-error-icon">⚠</span>
        <span class="sticky-error-text">{store.activeTab!.stickyTranslationError}</span>
        <button
          type="button"
          class="sticky-error-dismiss"
          onclick={() => { if (store.activeTab) store.activeTab.stickyTranslationError = null; }}
          aria-label="Dismiss error"
          title="Dismiss"
        >×</button>
      </div>
    {/if}
  </div>
</div>

{#if showOverwriteDialog}
  <OverwriteConfirmDialog
    userCount={overwriteCounts.user}
    claudeCount={overwriteCounts.claude}
    emptyCount={overwriteCounts.empty}
    onConfirm={handleOverwriteConfirm}
    onCancel={() => (showOverwriteDialog = false)}
  />
{/if}

<style>
  .translation-controls {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 12px;
    background: #141430;
    border-bottom: 1px solid #2a2a4a;
  }

  .controls-left,
  .controls-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .model-select {
    padding: 4px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    font-size: 0.8rem;
    cursor: pointer;
    outline: none;
  }

  .model-select:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .model-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    font-size: 0.8rem;
    line-height: 1;
    color: #c0c0e0;
    user-select: none;
    font-family: inherit;
  }

  .model-badge.clickable {
    cursor: pointer;
    transition: background 120ms ease, border-color 120ms ease;
  }
  .model-badge.clickable:hover {
    background: #25254e;
    border-color: #5a5a7a;
  }

  .model-badge.placeholder {
    color: #ffb86b;
    border-color: #5a4a2a;
    background: #2a1f10;
  }
  .model-badge.placeholder:hover {
    background: #3a2a15;
    border-color: #8a7040;
  }

  .model-badge-provider {
    color: #8aa0ff;
    font-weight: 600;
  }

  .model-badge-sep {
    color: #555577;
  }

  .model-badge-model {
    color: #c0c0e0;
  }

  .ctrl-btn {
    padding: 4px 12px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    font-size: 0.8rem;
    cursor: pointer;
    transition: all 0.15s;
  }

  .ctrl-btn.start {
    background: #1a3a2e;
    color: #6fd886;
    border-color: #2a5a3e;
  }

  .ctrl-btn.start:hover:not(:disabled) {
    background: #2a4a3e;
    border-color: #4a7a5e;
  }

  .ctrl-btn.start:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .ctrl-btn.apply-glossary {
    background: #1a2a3e;
    color: #8ab4f8;
    border-color: #2a4a6a;
  }

  .ctrl-btn.apply-glossary:hover {
    background: #2a3a5e;
    border-color: #4a6a8a;
  }

  .ctrl-btn.optimize {
    background: #2a2a1a;
    color: #e0a030;
    border-color: #4a4a2a;
  }

  .ctrl-btn.optimize:hover {
    background: #3a3a2a;
    border-color: #6a6a3a;
  }

  /* Box overflows are more severe than line overflows (text gets dropped
   * in-game), so flag the button red when any box is over the per-box limits. */
  .ctrl-btn.optimize.has-box-overflow {
    background: #3a1f1a;
    color: #f0a070;
    border-color: #6a3a2a;
  }
  .ctrl-btn.optimize.has-box-overflow:hover {
    background: #4a2a22;
    border-color: #8a4a36;
  }

  .optimizing-label {
    font-size: 0.8rem;
    color: #e0a030;
    padding: 4px 8px;
  }

  .ctrl-btn.pause {
    background: #3a3a1a;
    color: #e0d060;
    border-color: #5a5a2a;
  }

  .ctrl-btn.pause:hover {
    background: #4a4a2a;
  }

  .ctrl-btn.resume {
    background: #1a3a2e;
    color: #6fd886;
    border-color: #2a5a3e;
  }

  .ctrl-btn.resume:hover {
    background: #2a4a3e;
  }

  .ctrl-btn.stop {
    background: #3a1a1a;
    color: #e06060;
    border-color: #5a2a2a;
  }

  .ctrl-btn.stop:hover {
    background: #4a2a2a;
  }

  .progress-info {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .progress-text {
    font-size: 0.75rem;
    color: #888;
    min-width: 50px;
    text-align: right;
  }

  .progress-bar {
    width: 100px;
    height: 6px;
    background: #2a2a4a;
    border-radius: 3px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: #6c9bd2;
    border-radius: 3px;
    transition: width 0.3s ease;
  }

  .token-display {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 0.75rem;
    color: #666;
  }

  .token-label {
    color: #555;
  }

  .token-in {
    color: #8ab4f8;
  }

  .token-sep {
    color: #444;
  }

  .token-out {
    color: #a4d4a8;
  }

  .error-indicator,
  .status-indicator {
    padding: 2px 6px;
    border-radius: 3px;
    font-size: 0.7rem;
    cursor: help;
  }
  .error-indicator,
  .status-indicator.error {
    background: #3a1a1a;
    color: #e06060;
  }
  .status-indicator.retrying {
    background: #3a2a10;
    color: #e8a060;
  }
  .status-indicator.waiting {
    background: #2a2a10;
    color: #d8d060;
  }

  .sticky-error {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    margin-left: 8px;
    max-width: 480px;
    border: 1px solid #6a2a2a;
    border-radius: 4px;
    background: #2a1010;
    color: #ffb0b0;
    font-size: 0.78rem;
    line-height: 1.2;
  }
  .sticky-error-icon { flex: 0 0 auto; color: #ff8060; }
  .sticky-error-text {
    flex: 1 1 auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sticky-error-dismiss {
    flex: 0 0 auto;
    background: transparent;
    border: none;
    color: #ffb0b0;
    font-size: 1rem;
    line-height: 1;
    cursor: pointer;
    padding: 0 4px;
  }
  .sticky-error-dismiss:hover { color: #fff; }

  .ctrl-btn.empty-cells {
    background: #2a2a4e;
    border-color: #4a4a7a;
    color: #c0c0e0;
  }

  .ctrl-btn.empty-cells:hover {
    background: #3a3a6e;
  }

  .ctrl-btn.empty-cells.active {
    background: #3a1a2a;
    border-color: #6a3a4a;
    color: #e0a0b0;
  }

  .ctrl-btn.empty-cells.active:hover {
    background: #4a2a3a;
  }
</style>
