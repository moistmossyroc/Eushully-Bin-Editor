<script lang="ts">
  import { store } from "../lib/stores.svelte";
  import { furiganaDisplayLength, stripQuoteBrackets } from "../lib/types";
  import { proofreadBlock } from "../lib/ipc";
  import type { DialogueEntry } from "../lib/types";
  import { applyBoxProfile, binPrefix, defaultEngineMetrics, slotGeometry } from "../lib/gameLayout";
  import type { WrapMode } from "../lib/gameLayout";
  import GamePreviewBox from "./GamePreviewBox.svelte";
  import BoxProfilePanel from "./BoxProfilePanel.svelte";

  // The active tab's text layout is loaded by App (engineMode.ensureTextLayout).
  let textLayout = $derived(store.activeTab ? store.textLayoutByTab[store.activeTab.id] ?? null : null);
  let prefix = $derived(binPrefix(store.activeTab?.sourceBinFilename ?? ""));
  let previewSlot = $derived(store.previewSlotByPrefix[prefix] ?? 1);
  // An enabled text box profile previews on the base box (what the export patches).
  let profileEntry = $derived(textLayout?.gameDir ? store.boxProfiles[textLayout.gameDir] : undefined);
  let boxGeometry = $derived(
    !textLayout
      ? null
      : previewSlot === 1 && profileEntry?.enabled && textLayout.baseBody
        ? applyBoxProfile(textLayout.baseBody, profileEntry.profile)
        : slotGeometry(textLayout, previewSlot),
  );

  // The engine's row breaking: reported metrics for the dialogue box (loader word-wrap
  // hook ran → fanpatch_layout.txt); its word wrap whenever the setting is on and the
  // game has reported (the report only exists where the hook works).
  let wrapMode = $derived<WrapMode | null>(
    !boxGeometry
      ? null
      : {
          metrics: previewSlot === 1 && textLayout?.metrics ? textLayout.metrics : defaultEngineMetrics(boxGeometry),
          wordWrap: store.fanpatchWordWrap && !!textLayout?.metrics,
        },
  );

  function useBottomLineCap(cap: number) {
    store.bottomLineCharCap = cap;
    store.statusMessage = `Bottom-line cap set to ${cap} (from the window's button row)`;
    void import("../lib/settings").then((m) => m.persistAllSettingsDebounced());
  }

  function choosePreviewSlot(slot: number) {
    store.previewSlotByPrefix = { ...store.previewSlotByPrefix, [prefix]: slot };
    void import("../lib/settings").then((m) => m.persistAllSettingsDebounced());
  }

  /** Indices into tab.entries for the editable rows in the current block */
  let blockEditableIndices = $derived.by(() => {
    const tab = store.activeTab;
    if (!tab || tab.selectedRowIndex === null) return [] as number[];
    const entries = tab.entries;
    const selectedIdx = tab.selectedRowIndex;
    const selectedEntry = entries[selectedIdx];
    if (!selectedEntry || selectedEntry.label !== "show-text" && selectedEntry.label !== "display-furigana") {
      return [];
    }

    let blockStart = 0;
    for (let i = selectedIdx - 1; i >= 0; i--) {
      if (entries[i].label === "wait-for-input") { blockStart = i + 1; break; }
    }
    let blockEnd = entries.length;
    for (let i = selectedIdx + 1; i < entries.length; i++) {
      if (entries[i].label === "wait-for-input") { blockEnd = i; break; }
    }

    const out: number[] = [];
    for (let i = blockStart; i < blockEnd; i++) {
      const e = entries[i];
      if (e.deleted) continue;
      if (e.label === "show-text" || e.label === "display-furigana") {
        out.push(i);
      }
    }
    return out;
  });

  let proofreading = $state(false);
  let proofreadDiff = $state<{
    indices: number[];
    originals: string[];
    proposed: string[];
    notes: string;
  } | null>(null);

  async function handleProofread() {
    const tab = store.activeTab;
    if (!tab) return;
    if (!store.apiKeyConfigured) {
      store.statusMessage = "Set your API key in Settings first";
      return;
    }
    const indices = blockEditableIndices;
    if (indices.length === 0) return;

    const blockEntries: DialogueEntry[] = indices.map((i) => tab.entries[i]);
    const hasAnyTranslation = blockEntries.some((e) => e.translated_text.trim() !== "");
    if (!hasAnyTranslation) {
      store.statusMessage = "Nothing to proofread — block has no translations yet";
      return;
    }

    proofreading = true;
    store.statusMessage = "Proofreading dialog block...";
    try {
      const result = await proofreadBlock(
        blockEntries,
        store.promptGlossary,
        tab.claudeModel,
        store.translatedLineCharCap,
        store.translatedCharCap,
        store.enforceCharLimits,
      );
      proofreadDiff = {
        indices,
        originals: blockEntries.map((e) => e.translated_text),
        proposed: result.translations,
        notes: result.notes,
      };
      store.statusMessage = "Proofread complete — review changes";
    } catch (err) {
      store.statusMessage = `Proofread error: ${err}`;
    } finally {
      proofreading = false;
    }
  }

  function applyProofreadAll() {
    const tab = store.activeTab;
    if (!tab || !proofreadDiff) return;
    let changed = 0;
    for (let i = 0; i < proofreadDiff.indices.length; i++) {
      const idx = proofreadDiff.indices[i];
      const newText = proofreadDiff.proposed[i];
      if (newText !== proofreadDiff.originals[i]) {
        tab.entries[idx].translated_text = newText;
        const existingNotes = tab.entries[idx].notes;
        const proofreadTag = `[Proofread: ${tab.claudeModel}]`;
        tab.entries[idx].notes = existingNotes.includes(proofreadTag)
          ? existingNotes
          : `${proofreadTag} ${existingNotes}`.trim();
        changed++;
      }
    }
    if (changed > 0) tab.isDirty = true;
    store.statusMessage = `Proofread applied — ${changed} entries updated`;
    proofreadDiff = null;
  }

  function applyProofreadOne(slotIdx: number) {
    const tab = store.activeTab;
    if (!tab || !proofreadDiff) return;
    const entryIdx = proofreadDiff.indices[slotIdx];
    const newText = proofreadDiff.proposed[slotIdx];
    if (newText === proofreadDiff.originals[slotIdx]) return;
    tab.entries[entryIdx].translated_text = newText;
    const existingNotes = tab.entries[entryIdx].notes;
    const proofreadTag = `[Proofread: ${tab.claudeModel}]`;
    tab.entries[entryIdx].notes = existingNotes.includes(proofreadTag)
      ? existingNotes
      : `${proofreadTag} ${existingNotes}`.trim();
    tab.isDirty = true;
    // Mark this slot as accepted by setting original = proposed so the diff hides
    proofreadDiff.originals[slotIdx] = newText;
  }

  function dismissProofread() {
    proofreadDiff = null;
    store.statusMessage = "Proofread dismissed";
  }

  /**
   * Collects all show-text / display-furigana lines in the same text block
   * as the currently selected row (bounded by end-text-line entries).
   * Also resolves the speaker for the block.
   */
  let blockData = $derived.by(() => {
    const tab = store.activeTab;
    if (!tab || tab.selectedRowIndex === null) return null;

    const entries = tab.entries;
    const selectedIdx = tab.selectedRowIndex;
    const selectedEntry = entries[selectedIdx];
    if (!selectedEntry || selectedEntry.is_context_row || selectedEntry.label === "end-text-line") return null;
    if (selectedEntry.label !== "show-text" && selectedEntry.label !== "display-furigana") return null;

    // Find block boundaries (wait-for-input or start/end of array)
    let blockStart = 0;
    for (let i = selectedIdx - 1; i >= 0; i--) {
      if (entries[i].label === "wait-for-input") {
        blockStart = i + 1;
        break;
      }
    }

    let blockEnd = entries.length;
    for (let i = selectedIdx + 1; i < entries.length; i++) {
      if (entries[i].label === "wait-for-input") {
        blockEnd = i;
        break;
      }
    }

    // Collect visual lines — consecutive text entries are merged into one line,
    // only broken by end-text-line markers (since that's a single displayed line in-game).
    // display-furigana entries render as ruby annotations.
    const lines: { html: string; plain: string; plainLength: number; originalLength: number; isSelected: boolean }[] = [];
    let currentHtml = "";
    let currentPlainLen = 0;
    let currentOriginalLen = 0;
    let currentPlainText = "";
    let currentOriginalText = "";
    let currentSelected = false;

    function flushLine() {
      if (currentHtml) {
        lines.push({
          html: currentHtml,
          plain: currentPlainText,
          plainLength: stripQuoteBrackets(currentPlainLen, currentPlainText),
          originalLength: stripQuoteBrackets(currentOriginalLen, currentOriginalText),
          isSelected: currentSelected,
        });
        currentHtml = "";
        currentPlainLen = 0;
        currentOriginalLen = 0;
        currentPlainText = "";
        currentOriginalText = "";
        currentSelected = false;
      }
    }

    for (let i = blockStart; i < blockEnd; i++) {
      const e = entries[i];
      if (e.deleted) continue;

      // Entries flagged for omission contribute nothing to the rendered
      // preview — they'll be empty strings in the exported BIN.
      // Still mark selection if user has the row highlighted.
      if (e.omit_export === true) {
        if (i === selectedIdx) currentSelected = true;
        continue;
      }

      if (e.label === "show-text") {
        const displayText = e.translated_text || e.original_text;
        currentHtml += escapeHtml(displayText);
        currentPlainLen += displayText.length;
        currentPlainText += displayText;
        currentOriginalLen += e.original_text.length;
        currentOriginalText += e.original_text;
        if (i === selectedIdx) currentSelected = true;
      } else if (e.label === "display-furigana") {
        const displayText = e.translated_text || e.original_text;
        // Pad ruby with spaces so the annotation visually separates from
        // adjacent show-text on the preview line. Avoid double spaces if the
        // previous run already ends in whitespace.
        const needsLead = currentHtml.length > 0 && !/\s$/.test(currentHtml);
        currentHtml += (needsLead ? " " : "") + formatFuriganaHtml(displayText) + " ";
        const dispMatch = displayText.match(/^(.+?)\s*\[.+?\]$/);
        const origMatch = e.original_text.match(/^(.+?)\s*\[.+?\]$/);
        currentPlainLen += furiganaDisplayLength(displayText);
        currentPlainText += dispMatch ? dispMatch[1] : displayText;
        currentOriginalLen += furiganaDisplayLength(e.original_text);
        currentOriginalText += origMatch ? origMatch[1] : e.original_text;
        if (i === selectedIdx) currentSelected = true;
      } else if (e.label === "end-text-line") {
        flushLine();
      }
    }
    // Flush any remaining text after the last entry (no trailing end-text-line)
    flushLine();

    // Resolve speaker for this block — find the first show-text row's speaker
    let speaker: string | null = null;
    for (let i = blockStart; i < blockEnd; i++) {
      const e = entries[i];
      if (e.deleted) continue;
      if (e.label === "show-text") {
        const ref = tab.rowSpeakers[String(e.instruction_index)];
        if (ref) {
          if (ref.literal_name) {
            speaker = ref.literal_name;
          } else {
            const char = tab.characters.find((c) => c.pointer === ref.source_raw_data);
            if (char && char.name.length > 0) {
              speaker = char.name;
            }
          }
        }
        break;
      }
    }

    // Sum character counts for the block
    let originalTotal = 0;
    let translatedTotal = 0;
    for (const line of lines) {
      originalTotal += line.originalLength;
      translatedTotal += line.plainLength;
    }

    return { lines, speaker, originalTotal, translatedTotal };
  });

  /** Escape HTML special chars for safe {@html} usage */
  function escapeHtml(text: string): string {
    return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  /** Convert "kanji [furigana]" format to <ruby> HTML */
  function formatFuriganaHtml(text: string): string {
    const match = text.match(/^(.+?)\s*\[(.+?)\]$/);
    if (match) {
      return `<ruby>${escapeHtml(match[1])}<rp>(</rp><rt>${escapeHtml(match[2])}</rt><rp>)</rp></ruby>`;
    }
    return escapeHtml(text);
  }
</script>

{#if blockData && blockData.lines.length > 0}
  <div class="dialog-preview">
    <div class="preview-header">
      <span>Dialog Preview</span>
      {#if textLayout && store.gameAccuratePreview}
        <select
          class="slot-select"
          value={previewSlot}
          onchange={(e) => choosePreviewSlot(Number((e.currentTarget as HTMLSelectElement).value))}
          title="Which of the game's text layouts this file's text is drawn in (remembered per file prefix: {prefix || '—'}). 1 = main dialog box."
        >
          {#each textLayout.slots as s}
            <option value={s.slot}>{s.slot === 1 ? "Dialog box" : `Layout ${s.slot}`} · {s.width}×{s.height}</option>
          {/each}
        </select>
      {/if}
      {#if boxGeometry}
        <button
          class="proofread-btn"
          onclick={() => { store.gameAccuratePreview = !store.gameAccuratePreview; void import("../lib/settings").then((m) => m.persistAllSettingsDebounced()); }}
          title="Switch between the game's real text box (geometry read from its system script) and the simple preview"
        >
          {store.gameAccuratePreview ? "Simple view" : "Game view"}
        </button>
      {/if}
      <button
        class="proofread-btn"
        onclick={handleProofread}
        disabled={proofreading || blockEditableIndices.length === 0}
        title="Send the entire dialog block to the LLM for proofreading"
      >
        {proofreading ? "Proofreading..." : "Proofread"}
      </button>
    </div>
    {#if boxGeometry && store.gameAccuratePreview}
      <GamePreviewBox
        speaker={previewSlot === 1 ? blockData.speaker : null}
        geometry={boxGeometry}
        {wrapMode}
        lines={blockData.lines.map((l) => l.plain)}
        selectedLine={blockData.lines.findIndex((l) => l.isSelected)}
        maxLines={previewSlot === 1 ? store.maxLinesPerBox : 0}
        bottomLineCap={previewSlot === 1 ? store.bottomLineCharCap : 0}
        art={previewSlot === 1 && textLayout ? textLayout.art : []}
        atlases={textLayout?.atlases ?? []}
        onUseBottomLineCap={previewSlot === 1 ? useBottomLineCap : null}
      />
      {#if previewSlot === 1 && textLayout?.baseBody && textLayout.gameDir}
        <BoxProfilePanel base={textLayout.baseBody} gameDir={textLayout.gameDir} />
      {/if}
    {:else}
    <div class="dialog-box">
      {#if blockData.speaker}
        <div class="speaker-nameplate">{blockData.speaker}</div>
      {/if}
      <div class="dialog-text-area">
        {#each blockData.lines as line, li}
          {#if store.maxLinesPerBox > 0 && li === store.maxLinesPerBox && blockData.lines.length > store.maxLinesPerBox}
            <div
              class="box-full-divider"
              title="In-game text boxes show {store.maxLinesPerBox} lines. Lines below this fall off-screen until you paginate (Overflow Review \u2192 Paginate)."
            >
              \u25BD box full \u2014 {store.maxLinesPerBox}-line limit; lines below overflow off-screen
            </div>
          {/if}
          <div
            class="dialog-line"
            class:active={line.isSelected}
            class:over-line-cap={store.translatedLineCharCap > 0 && line.plainLength > store.translatedLineCharCap}
            class:overflow-line={store.maxLinesPerBox > 0 && li >= store.maxLinesPerBox}
          >
            {@html line.html || "\u00A0"}
          </div>
        {/each}
      </div>
    </div>
    {/if}
    <div class="char-totals">
      <span class="char-total-label">Original:</span>
      <span class="char-total-value">{blockData.originalTotal}</span>
      <span class="char-total-separator">|</span>
      <span class="char-total-label">Translated:</span>
      <span
        class="char-total-value"
        class:over-cap={store.translatedCharCap > 0 && blockData.translatedTotal > store.translatedCharCap}
      >{blockData.translatedTotal}{#if store.translatedCharCap > 0} / {store.translatedCharCap}{/if}</span>
    </div>

    {#if proofreadDiff}
      <div class="proofread-panel">
        <div class="proofread-header">
          <span class="proofread-title">Proofread suggestions</span>
          <div class="proofread-actions">
            <button class="accept-all-btn" onclick={applyProofreadAll}>Accept all</button>
            <button class="dismiss-btn" onclick={dismissProofread}>Dismiss</button>
          </div>
        </div>
        {#if proofreadDiff.notes}
          <div class="proofread-notes">{proofreadDiff.notes}</div>
        {/if}
        <div class="proofread-slots">
          {#each proofreadDiff.proposed as proposed, slotIdx}
            {#if proposed !== proofreadDiff.originals[slotIdx]}
              <div class="proofread-slot">
                <div class="slot-current">
                  <span class="slot-label">−</span>
                  <span class="slot-text">{proofreadDiff.originals[slotIdx] || "(empty)"}</span>
                </div>
                <div class="slot-proposed">
                  <span class="slot-label">+</span>
                  <span class="slot-text">{proposed}</span>
                </div>
                <button class="accept-one-btn" onclick={() => applyProofreadOne(slotIdx)}>Accept</button>
              </div>
            {/if}
          {/each}
          {#if proofreadDiff.proposed.every((p, i) => p === proofreadDiff!.originals[i])}
            <div class="no-changes">No changes suggested.</div>
          {/if}
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .dialog-preview {
    border-top: 1px solid #2a2a4a;
    background: #12122a;
    padding: 8px;
  }

  .preview-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 0.7rem;
    color: #666;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 18px;
    padding: 0 4px;
  }

  .slot-select {
    margin-left: auto;
    font-size: 11px;
    background: var(--bg-secondary, #222);
    color: inherit;
    border: 1px solid var(--border, #444);
    border-radius: 3px;
    padding: 1px 4px;
  }
  .proofread-btn {
    padding: 2px 10px;
    border: 1px solid #2a4a5a;
    border-radius: 3px;
    background: #1a2a3e;
    color: #8ab4f8;
    font-size: 0.7rem;
    text-transform: none;
    letter-spacing: 0;
    cursor: pointer;
  }

  .proofread-btn:hover:not(:disabled) {
    background: #2a3a5e;
    border-color: #4a6a8a;
  }

  .proofread-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .proofread-panel {
    margin-top: 10px;
    padding: 8px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #16213e;
  }

  .proofread-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }

  .proofread-title {
    font-size: 0.75rem;
    color: #8ab4f8;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .proofread-actions {
    display: flex;
    gap: 4px;
  }

  .accept-all-btn {
    padding: 2px 8px;
    border: 1px solid #2a5a3e;
    border-radius: 3px;
    background: #1a3a2e;
    color: #6fd886;
    font-size: 0.7rem;
    cursor: pointer;
  }

  .accept-all-btn:hover {
    background: #2a4a3e;
  }

  .dismiss-btn {
    padding: 2px 8px;
    border: 1px solid #5a2a2a;
    border-radius: 3px;
    background: #3a1a1a;
    color: #e06060;
    font-size: 0.7rem;
    cursor: pointer;
  }

  .dismiss-btn:hover {
    background: #4a2a2a;
  }

  .proofread-notes {
    font-size: 0.72rem;
    color: #888;
    font-style: italic;
    margin-bottom: 6px;
    padding: 4px 6px;
    background: #1a1a3e;
    border-radius: 3px;
  }

  .proofread-slots {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .proofread-slot {
    display: grid;
    grid-template-columns: 1fr auto;
    grid-template-rows: auto auto;
    gap: 2px 8px;
    padding: 4px 6px;
    background: #12122a;
    border-radius: 3px;
  }

  .slot-current,
  .slot-proposed {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    font-size: 0.78rem;
    line-height: 1.4;
  }

  .slot-current {
    grid-row: 1;
  }

  .slot-proposed {
    grid-row: 2;
  }

  .slot-label {
    font-family: monospace;
    font-weight: 600;
    flex-shrink: 0;
  }

  .slot-current .slot-label {
    color: #e06060;
  }

  .slot-proposed .slot-label {
    color: #6fd886;
  }

  .slot-current .slot-text {
    color: #b48080;
    text-decoration: line-through;
    text-decoration-color: #5a2a2a;
  }

  .slot-proposed .slot-text {
    color: #c0e0c0;
  }

  .accept-one-btn {
    grid-row: 1 / span 2;
    grid-column: 2;
    align-self: center;
    padding: 3px 10px;
    border: 1px solid #2a5a3e;
    border-radius: 3px;
    background: #1a3a2e;
    color: #6fd886;
    font-size: 0.7rem;
    cursor: pointer;
  }

  .accept-one-btn:hover {
    background: #2a4a3e;
  }

  .no-changes {
    padding: 6px;
    text-align: center;
    color: #555;
    font-size: 0.75rem;
    font-style: italic;
  }

  .dialog-box {
    background: rgba(0, 0, 0, 0.7);
    border: 2px solid #4a4a7a;
    border-radius: 6px;
    padding: 10px 14px;
    position: relative;
  }

  .speaker-nameplate {
    position: absolute;
    top: -12px;
    left: 12px;
    background: #2a3a6e;
    border: 1px solid #5a6aaa;
    border-radius: 3px;
    padding: 1px 10px;
    font-size: 0.8rem;
    color: #c8d8f8;
    font-weight: 600;
  }

  .dialog-text-area {
    padding-top: 4px;
    min-height: 40px;
  }

  .dialog-line {
    font-size: 0.85rem;
    color: #e0e0e0;
    line-height: 1.6;
    padding: 0 2px;
    border-left: 2px solid transparent;
    transition: border-color 0.15s;
  }

  .dialog-line.active {
    border-left-color: #6c9bd2;
    color: #ffffff;
  }

  .dialog-line.over-line-cap {
    color: #e0a030;
  }

  /* Lines past the per-box line limit: render off-screen in-game until paginated. */
  .dialog-line.overflow-line {
    opacity: 0.5;
    color: #d07a7a;
  }
  .box-full-divider {
    margin: 4px 0;
    border-top: 1px dashed #6a4a4a;
    padding-top: 3px;
    font-size: 0.68rem;
    color: #d07a7a;
    letter-spacing: 0.3px;
    user-select: none;
  }

  .dialog-line :global(ruby) {
    ruby-align: center;
  }

  .dialog-line :global(rt) {
    font-size: 0.55em;
    color: #aab4d8;
  }

  .dialog-line :global(rp) {
    display: none;
  }

  .char-totals {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
    padding: 0 4px;
    font-family: "Consolas", "Courier New", monospace;
    font-size: 0.75rem;
  }

  .char-total-label {
    color: #666;
  }

  .char-total-value {
    color: #6c9bd2;
  }

  .char-total-value.over-cap {
    color: #e05050;
  }

  .char-total-separator {
    color: #444;
  }

</style>
