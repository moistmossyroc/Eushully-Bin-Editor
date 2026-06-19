<script lang="ts">
  interface Props {
    onConfirm: (mode: "fill_missing" | "overwrite") => void;
    onCancel: () => void;
  }

  let { onConfirm, onCancel }: Props = $props();
  let selectedMode = $state<"fill_missing" | "overwrite">("fill_missing");

  function handleConfirm() {
    onConfirm(selectedMode);
  }
</script>

<div class="dialog-backdrop" onclick={onCancel} onkeydown={(e) => e.key === "Escape" && onCancel()} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog" onclick={(e) => e.stopPropagation()}>
    <h2>Import Translations</h2>
    <p class="subtitle">How should existing translations be handled?</p>

    <label class="mode-option" class:selected={selectedMode === "fill_missing"}>
      <input type="radio" bind:group={selectedMode} value="fill_missing" />
      <div class="mode-text">
        <div class="mode-title">Fill empty only <span class="recommended">(recommended)</span></div>
        <div class="mode-desc">Only set translations on entries that are currently empty. Existing translations are preserved.</div>
      </div>
    </label>

    <label class="mode-option" class:selected={selectedMode === "overwrite"}>
      <input type="radio" bind:group={selectedMode} value="overwrite" />
      <div class="mode-text">
        <div class="mode-title">Overwrite all</div>
        <div class="mode-desc">Replace every matched entry's translation and notes, even ones already filled in. Existing edits will be lost.</div>
      </div>
    </label>

    <div class="actions">
      <button class="btn cancel" onclick={onCancel}>Cancel</button>
      <button class="btn confirm" onclick={handleConfirm}>Import</button>
    </div>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    /* Above the modal layer (status/overflow at 1000) so it's reachable when
       launched from the Translation Status popup's per-folder Import button. */
    z-index: 2000;
  }

  .dialog {
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    padding: 20px 24px;
    min-width: 480px;
    max-width: 560px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }

  h2 {
    margin: 0 0 4px 0;
    color: #8ab4f8;
    font-size: 1.1rem;
  }

  .subtitle {
    margin: 0 0 16px 0;
    color: #a0a0c0;
    font-size: 0.85rem;
  }

  .mode-option {
    display: flex;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid #2a2a4a;
    border-radius: 4px;
    cursor: pointer;
    margin-bottom: 8px;
    transition: all 0.15s;
  }

  .mode-option:hover {
    border-color: #4a4a7a;
    background: #20204a;
  }

  .mode-option.selected {
    border-color: #6c9bd2;
    background: #2a3a6e;
  }

  .mode-option input[type="radio"] {
    margin-top: 4px;
  }

  .mode-title {
    color: #d0d0e0;
    font-weight: 600;
    font-size: 0.9rem;
    margin-bottom: 4px;
  }

  .recommended {
    color: #8ab4f8;
    font-weight: 400;
    font-size: 0.8rem;
  }

  .mode-desc {
    color: #a0a0c0;
    font-size: 0.8rem;
    line-height: 1.4;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }

  .btn {
    padding: 7px 16px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.85rem;
  }

  .btn:hover {
    background: #2a2a5e;
  }

  .btn.confirm {
    background: #2a3a6e;
    border-color: #6c9bd2;
    color: #8ab4f8;
    font-weight: 600;
  }

  .btn.confirm:hover {
    background: #3a4a8e;
  }
</style>
