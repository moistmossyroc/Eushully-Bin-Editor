<script lang="ts">
  /**
   * Filterable combobox for picking a model identifier. Styled to match the
   * editor's dark theme — replaces native `<input list=datalist>` (which
   * delegates the open list to OS rendering and breaks visual consistency).
   *
   * Behavior:
   * - Bound `value` is two-way; typing updates it character-by-character.
   * - Suggestions filter live by case-insensitive substring of `value`.
   * - Click input or ▾ toggles the dropdown. Clicking outside closes it.
   * - Keyboard: ↑/↓ to highlight, Enter to select, Esc to close.
   * - Empty suggestion list = field still works as plain text input.
   */
  interface Props {
    value: string;
    onValueChange: (v: string) => void;
    suggestions: string[];
    placeholder?: string;
    title?: string;
    disabled?: boolean;
  }

  let { value = $bindable(), onValueChange, suggestions, placeholder = "", title = "", disabled = false }: Props = $props();

  let open = $state(false);
  let highlight = $state(-1);
  let containerEl = $state<HTMLDivElement | null>(null);
  let inputEl = $state<HTMLInputElement | null>(null);

  // Filter suggestions by case-insensitive substring of current value.
  let filtered = $derived.by(() => {
    const q = value.trim().toLowerCase();
    if (!q) return suggestions;
    return suggestions.filter(s => s.toLowerCase().includes(q));
  });

  function handleInput(e: Event) {
    const v = (e.target as HTMLInputElement).value;
    value = v;
    onValueChange(v);
    open = filtered.length > 0;
    highlight = -1;
  }

  function pick(item: string) {
    value = item;
    onValueChange(item);
    open = false;
    highlight = -1;
    inputEl?.focus();
  }

  function handleKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (!open && filtered.length > 0) { open = true; highlight = 0; return; }
      highlight = Math.min(highlight + 1, filtered.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      highlight = Math.max(highlight - 1, 0);
    } else if (e.key === "Enter") {
      if (open && highlight >= 0 && highlight < filtered.length) {
        e.preventDefault();
        pick(filtered[highlight]);
      }
    } else if (e.key === "Escape") {
      open = false;
      highlight = -1;
    }
  }

  function toggleDropdown() {
    if (disabled) return;
    open = !open;
    if (open) {
      highlight = -1;
      inputEl?.focus();
    }
  }

  // Close on outside click.
  $effect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!containerEl) return;
      if (!containerEl.contains(e.target as Node)) {
        open = false;
        highlight = -1;
      }
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  });
</script>

<div class="combobox" bind:this={containerEl}>
  <input
    bind:this={inputEl}
    type="text"
    {value}
    {placeholder}
    {title}
    {disabled}
    oninput={handleInput}
    onfocus={() => { if (filtered.length > 0) open = true; }}
    onkeydown={handleKey}
  />
  <button
    type="button"
    class="caret"
    onclick={toggleDropdown}
    {disabled}
    aria-label="Toggle suggestions"
    tabindex="-1"
  >▾</button>

  {#if open && filtered.length > 0}
    <ul class="dropdown" role="listbox">
      {#each filtered as item, i}
        <li
          role="option"
          aria-selected={i === highlight}
          class:highlighted={i === highlight}
          class:active={item === value}
          onmousedown={(e) => { e.preventDefault(); pick(item); }}
          onmouseenter={() => (highlight = i)}
        >
          {item}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .combobox {
    position: relative;
    display: inline-flex;
    align-items: stretch;
    flex: 1;
    min-width: 0;
  }

  input {
    flex: 1;
    min-width: 0;
    padding: 4px 28px 4px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #c0c0e0;
    font-size: 0.85rem;
    outline: none;
    font-family: inherit;
  }
  input:focus {
    border-color: #5a7aff;
  }
  input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .caret {
    position: absolute;
    right: 4px;
    top: 50%;
    transform: translateY(-50%);
    width: 20px;
    height: 20px;
    padding: 0;
    border: none;
    background: transparent;
    color: #8a8aaa;
    cursor: pointer;
    font-size: 0.7rem;
    line-height: 1;
  }
  .caret:hover { color: #c0c0e0; }
  .caret:disabled { cursor: not-allowed; opacity: 0.4; }

  .dropdown {
    position: absolute;
    top: calc(100% + 2px);
    left: 0;
    right: 0;
    z-index: 100;
    max-height: 240px;
    overflow-y: auto;
    margin: 0;
    padding: 4px 0;
    list-style: none;
    background: #1a1a3e;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
  }

  .dropdown li {
    padding: 6px 10px;
    color: #c0c0e0;
    cursor: pointer;
    font-size: 0.85rem;
    line-height: 1.2;
  }
  .dropdown li.highlighted { background: #2a2a5e; }
  .dropdown li.active { color: #8aa0ff; font-weight: 600; }
  .dropdown li:hover { background: #2a2a5e; }

  /* Scrollbar styling for the dropdown — matches the rest of the app */
</style>
