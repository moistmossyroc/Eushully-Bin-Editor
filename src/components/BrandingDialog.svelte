<script lang="ts">
  /**
   * Fanpatch branding for SYS4 games: a title badge (append-style banner or
   * tile) and an extra boot splash, written as `zz_branding.fanpatch` into the
   * game folder. The backend detects what each game supports (engine::branding);
   * art is rendered here on canvases at the game's own sizes. See
   * docs/engine/age-sys.md §4–5.
   */
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    engineBuildBranding,
    engineInspectBranding,
    engineReadImageDataUrl,
    type BrandingProfile,
  } from "../lib/ipc";
  import { DEFAULT_SPLASH_LINES, renderBannerBadge, renderSplash, renderTileBadge } from "../lib/brandingArt";
  import { store } from "../lib/stores.svelte";
  import { toastInfo, toastWarn } from "../lib/toast";

  interface Props {
    onClose: () => void;
  }
  let { onClose }: Props = $props();

  let gameDir = $state<string | null>(null);
  let profile = $state<BrandingProfile | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);

  let badgeEnabled = $state(true);
  let bannerText = $state("★moist fanpatch v1.0 English");
  let tilePlateText = $state("EN TL");
  let tileLabelText = $state("moist");

  let splashEnabled = $state(true);
  let splashHeader = $state("English Fan Translation");
  let splashSubtitle = $state("moist fanpatch v1.0");
  let splashBody = $state(DEFAULT_SPLASH_LINES.join("\n"));
  let splashImage = $state<HTMLImageElement | null>(null);
  let splashImageName = $state<string | null>(null);

  let badgeCanvas = $state<HTMLCanvasElement | null>(null);
  let splashCanvas = $state<HTMLCanvasElement | null>(null);

  const badgeSupported = $derived(profile !== null && profile.badge.kind !== "unsupported");
  const splashSupported = $derived(profile !== null && profile.splash.kind !== "unsupported");
  const badgeSize = $derived.by(() => {
    const badge = profile?.badge;
    return badge && badge.kind !== "unsupported" ? { width: badge.width, height: badge.height } : null;
  });

  async function pickGame(): Promise<void> {
    const picked = await open({ directory: true, title: "Select the SYS4 game folder (the one with AGE.EXE)" });
    if (!picked || Array.isArray(picked)) return;
    gameDir = picked;
    profile = null;
    error = null;
    busy = true;
    try {
      profile = await engineInspectBranding(picked);
      badgeEnabled = profile.badge.kind !== "unsupported";
      splashEnabled = profile.splash.kind !== "unsupported";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function pickSplashImage(): Promise<void> {
    const picked = await open({
      title: "Splash image (shown left of the text)",
      filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "gif"] }],
    });
    if (!picked || Array.isArray(picked)) return;
    const image = new Image();
    image.src = await engineReadImageDataUrl(picked);
    await image.decode();
    splashImage = image;
    splashImageName = picked.replace(/\\/g, "/").split("/").pop() ?? picked;
  }

  // Live previews.
  $effect(() => {
    if (!badgeCanvas || !badgeSize || !profile) return;
    badgeCanvas.width = badgeSize.width;
    badgeCanvas.height = badgeSize.height;
    if (profile.badge.kind === "tileRow") renderTileBadge(badgeCanvas, tilePlateText, tileLabelText);
    else renderBannerBadge(badgeCanvas, bannerText);
  });
  $effect(() => {
    if (!splashCanvas || !profile) return;
    splashCanvas.width = profile.screenWidth;
    splashCanvas.height = profile.screenHeight;
    renderSplash(splashCanvas, { header: splashHeader, subtitle: splashSubtitle, bodyLines: splashBody.split("\n") }, splashImage);
  });

  async function build(): Promise<void> {
    if (!gameDir || !profile) return;
    const badgePng = badgeEnabled && badgeSupported && badgeCanvas ? badgeCanvas.toDataURL("image/png") : null;
    const splashPng = splashEnabled && splashSupported && splashCanvas ? splashCanvas.toDataURL("image/png") : null;
    if (!badgePng && !splashPng) {
      toastWarn("Nothing to build: enable the badge and/or the splash.");
      return;
    }
    busy = true;
    try {
      const written = await engineBuildBranding(gameDir, badgePng, splashPng);
      toastInfo(`Branding written: ${written}`);
      store.statusMessage = `Branding fanpatch written to ${written} (needs the fanpatch loader, version.dll, in the game folder).`;
      onClose();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") onClose();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onclick={onClose}></div>

<div class="dialog" role="dialog" aria-label="Fanpatch branding">
  <header><h3>Fanpatch branding — title badge + boot splash</h3></header>
  <div class="body">
    <div class="row">
      <button class="btn" onclick={pickGame} disabled={busy}>{gameDir ? "Change game folder…" : "Select game folder…"}</button>
      <span class="path">{gameDir ?? "SYS4 game folder (with AGE.EXE)"}</span>
    </div>

    {#if error}<div class="error">{error}</div>{/if}

    {#if profile}
      <div class="profile">
        <strong>{profile.title}</strong> · {profile.build} · {profile.screenWidth}×{profile.screenHeight}
      </div>

      <section>
        <label class="toggle">
          <input type="checkbox" bind:checked={badgeEnabled} disabled={!badgeSupported} />
          Title badge
          {#if profile.badge.kind === "bannerArray"}<span class="hint">(append banner, stacked under official ones)</span>{/if}
          {#if profile.badge.kind === "tileRow"}<span class="hint">(append tile, after official tiles)</span>{/if}
        </label>
        {#if profile.badge.kind === "unsupported"}
          <p class="hint">Not available: {profile.badge.reason}</p>
        {:else if profile.badge.kind === "tileRow"}
          <div class="fields">
            <input type="text" bind:value={tilePlateText} placeholder="Plate text" />
            <input type="text" bind:value={tileLabelText} placeholder="Label text" />
          </div>
        {:else}
          <input type="text" class="wide" bind:value={bannerText} />
        {/if}
        {#if badgeSize}
          <canvas class="badge-preview" bind:this={badgeCanvas} style="width: {badgeSize.width * 2}px"></canvas>
        {/if}
      </section>

      <section>
        <label class="toggle">
          <input type="checkbox" bind:checked={splashEnabled} disabled={!splashSupported} />
          Boot splash <span class="hint">(after the official splashes, same fades + click-to-skip)</span>
        </label>
        {#if profile.splash.kind === "unsupported"}
          <p class="hint">Not available: {profile.splash.reason}</p>
        {:else}
          <div class="fields">
            <input type="text" bind:value={splashHeader} placeholder="Header" />
            <input type="text" bind:value={splashSubtitle} placeholder="Subtitle" />
          </div>
          <textarea rows="4" bind:value={splashBody}></textarea>
          <div class="row">
            <button class="btn" onclick={pickSplashImage}>Splash image…</button>
            {#if splashImageName}
              <span class="path">{splashImageName}</span>
              <button class="btn" onclick={() => { splashImage = null; splashImageName = null; }}>Remove</button>
            {/if}
          </div>
          <canvas class="splash-preview" bind:this={splashCanvas}></canvas>
        {/if}
      </section>
    {/if}
  </div>
  <footer>
    <button class="btn" onclick={onClose}>Cancel</button>
    <button class="btn primary" onclick={build} disabled={busy || !profile || (!badgeSupported && !splashSupported)}>
      {busy ? "Working…" : "Write zz_branding.fanpatch"}
    </button>
  </footer>
</div>

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.55); z-index: 2000; }
  .dialog {
    position: fixed; top: 50%; left: 50%; transform: translate(-50%, -50%); z-index: 2001;
    width: 640px; max-width: 94vw; max-height: 90vh; overflow: auto;
    background: #15152a; border: 1px solid #3a3a5a; border-radius: 6px;
    display: flex; flex-direction: column; box-shadow: 0 8px 32px rgba(0, 0, 0, 0.6);
  }
  header { padding: 12px 16px; border-bottom: 1px solid #2a2a4a; }
  header h3 { margin: 0; font-size: 1rem; color: #8ab4f8; }
  .body { padding: 14px 16px; color: #c0c0e0; font-size: 0.86rem; display: flex; flex-direction: column; gap: 12px; }
  .row { display: flex; gap: 8px; align-items: center; }
  .path { color: #9090b0; font-size: 0.8rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .profile { color: #d0d0f0; }
  .error { color: #ff9090; white-space: pre-line; }
  section { display: flex; flex-direction: column; gap: 6px; border-top: 1px solid #2a2a4a; padding-top: 10px; }
  .toggle { display: flex; gap: 6px; align-items: center; font-weight: 600; }
  .hint { color: #8888a8; font-weight: normal; font-size: 0.8rem; margin: 0; }
  .fields { display: flex; gap: 6px; }
  .fields input { flex: 1; }
  input[type="text"], textarea {
    background: #0f0f22; color: #e0e0f0; border: 1px solid #3a3a5a; border-radius: 4px; padding: 5px 7px; font-size: 0.85rem;
  }
  .wide { width: 100%; box-sizing: border-box; }
  textarea { resize: vertical; font-family: inherit; }
  .badge-preview { image-rendering: pixelated; background: repeating-conic-gradient(#333 0 25%, #222 0 50%) 0 0 / 12px 12px; align-self: flex-start; }
  .splash-preview { width: 100%; border: 1px solid #3a3a5a; }
  footer { padding: 12px 16px; border-top: 1px solid #2a2a4a; display: flex; gap: 8px; justify-content: flex-end; }
  .btn { padding: 6px 14px; border: 1px solid #3a3a5a; border-radius: 4px; background: #1a1a3e; color: #c0c0e0; cursor: pointer; font-size: 0.85rem; }
  .btn:hover:not(:disabled) { background: #2a2a5e; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn.primary { background: #2a3a6e; border-color: #5a7aff; color: #fff; font-weight: 600; }
</style>
