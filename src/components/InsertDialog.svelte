<script lang="ts">
  import { insertInstructionVisually } from "../lib/insertions";
  import type { SerializedArg } from "../lib/types";
  import { store } from "../lib/stores.svelte";

  interface Props {
    afterIndex: number;
    onClose: () => void;
  }

  let { afterIndex, onClose }: Props = $props();

  type InsertType = "show-text" | "display-furigana";

  let insertType = $state<InsertType>("show-text");
  /** Selected speaker: "narrator" for no speaker, or the character pointer as string */
  let selectedSpeaker = $state("narrator");
  let showTextLine = $state("");
  let furiganaText = $state("");
  let furiganaReading = $state("");

  /** Build the speaker dropdown options from the characters list */
  let speakerOptions = $derived.by(() => {
    const options: { value: string; label: string }[] = [
      { value: "narrator", label: "Narrator (no speaker)" },
    ];
    for (const char of store.activeTab!.characters) {
      const hex = "0x" + char.pointer.toString(16).toUpperCase();
      const label = char.name.length > 0
        ? `${hex} — ${char.name}`
        : hex;
      options.push({ value: String(char.pointer), label });
    }
    return options;
  });

  function handleInsert() {
    if (insertType === "show-text") {
      const isNarrator = selectedSpeaker === "narrator";
      // show-text arg[0]: display mode (0 = narrator, 2 = speaker nameplate)
      // show-text arg[1]: dialogue string (arg_type 2 = inline string)
      const args: SerializedArg[] = [
        { arg_type: 0, raw_value: isNarrator ? 0 : 2 },
        { arg_type: 2, raw_value: 0, string_value: showTextLine },
      ];

      // Build display text: "Speaker | line" or just "line" for narrator
      let speakerLabel = "";
      if (!isNarrator) {
        const char = store.activeTab!.characters.find(
          (c) => c.pointer === Number(selectedSpeaker)
        );
        speakerLabel = char && char.name.length > 0
          ? char.name
          : "0x" + Number(selectedSpeaker).toString(16).toUpperCase();
      }
      const originalText = speakerLabel
        ? `${speakerLabel} | ${showTextLine}`
        : showTextLine;

      const syntheticIndex = insertInstructionVisually(
        afterIndex, "show-text", 0x6e,
        args,
        false, originalText, showTextLine,
      );

      // Register the speaker for the inserted row so the badge displays
      if (!isNarrator) {
        store.activeTab!.rowSpeakers = {
          ...store.activeTab!.rowSpeakers,
          [String(syntheticIndex)]: {
            source_type: 0,
            source_raw_data: Number(selectedSpeaker),
            literal_name: null,
          },
        };
      }
    } else {
      const originalText = `${furiganaText} [${furiganaReading}]`;
      insertInstructionVisually(
        afterIndex, "display-furigana", 0x196,
        [
          { arg_type: 0, raw_value: 0 },
          { arg_type: 2, raw_value: 0, string_value: furiganaText },
          { arg_type: 2, raw_value: 0, string_value: furiganaReading },
        ],
        false, originalText,
      );
    }
    onClose();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
    if (e.key === "Enter" && e.ctrlKey) handleInsert();
  }

  let backdropMouseDown = false;
  function handleBackdropMouseDown(e: MouseEvent) {
    backdropMouseDown = (e.target as HTMLElement).classList.contains("dialog-backdrop");
  }
  function handleBackdropMouseUp(e: MouseEvent) {
    if (backdropMouseDown && (e.target as HTMLElement).classList.contains("dialog-backdrop")) {
      onClose();
    }
    backdropMouseDown = false;
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="dialog-backdrop" onmousedown={handleBackdropMouseDown} onmouseup={handleBackdropMouseUp}>
  <div class="dialog">
    <div class="dialog-header">
      <h2>Insert Instruction</h2>
      <button class="close-btn" onclick={onClose}>&times;</button>
    </div>

    <div class="dialog-body">
      <p class="insert-info">
        Insert after instruction <strong>#{afterIndex}</strong>
      </p>

      <label class="field">
        <span>Type</span>
        <select bind:value={insertType}>
          <option value="show-text">show-text (new dialogue)</option>
          <option value="display-furigana">display-furigana</option>
        </select>
      </label>

      {#if insertType === "show-text"}
        <label class="field">
          <span>Speaker</span>
          <select bind:value={selectedSpeaker}>
            {#each speakerOptions as opt}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </label>
        <label class="field">
          <span>Dialogue Line</span>
          <textarea
            bind:value={showTextLine}
            placeholder="Enter text..."
            rows="3"
          ></textarea>
        </label>
      {/if}

      {#if insertType === "display-furigana"}
        <label class="field">
          <span>Base Text</span>
          <input
            type="text"
            bind:value={furiganaText}
            placeholder="Kanji/text to annotate"
          />
        </label>
        <label class="field">
          <span>Reading</span>
          <input
            type="text"
            bind:value={furiganaReading}
            placeholder="Furigana reading"
          />
        </label>
      {/if}
    </div>

    <div class="dialog-footer">
      <button class="cancel-btn" onclick={onClose}>Cancel</button>
      <button class="insert-btn" onclick={handleInsert}>
        Insert
      </button>
    </div>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .dialog {
    width: 440px;
    background: #1a1a2e;
    border: 1px solid #3a3a5a;
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid #2a2a4a;
  }

  .dialog-header h2 {
    margin: 0;
    font-size: 1rem;
    color: #e0e0e0;
  }

  .close-btn {
    background: none;
    border: none;
    color: #888;
    font-size: 1.4rem;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }

  .close-btn:hover {
    color: #e0e0e0;
  }

  .dialog-body {
    padding: 16px;
  }

  .insert-info {
    margin: 0 0 12px 0;
    font-size: 0.85rem;
    color: #888;
  }

  .insert-info strong {
    color: #8ab4f8;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 12px;
  }

  .field span {
    font-size: 0.8rem;
    color: #888;
  }

  .field select,
  .field input,
  .field textarea {
    padding: 6px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #16213e;
    color: #c0c0e0;
    font-size: 0.85rem;
    outline: none;
    font-family: inherit;
  }

  .field select:focus,
  .field input:focus,
  .field textarea:focus {
    border-color: #6c9bd2;
  }

  .field textarea {
    resize: vertical;
  }

  .dialog-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 16px;
    border-top: 1px solid #2a2a4a;
  }

  .cancel-btn {
    padding: 6px 14px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #888;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .cancel-btn:hover {
    color: #c0c0e0;
    border-color: #5a5a8a;
  }

  .insert-btn {
    padding: 6px 14px;
    border: 1px solid #2a5a3e;
    border-radius: 4px;
    background: #1a3a2e;
    color: #6fd886;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .insert-btn:hover {
    background: #2a4a3e;
  }
</style>
