<script lang="ts">
  import { getApiKey, setApiKey, loadSettings, setProviderConfig, getProviderConfig, fetchOpenAiCompatModels } from "../lib/ipc";
  import { persistAllSettings } from "../lib/settings";
  import { store } from "../lib/stores.svelte";
  import { fetchAvailableModels } from "../lib/ipc";
  import { confirmDialog } from "../lib/confirm";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import ModelCombobox from "./ModelCombobox.svelte";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  let apiKey = $state("");
  let showKey = $state(false);
  let loading = $state(true);
  let fetchingModels = $state(false);

  const PROVIDER_PRESETS: { label: string; endpoint: string }[] = [
    { label: "OpenRouter", endpoint: "https://openrouter.ai/api" },
    { label: "OpenAI", endpoint: "https://api.openai.com" },
    { label: "Mistral", endpoint: "https://api.mistral.ai" },
    { label: "Groq", endpoint: "https://api.groq.com/openai" },
    { label: "Ollama (local)", endpoint: "http://localhost:11434" },
    { label: "LM Studio (local)", endpoint: "http://localhost:1234" },
  ];

  onMount(async () => {
    try {
      const storedKey = await getApiKey();
      if (storedKey) {
        apiKey = storedKey;
      }
    } catch {
      // No key stored yet
    }

    // NOTE: do not overwrite store.apiProvider / customEndpoint here. App.svelte
    // already loaded them from settings.json at boot and pushed to Rust state.
    // Re-reading from Rust here was racing against the auto-save and could blank
    // the dropdown on dialog reopen. Trust the store as the source of truth.

    loading = false;
  });

  let keySaveState = $state<"idle" | "saved" | "error">("idle");
  let keySaveTimeout: ReturnType<typeof setTimeout> | null = null;

  async function handleSaveKey() {
    try {
      await setApiKey(apiKey.trim());
      // For OpenAI-compatible providers, an empty key is valid (Ollama, LM Studio)
      const isOpenAi = store.apiProvider === "openai-compatible";
      store.apiKeyConfigured = apiKey.trim().length > 0 || isOpenAi;
      store.statusMessage = store.apiKeyConfigured
        ? "API key saved"
        : "API key cleared";

      // Inline confirmation — flip the Save button to "Saved ✓" for 2s.
      keySaveState = "saved";
      if (keySaveTimeout) clearTimeout(keySaveTimeout);
      keySaveTimeout = setTimeout(() => { keySaveState = "idle"; }, 2000);

      // Auto-fetch models when a new key is saved
      if (store.apiKeyConfigured) {
        fetchAvailableModels()
          .then((models) => {
            if (models.length > 0) {
              store.availableModels = models;
              store.statusMessage = `API key saved — found ${models.length} models`;
            }
          })
          .catch(() => { /* Non-critical */ });
      }
    } catch (err) {
      store.statusMessage = `Error saving API key: ${err}`;
      keySaveState = "error";
      if (keySaveTimeout) clearTimeout(keySaveTimeout);
      keySaveTimeout = setTimeout(() => { keySaveState = "idle"; }, 2500);
    }

    // After saving a key for an OpenAI-compatible provider, auto-fetch the
    // model list from /v1/models so the datalist populates without needing
    // an extra user click. Failure is silent — datalist stays empty, the
    // input still accepts manual typing.
    if (store.apiProvider === "openai-compatible" && store.customEndpoint) {
      refreshOpenAiCompatModels();
    }
  }

  /** Normalize an endpoint for use as the cache key — strip trailing slash. */
  function normalizeEndpoint(ep: string): string {
    return ep.trim().replace(/\/+$/, "");
  }

  /** Fetch the model list from the current OpenAI-compatible endpoint and
   *  populate the cache + placeholder state. Safe to call repeatedly — it
   *  no-ops when there's nothing to fetch. */
  async function refreshOpenAiCompatModels() {
    const ep = normalizeEndpoint(store.customEndpoint);
    if (!ep) return;
    // For local providers (Ollama, LM Studio) an empty key is OK. For remote
    // providers a key is required — skip the fetch and let placeholder hint
    // the user to save one first.
    const isLocal = ep.startsWith("http://localhost") || ep.startsWith("http://127.0.0.1");
    if (!isLocal && !apiKey.trim()) {
      store.openAiCompatFetchState = "idle";
      return;
    }

    store.openAiCompatFetchState = "loading";
    store.openAiCompatFetchError = "";
    try {
      const ids = await fetchOpenAiCompatModels(ep, apiKey.trim());
      store.openAiCompatModelCache = { ...store.openAiCompatModelCache, [ep]: ids };
      store.openAiCompatFetchState = ids.length > 0 ? "loaded" : "failed";
      if (ids.length === 0) {
        store.openAiCompatFetchError = "Provider returned no models";
      }
    } catch (err) {
      store.openAiCompatFetchState = "failed";
      store.openAiCompatFetchError = String(err);
    }
  }

  /** Current model suggestions for the active endpoint. Empty if not fetched
   *  yet or fetch failed — the input still works as plain text in that case. */
  let openAiCompatSuggestions = $derived(
    store.openAiCompatModelCache[normalizeEndpoint(store.customEndpoint)] ?? []
  );

  /** State-aware placeholder text for the OpenAI-compat model input. */
  let modelInputPlaceholder = $derived.by(() => {
    const ep = normalizeEndpoint(store.customEndpoint);
    const isLocal = ep.startsWith("http://localhost") || ep.startsWith("http://127.0.0.1");
    if (store.openAiCompatFetchState === "loading") return "Loading available models...";
    if (store.openAiCompatFetchState === "loaded" && openAiCompatSuggestions.length > 0) {
      return "Type to filter or click ▾ to pick a model";
    }
    if (store.openAiCompatFetchState === "failed") {
      return "Type a model name (e.g. mistral-large-latest)";
    }
    if (!apiKey.trim() && !isLocal) {
      return "Save API key to load model suggestions — or type a model name manually";
    }
    return "e.g. mistral-large-latest, gpt-4o, llama3";
  });

  async function handleProviderChange() {
    try {
      await setProviderConfig(
        store.apiProvider,
        store.apiProvider === "openai-compatible" ? (store.customEndpoint || null) : null,
      );
      // Also persist to settings.json so it survives app restart (Rust state is in-memory).
      await persistSettings();
      store.statusMessage = "Provider config saved";

      // Endpoint changed — invalidate any cached state-aware UI and try to
      // fetch the model list. Silent failure is fine; placeholder hint covers it.
      if (store.apiProvider === "openai-compatible") {
        store.openAiCompatFetchState = "idle";
        store.openAiCompatFetchError = "";
        refreshOpenAiCompatModels();
      }
    } catch (err) {
      store.statusMessage = `Error saving provider config: ${err}`;
    }
  }

  let endpointSaveTimer: ReturnType<typeof setTimeout> | null = null;
  function handleEndpointInput() {
    if (endpointSaveTimer) clearTimeout(endpointSaveTimer);
    endpointSaveTimer = setTimeout(() => {
      handleProviderChange();
      endpointSaveTimer = null;
    }, 300);
  }

  async function persistSettings() {
    await persistAllSettings();
  }

  /** Pick a UI translation table file (`fanpatch_ui.txt`) to bundle inside the
   *  exported `.fanpatch`. Stored as an absolute path on the store. */
  async function handleBrowseUiConfig() {
    const selected = await open({
      multiple: false,
      filters: [{ name: "Text Files", extensions: ["txt"] }],
    });
    if (typeof selected === "string") {
      store.fanpatchUiConfigPath = selected;
      await persistSettings();
    }
  }

  async function handleRefreshModels() {
    if (!store.apiKeyConfigured) {
      store.statusMessage = "Set an API key first to fetch models";
      return;
    }
    fetchingModels = true;
    try {
      const models = await fetchAvailableModels();
      if (models.length > 0) {
        store.availableModels = models;
        store.statusMessage = `Found ${models.length} available models`;
      } else {
        store.statusMessage = "No models returned — using defaults";
      }
    } catch (err) {
      store.statusMessage = `Failed to fetch models: ${err}`;
    }
    fetchingModels = false;
  }

  function handleApplyModelToAll() {
    const tab = store.activeTab;
    if (!tab) return;
    // Copy the active provider's model to every other tab's matching field.
    const isOpenAi = store.apiProvider === "openai-compatible";
    const model = isOpenAi ? tab.openAiCompatModel : tab.anthropicModel;
    if (!model) return;
    let count = 0;
    for (const t of store.tabs) {
      if (isOpenAi) {
        if (t.openAiCompatModel !== model) { t.openAiCompatModel = model; count++; }
      } else {
        if (t.anthropicModel !== model) { t.anthropicModel = model; count++; }
      }
    }
    persistSettings();
    store.statusMessage = count > 0
      ? `Applied model to ${count} other tab${count !== 1 ? "s" : ""}`
      : "All tabs already use this model";
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      onClose();
    }
  }

  /** Tab navigation for the Settings dialog. Sections used to all render in
   *  one long scroll; with provider + translation + display options each
   *  growing, tabs scan faster and reduce scroll fatigue. */
  type SettingsTab = "provider" | "editor" | "translation";
  let activeTab = $state<SettingsTab>("provider");
  const tabs: { id: SettingsTab; label: string }[] = [
    { id: "provider", label: "LLM Provider" },
    { id: "editor", label: "Editor" },
    { id: "translation", label: "Live API" },
  ];

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
      <h2>Settings</h2>
      <button class="close-btn" onclick={onClose}>&times;</button>
    </div>

    <nav class="settings-tabs">
      {#each tabs as t}
        <button
          class="tab-btn"
          class:active={activeTab === t.id}
          onclick={() => (activeTab = t.id)}
        >{t.label}</button>
      {/each}
    </nav>

    <div class="dialog-body">
      {#if activeTab === "provider"}
      <section class="setting-group">
        <h3>LLM Provider</h3>

        <label class="setting-field">
          <span>Provider</span>
          <select
            bind:value={store.apiProvider}
            onchange={handleProviderChange}
          >
            <option value="anthropic">Anthropic (Claude API)</option>
            <option value="openai-compatible">OpenAI-Compatible (OpenRouter, OpenAI, Ollama, etc.)</option>
          </select>
        </label>

        {#if store.apiProvider === "openai-compatible"}
          <label class="setting-field">
            <span>Endpoint URL</span>
            <div class="endpoint-row">
              <input
                type="text"
                bind:value={store.customEndpoint}
                oninput={handleEndpointInput}
                placeholder="https://openrouter.ai/api"
              />
            </div>
          </label>

          <div class="preset-row">
            {#each PROVIDER_PRESETS as preset}
              <button
                class="preset-btn"
                class:active={store.customEndpoint === preset.endpoint}
                onclick={() => { store.customEndpoint = preset.endpoint; handleProviderChange(); }}
              >
                {preset.label}
              </button>
            {/each}
          </div>

          {#if store.activeTab}
            <label class="setting-field">
              <span>Model name</span>
              <div class="model-row">
                <ModelCombobox
                  bind:value={store.activeTab.openAiCompatModel}
                  onValueChange={persistSettings}
                  suggestions={openAiCompatSuggestions}
                  placeholder={modelInputPlaceholder}
                  title="Suggestions auto-populate once endpoint + API key are set. You can also type any model name your provider accepts."
                />
                <button
                  class="refresh-btn"
                  onclick={refreshOpenAiCompatModels}
                  disabled={store.openAiCompatFetchState === "loading"}
                  title="Refetch model list from /v1/models"
                >
                  {store.openAiCompatFetchState === "loading" ? "..." : "Refresh"}
                </button>
                {#if store.tabs.length > 1}
                  <button
                    class="apply-all-btn"
                    onclick={handleApplyModelToAll}
                    title="Set this model on all open tabs"
                  >
                    All tabs
                  </button>
                {/if}
              </div>
            </label>

            <p class="setting-hint">
              {#if store.openAiCompatFetchState === "loaded"}
                {openAiCompatSuggestions.length} model{openAiCompatSuggestions.length === 1 ? "" : "s"} loaded from this provider. Click the field to browse, or type to filter.
              {:else if store.openAiCompatFetchState === "failed"}
                Couldn't fetch model list ({store.openAiCompatFetchError}). Type the model name manually.
              {:else if store.openAiCompatFetchState === "loading"}
                Querying {store.customEndpoint}/v1/models …
              {:else}
                Save an API key (or set a local endpoint) to auto-load suggestions. Manual entry always works.
              {/if}
            </p>
          {/if}
        {:else}
          {#if store.activeTab}
            <label class="setting-field">
              <span>Model</span>
              <div class="model-row">
                <select bind:value={store.activeTab.anthropicModel} disabled={store.availableModels.length === 0}>
                  {#if store.availableModels.length === 0}
                    <option value="">Add API key + Refresh to load models</option>
                  {/if}
                  {#each store.availableModels as model}
                    <option value={model.id}>{model.label}</option>
                  {/each}
                </select>
                <button
                  class="refresh-btn"
                  onclick={handleRefreshModels}
                  disabled={fetchingModels || !store.apiKeyConfigured}
                  title="Fetch latest models from Anthropic"
                >
                  {fetchingModels ? "..." : "Refresh"}
                </button>
                {#if store.tabs.length > 1}
                  <button
                    class="apply-all-btn"
                    onclick={handleApplyModelToAll}
                    title="Set this model on all open tabs"
                  >
                    All tabs
                  </button>
                {/if}
              </div>
            </label>
          {/if}

          <p class="setting-hint">
            Click Refresh to fetch the latest available models from the Anthropic API.
          </p>
        {/if}

        <label class="setting-field">
          <span>API Key</span>
          <div class="key-input-row">
            <input
              type={showKey ? "text" : "password"}
              bind:value={apiKey}
              placeholder={loading ? "Loading..." : store.apiProvider === "anthropic" ? "sk-ant-..." : "API key (leave empty for Ollama)"}
              disabled={loading}
            />
            <button
              class="toggle-btn"
              onclick={() => (showKey = !showKey)}
              title={showKey ? "Hide" : "Show"}
            >
              {showKey ? "Hide" : "Show"}
            </button>
          </div>
        </label>

        <div class="key-actions">
          <button
            class="save-key-btn"
            class:saved={keySaveState === "saved"}
            class:error={keySaveState === "error"}
            onclick={handleSaveKey}
          >
            {keySaveState === "saved" ? "Saved ✓" : keySaveState === "error" ? "Save failed" : "Save API Key"}
          </button>
          {#if store.apiKeyConfigured}
            <span class="key-status configured">Key configured</span>
          {:else}
            <span class="key-status not-configured">No key set</span>
          {/if}
        </div>

        <p class="setting-hint">
          Your API key is stored in Windows Credential Manager.
          {#if store.apiProvider === "openai-compatible"}
            For local providers (Ollama, LM Studio), you can leave the key empty.
          {/if}
        </p>
      </section>
      {:else if activeTab === "editor"}
      <section class="setting-group">
        <h3>Editor</h3>

        <label class="setting-field checkbox-field">
          <input
            type="checkbox"
            bind:checked={store.autoRestoreSession}
            onchange={persistSettings}
          />
          <span>Auto-restore last session on launch</span>
        </label>

        <p class="setting-hint">
          When enabled, the editor reopens the tabs that were open the last
          time you closed it. Missing files (moved/deleted) are skipped
          silently. Adds startup time proportional to BIN parse cost.
        </p>

        <label class="setting-field checkbox-field">
          <input
            type="checkbox"
            bind:checked={store.confirmTabClose}
            onchange={persistSettings}
          />
          <span>Confirm before closing a tab</span>
        </label>

        <p class="setting-hint">
          When enabled, closing any tab asks for confirmation. When disabled,
          tabs close immediately — except tabs with unsaved changes, which
          always prompt so edits aren't lost.
        </p>

        <label class="setting-field">
          <span>Block character cap (0 = disabled)</span>
          <input
            type="number"
            min="0"
            bind:value={store.translatedCharCap}
            onchange={persistSettings}
            placeholder="0"
          />
        </label>

        <p class="setting-hint">
          Total characters across all lines between end-text-line markers.
          Turns red when exceeded.
        </p>

        <label class="setting-field">
          <span>Per-line character cap (0 = disabled)</span>
          <input
            type="number"
            min="0"
            bind:value={store.translatedLineCharCap}
            onchange={persistSettings}
            placeholder="0"
          />
        </label>

        <p class="setting-hint">
          Character limit for each individual translated text cell.
          Turns yellow/orange when exceeded.
        </p>

        <label class="setting-field">
          <span>Max lines per text box</span>
          <input
            type="number"
            min="1"
            max="10"
            bind:value={store.maxLinesPerBox}
            onchange={persistSettings}
            placeholder="3"
          />
        </label>

        <p class="setting-hint">
          Maximum display lines the game renders in one text box. Auto-split
          starts a new box (via bridge) instead of overflowing into a hidden
          fourth line. Default 3 for most Eushully games.
        </p>

        <label class="setting-field">
          <span>Special-scene file prefix(es)</span>
          <input
            type="text"
            bind:value={store.spScenePrefixes}
            onchange={persistSettings}
            placeholder="SP"
          />
        </label>

        <p class="setting-hint">
          Comma-separated filename prefix(es) that group BINs under the
          <strong>Special scenes (SP)</strong> bucket in the game-folder picker.
          Default <code>SP</code>. Story scenes (<code>SC…</code>) and UI/system
          files are detected automatically.
        </p>

        <label class="setting-field">
          <span>Remembered workspaces: {Object.keys(store.workspaceFolders).length}</span>
          <div class="ui-config-row">
            <button
              class="refresh-btn"
              disabled={Object.keys(store.workspaceFolders).length === 0}
              onclick={() => { store.workspaceFolders = {}; persistSettings(); }}
              title="Forget all saved game-folder → workspace mappings"
            >Clear remembered workspace folders</button>
          </div>
        </label>

        <p class="setting-hint">
          When you open an archived game from a read-only folder (e.g. a game in
          Program Files), the editor remembers the writable workspace you chose
          and reuses it next time — skipping re-extraction of files already there.
          Clear this if a mapping points at the wrong or a stale folder.
        </p>

        <label class="setting-field checkbox-field">
          <input
            type="checkbox"
            bind:checked={store.autoSplitEnabled}
            onchange={async (e) => {
              if ((e.target as HTMLInputElement).checked) {
                const ok = await confirmDialog({
                  title: "Enable auto-split?",
                  confirmLabel: "Enable",
                  message:
                    "Auto-split inserts new dialog-box rows (shown as #-1, #-2, …) when a translation exceeds the character cap. " +
                    "This can be useful for keeping in-game text within bounds, but it can also produce surprising row layouts " +
                    "and occasionally interact badly with block-mode live translation — especially on non-Anthropic providers " +
                    "(Mistral, OpenAI, etc.).\n\n" +
                    "Recommended only for fine-tuned passes after the initial translation completes. Leave OFF for the first pass.",
                });
                if (!ok) {
                  store.autoSplitEnabled = false;
                  return;
                }
              }
              persistSettings();
            }}
          />
          <span>Auto-split dialog boxes on overflow</span>
        </label>

        <p class="setting-hint">
          <strong>Recommended: OFF for live translation.</strong> When enabled,
          translations exceeding the char cap are split into multiple dialog
          boxes by inserting new rows (e.g. <code>#-1</code>, <code>#-2</code>).
          Can produce unexpected row layouts mid-translation and is known to
          interact poorly with block-mode on non-Anthropic providers. Best used
          as a fine-tuning pass after translation is complete.
        </p>

        <label class="setting-field checkbox-field">
          <input
            type="checkbox"
            bind:checked={store.autoSplitOnImport}
            onchange={persistSettings}
          />
          <span>Auto-split on import</span>
        </label>

        <p class="setting-hint">
          When importing translations (single or <strong>Import all</strong>),
          automatically run the bulk wrap/box-split on each affected BIN so
          overflowing rows are fixed without opening the Overflow dialog per file.
          Requires a per-line cap. Rows that can't be split (no bridge in that
          BIN) are still flagged for manual shortening. Default OFF.
        </p>
      </section>

      <section class="setting-group">
        <h3>Fanpatch title bar</h3>

        <label class="setting-field">
          <span>Title</span>
          <input
            type="text"
            bind:value={store.fanpatchTitle}
            onchange={persistSettings}
            placeholder="Battle Goddess ZERO"
          />
        </label>

        <label class="setting-field">
          <span>Label</span>
          <input
            type="text"
            bind:value={store.fanpatchLabel}
            onchange={persistSettings}
            placeholder="Fanpatch v1.0"
          />
        </label>

        <p class="setting-hint">
          Bundled inside the exported <code>.fanpatch</code> as
          <code>fanpatch_title.txt</code>. The loader shows
          <em>&lt;title&gt; [&lt;label&gt; - English]</em> in the window title
          bar. Leave both blank to use the loader's defaults.
        </p>

        <label class="setting-field">
          <span>UI translation file</span>
          <div class="ui-config-row">
            <input
              type="text"
              readonly
              value={store.fanpatchUiConfigPath}
              placeholder="(none — optional)"
              title={store.fanpatchUiConfigPath}
            />
            <button class="refresh-btn" onclick={handleBrowseUiConfig}>Browse…</button>
            {#if store.fanpatchUiConfigPath}
              <button
                class="refresh-btn"
                onclick={() => { store.fanpatchUiConfigPath = ""; persistSettings(); }}
                title="Don't bundle a UI translation file"
              >Clear</button>
            {/if}
          </div>
        </label>

        <p class="setting-hint">
          Optional. Bundled into the <code>.fanpatch</code> as
          <code>fanpatch_ui.txt</code> — the loader swaps menu/system UI strings
          with the in-game language toggle. Leave empty to skip.
        </p>
      </section>
      {:else if activeTab === "translation"}
      <section class="setting-group">
        <h3>Live API Translation</h3>

        <div class="scope-note">
          <strong>Scope:</strong> these settings only affect the <em>in-editor</em>
          Translate button (live API requests to Claude / OpenAI-compatible providers).
          <br />
          <strong>CC Translate is not affected</strong> — its behavior is baked into the
          generated <code>PROMPT.md</code> + <code>translate.py</code> at export time.
          Adjust CC Translate behavior via the export dialog's options when you click
          "CC Translate → Export…".
        </div>

        <label class="setting-field checkbox-field">
          <input
            type="checkbox"
            bind:checked={store.enforceCharLimits}
            onchange={persistSettings}
          />
          <span>Enforce character limits</span>
        </label>

        <p class="setting-hint">
          When enabled, Claude will try to keep translations within the block and
          per-line character caps set above. When disabled, Claude translates
          freely for accuracy and flags lines that exceed limits in the notes.
        </p>

        <label class="setting-field checkbox-field">
          <input
            type="checkbox"
            bind:checked={store.crossBinContext}
            onchange={persistSettings}
            disabled={store.fastMode}
          />
          <span>Use cross-file context</span>
          <span class="cost-badge cost-up" title="Scans sibling BINs — significantly more tokens per first request">More tokens</span>
          {#if store.fastMode}
            <span class="cost-badge cost-overridden" title="Forced off while Fast mode is on">Disabled by Fast mode</span>
          {/if}
        </label>

        <p class="setting-hint">
          When enabled, Claude will scan other BIN files in the same folder before
          translating to gather narrative context (character relationships, plot
          events, terminology). Increases accuracy but uses more tokens on the
          first request.
        </p>

        <label class="setting-field checkbox-field">
          <input
            type="checkbox"
            bind:checked={store.minimalNotes}
            onchange={persistSettings}
          />
          <span>Minimal notes (reduce token cost)</span>
          <span class="cost-badge cost-down" title="LLM returns translation only — no notes/explanations">Saves tokens</span>
        </label>

        <p class="setting-hint">
          When enabled, the LLM only returns the translation text — no detailed
          notes or glossary suggestions. Reduces output tokens and shortens the
          system prompt. Notes will show "[LLM: model-name] Translated automatically" instead.
        </p>

        <label class="setting-field checkbox-field">
          <input
            type="checkbox"
            bind:checked={store.fastMode}
            onchange={persistSettings}
          />
          <span>Fast mode (skip previous-dialogue context)</span>
          <span class="cost-badge cost-down" title="Disables prior context + cross-file scan for fastest, cheapest calls">Saves tokens</span>
          <span class="cost-badge cost-info" title="Forces cross-file context off and prior block context to 0">Overrides other settings</span>
        </label>

        <p class="setting-hint">
          When enabled, each translation request omits the rolling window of
          recent dialogue and forces cross-file context off. Much cheaper and
          faster, but the LLM has less narrative context per line.
        </p>

        <label class="setting-field checkbox-field">
          <input
            type="checkbox"
            bind:checked={store.skipGlossary}
            onchange={persistSettings}
          />
          <span>Skip glossary in prompts</span>
          <span class="cost-badge cost-down" title="Doesn't include glossary entries in the LLM prompt">Saves tokens</span>
        </label>

        <p class="setting-hint">
          When enabled, glossary entries are not included in translation prompts.
          Useful when the glossary is large and you want to reduce token usage.
          Glossary editing in the panel still works.
        </p>

        <label class="setting-field checkbox-field">
          <input
            type="checkbox"
            bind:checked={store.scopedGlossary}
            onchange={persistSettings}
            disabled={store.skipGlossary}
          />
          <span>Scoped glossary (only send matching terms per call)</span>
          <span class="cost-badge cost-down" title="Only includes glossary entries whose Japanese appears in the current line/block">Saves tokens</span>
          {#if store.skipGlossary}
            <span class="cost-badge cost-overridden" title="Skip glossary is on, so no glossary is sent at all">Disabled by Skip glossary</span>
          {/if}
        </label>

        <p class="setting-hint">
          When enabled, only glossary entries whose Japanese term appears in the
          current line/block are sent — keeps token cost minimal while preserving
          name and term consistency. Best paired with OpenAI-compatible providers
          where prompt caching isn't a factor. Ignored when "Skip glossary" is on.
        </p>

        <label class="setting-field checkbox-field">
          <input
            type="checkbox"
            bind:checked={store.blockMode}
            onchange={persistSettings}
          />
          <span>Block-mode translation</span>
          <span class="cost-badge cost-down" title="Translates multiple rows per call instead of one-per-call">Fewer calls</span>
        </label>

        <p class="setting-hint">
          When enabled, contiguous dialog rows bounded by wait-for-input are
          translated together in a single LLM call (up to 12 slots per call).
          Falls back to per-row when a block is too large or partially translated.
        </p>

        <label class="setting-field">
          <span>
            Prior block context window
            <span class="cost-badge cost-up" title="Each additional block adds its content to every prompt">More tokens per increment</span>
            {#if store.fastMode}
              <span class="cost-badge cost-overridden" title="Forced to 0 while Fast mode is on">Forced to 0 by Fast mode</span>
            {/if}
          </span>
          <input
            type="number"
            min="0"
            max="20"
            bind:value={store.contextBlocks}
            onchange={persistSettings}
            disabled={store.fastMode}
          />
        </label>

        <p class="setting-hint">
          Number of preceding dialog blocks fed to the LLM as context. 0 disables.
          Forced to 0 in fast mode.
        </p>
      </section>
      {/if}
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
    width: 480px;
    /* Lock height (not max-height) so the dialog stays the same size as the
     * user cycles tabs — otherwise short tabs collapse the dialog and the
     * tab row jumps to a new vertical position, making rapid tab switching
     * a moving-target frustration. Body becomes scrollable for tabs with
     * lots of content; short tabs just have empty space at the bottom. */
    height: 80vh;
    max-height: 700px;
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

  /* Per-setting cost / dependency badges. Color-coded so a glance tells
   * the user the token impact without reading the hint paragraph. */
  .cost-badge {
    display: inline-block;
    margin-left: 6px;
    padding: 1px 6px;
    border-radius: 8px;
    font-size: 0.65rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    vertical-align: middle;
    cursor: help;
  }
  .cost-down { background: #1a3a2a; color: #6fd886; }    /* saves tokens */
  .cost-up { background: #3a2a14; color: #f4c87a; }      /* uses more */
  .cost-info { background: #1a2a4a; color: #8aa0ff; }    /* informational override */
  .cost-overridden { background: #2a1a3a; color: #c08aff; opacity: 0.85; } /* disabled by another setting */

  .scope-note {
    margin: 0 0 14px 0;
    padding: 10px 12px;
    background: #1a2240;
    border: 1px solid #2a3a6e;
    border-left: 3px solid #5a7aff;
    border-radius: 3px;
    font-size: 0.78rem;
    color: #c0c0e0;
    line-height: 1.5;
  }
  .scope-note strong {
    color: #8aa0ff;
    font-weight: 600;
  }
  .scope-note code {
    background: #0e0e22;
    padding: 1px 4px;
    border-radius: 2px;
    color: #e8a838;
    font-size: 0.72rem;
  }

  .settings-tabs {
    display: flex;
    gap: 0;
    padding: 0 12px;
    background: #16213e;
    border-bottom: 1px solid #2a2a4a;
  }
  .tab-btn {
    background: transparent;
    border: none;
    color: #888;
    padding: 8px 14px;
    font-size: 0.82rem;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    transition: color 0.1s, border-color 0.1s;
  }
  .tab-btn:hover { color: #c0c0e0; }
  .tab-btn.active {
    color: #8ab4f8;
    border-bottom-color: #6c9bd2;
  }

  .dialog-body {
    padding: 16px;
    overflow-y: auto;
    /* Flex-grow to fill the locked dialog height so short tabs don't leave
     * the body collapsed (which would still shift the apparent footer). */
    flex: 1;
    min-height: 0;
  }

  .setting-group {
    margin-bottom: 16px;
  }

  .setting-group h3 {
    margin: 0 0 12px 0;
    font-size: 0.85rem;
    color: #8ab4f8;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .setting-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 12px;
  }

  .setting-field span {
    font-size: 0.8rem;
    color: #888;
  }

  .setting-field select,
  .setting-field input {
    padding: 6px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #16213e;
    color: #c0c0e0;
    font-size: 0.85rem;
    outline: none;
  }

  .setting-field select:focus,
  .setting-field input:focus {
    border-color: #6c9bd2;
  }

  .key-input-row {
    display: flex;
    gap: 6px;
  }

  .key-input-row input {
    flex: 1;
    padding: 6px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #16213e;
    color: #c0c0e0;
    font-size: 0.85rem;
    outline: none;
    font-family: monospace;
  }

  .key-input-row input:focus {
    border-color: #6c9bd2;
  }

  .toggle-btn {
    padding: 4px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #888;
    font-size: 0.8rem;
    cursor: pointer;
  }

  .toggle-btn:hover {
    color: #c0c0e0;
    border-color: #5a5a8a;
  }

  .key-actions {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }

  .save-key-btn {
    padding: 6px 14px;
    border: 1px solid #2a5a3e;
    border-radius: 4px;
    background: #1a3a2e;
    color: #6fd886;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .save-key-btn:hover {
    background: #2a4a3e;
  }

  .save-key-btn.saved {
    background: #2d6b3f;
    border-color: #4caf6f;
    color: #eaffea;
    transition: background 180ms ease, color 180ms ease;
  }

  .save-key-btn.error {
    background: #6b2d2d;
    border-color: #b85555;
    color: #ffeaea;
    transition: background 180ms ease, color 180ms ease;
  }

  .key-status {
    font-size: 0.8rem;
  }

  .key-status.configured {
    color: #6fd886;
  }

  .key-status.not-configured {
    color: #e06060;
  }

  .setting-hint {
    margin: 0;
    font-size: 0.75rem;
    color: #555;
    line-height: 1.4;
  }

  .checkbox-field {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }

  .checkbox-field input[type="checkbox"] {
    width: 16px;
    height: 16px;
    accent-color: #6c9bd2;
    cursor: pointer;
  }

  .checkbox-field span {
    font-size: 0.85rem;
    color: #c0c0e0;
  }

  .model-row {
    display: flex;
    gap: 6px;
  }

  .model-row select {
    flex: 1;
    padding: 6px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #16213e;
    color: #c0c0e0;
    font-size: 0.85rem;
    outline: none;
  }

  .model-row select:focus {
    border-color: #6c9bd2;
  }

  .refresh-btn {
    padding: 4px 12px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #1a1a3e;
    color: #8ab4f8;
    font-size: 0.8rem;
    cursor: pointer;
    white-space: nowrap;
  }

  .refresh-btn:hover:not(:disabled) {
    background: #2a2a4e;
    border-color: #6c9bd2;
  }

  .refresh-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .apply-all-btn {
    padding: 4px 10px;
    border: 1px solid #2a4a5a;
    border-radius: 4px;
    background: #1a2a3e;
    color: #8ab4f8;
    font-size: 0.8rem;
    cursor: pointer;
    white-space: nowrap;
  }

  .apply-all-btn:hover {
    background: #2a3a5e;
    border-color: #4a6a8a;
  }

  .ui-config-row {
    display: flex;
    gap: 6px;
  }

  .ui-config-row input {
    flex: 1;
    padding: 6px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #16213e;
    color: #c0c0e0;
    font-size: 0.85rem;
    outline: none;
    min-width: 0;
  }

  .ui-config-row input:focus {
    border-color: #6c9bd2;
  }

  .endpoint-row {
    display: flex;
    gap: 6px;
  }

  .endpoint-row input {
    flex: 1;
    padding: 6px 10px;
    border: 1px solid #3a3a5a;
    border-radius: 4px;
    background: #16213e;
    color: #c0c0e0;
    font-size: 0.85rem;
    font-family: monospace;
    outline: none;
  }

  .endpoint-row input:focus {
    border-color: #6c9bd2;
  }

  .preset-row {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-bottom: 12px;
  }

  .preset-btn {
    padding: 3px 8px;
    border: 1px solid #3a3a5a;
    border-radius: 3px;
    background: #1a1a3e;
    color: #888;
    font-size: 0.72rem;
    cursor: pointer;
    transition: all 0.15s;
  }

  .preset-btn:hover {
    color: #c0c0e0;
    border-color: #5a5a8a;
  }

  .preset-btn.active {
    background: #2a3a6e;
    border-color: #6c9bd2;
    color: #8ab4f8;
  }
</style>
