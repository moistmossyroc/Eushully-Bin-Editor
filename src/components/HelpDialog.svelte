<script lang="ts">
  import { marked } from "marked";
  import { HELP_SECTIONS, DEFAULT_HELP_SECTION } from "../lib/help";
  import { store } from "../lib/stores.svelte";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  let query = $state("");

  // store.helpSection is the source of truth (set by deep-links + nav clicks).
  const selectedId = $derived(store.helpSection || DEFAULT_HELP_SECTION);
  const current = $derived(
    HELP_SECTIONS.find((s) => s.id === selectedId) ?? HELP_SECTIONS[0],
  );
  const html = $derived(marked.parse(current.markdown) as string);

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return HELP_SECTIONS;
    return HELP_SECTIONS.filter(
      (s) =>
        s.title.toLowerCase().includes(q) ||
        s.markdown.toLowerCase().includes(q),
    );
  });

  function select(id: string) {
    store.helpSection = id;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") onClose();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="backdrop" onclick={onClose}></div>
<div class="dialog" role="dialog" aria-modal="true" aria-label="Help Center">
  <header>
    <h3>Help Center</h3>
    <button class="close-btn" onclick={onClose} aria-label="Close">×</button>
  </header>

  <div class="content">
    <nav class="nav">
      <input class="search" placeholder="Search help…" bind:value={query} />
      <ul>
        {#each filtered as s (s.id)}
          <li>
            <button class:active={s.id === selectedId} onclick={() => select(s.id)}>
              {s.title}
            </button>
          </li>
        {/each}
        {#if filtered.length === 0}
          <li class="empty">No matches for “{query}”.</li>
        {/if}
      </ul>
    </nav>

    <article class="md">{@html html}</article>
  </div>
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
    width: 880px;
    max-width: 94vw;
    height: 80vh;
    max-height: 88vh;
    background: #15152a;
    border: 1px solid #3a3a5a;
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 16px;
    border-bottom: 1px solid #2a2a4a;
    flex: 0 0 auto;
  }
  header h3 {
    margin: 0;
    font-size: 1rem;
    color: #8ab4f8;
  }
  .close-btn {
    background: transparent;
    border: none;
    color: #8a8aaa;
    font-size: 1.4rem;
    line-height: 1;
    cursor: pointer;
  }
  .close-btn:hover {
    color: #fff;
  }
  .content {
    display: flex;
    min-height: 0;
    flex: 1 1 auto;
  }
  .nav {
    flex: 0 0 230px;
    border-right: 1px solid #2a2a4a;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .search {
    margin: 10px;
    padding: 6px 8px;
    background: #0f0f20;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    color: #d8d8f0;
    font-size: 0.85rem;
  }
  .search:focus {
    outline: none;
    border-color: #8ab4f8;
  }
  .nav ul {
    list-style: none;
    margin: 0;
    padding: 0 6px 10px;
    overflow-y: auto;
  }
  .nav li {
    margin: 2px 0;
  }
  .nav li button {
    width: 100%;
    text-align: left;
    background: transparent;
    border: none;
    color: #c0c0d8;
    padding: 7px 10px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.88rem;
  }
  .nav li button:hover {
    background: #20203a;
  }
  .nav li button.active {
    background: #2a3a5a;
    color: #fff;
  }
  .nav .empty {
    color: #777;
    padding: 8px 10px;
    font-size: 0.82rem;
  }
  .md {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 8px 28px 32px;
    color: #d8d8f0;
    line-height: 1.6;
    font-size: 0.92rem;
  }
  /* Markdown element styling */
  .md :global(h1) {
    color: #8ab4f8;
    font-size: 1.4rem;
    border-bottom: 1px solid #2a2a4a;
    padding-bottom: 8px;
    margin-top: 18px;
  }
  .md :global(h2) {
    color: #a9c7fb;
    font-size: 1.1rem;
    margin-top: 22px;
  }
  .md :global(h3) {
    color: #c0d4fb;
    font-size: 0.98rem;
    margin-top: 16px;
  }
  .md :global(a) {
    color: #8ab4f8;
  }
  .md :global(code) {
    background: #0f0f20;
    border: 1px solid #2a2a4a;
    border-radius: 3px;
    padding: 1px 5px;
    font-size: 0.85em;
  }
  .md :global(pre) {
    background: #0f0f20;
    border: 1px solid #2a2a4a;
    border-radius: 5px;
    padding: 12px 14px;
    overflow-x: auto;
  }
  .md :global(pre code) {
    background: none;
    border: none;
    padding: 0;
  }
  .md :global(blockquote) {
    border-left: 3px solid #4a5a8a;
    margin: 12px 0;
    padding: 4px 14px;
    color: #aab0d0;
    background: #1a1a30;
    border-radius: 0 4px 4px 0;
  }
  .md :global(ul),
  .md :global(ol) {
    padding-left: 22px;
  }
  .md :global(li) {
    margin: 4px 0;
  }
  .md :global(strong) {
    color: #f0f0ff;
  }
  .md :global(table) {
    border-collapse: collapse;
    margin: 12px 0;
  }
  .md :global(th),
  .md :global(td) {
    border: 1px solid #2a2a4a;
    padding: 5px 10px;
  }
</style>
