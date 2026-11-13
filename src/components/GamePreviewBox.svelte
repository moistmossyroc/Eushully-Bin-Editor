<script lang="ts">
  // Game-accurate text box, drawn in the game's own screen pixels (SVG viewBox):
  // the message-window art from the system script (band, name-plate frame, buttons),
  // the text surface with glyphs in engine cells, and its real clip edges.
  // Layout math: src/lib/gameLayout.ts. Geometry/art: engine/box_profile.rs, box_art.rs.
  import type { BoxArtSprite, BoxAtlas } from "../lib/ipc";
  import { bottomLineCapFromArt, bottomLineUiZone, bottomRowOverCap, defaultEngineMetrics, layoutLines, lineCapacityCells, lineCapacityFullWidth, speakerNamePosition, visibleLineCount } from "../lib/gameLayout";
  import type { WrapMode } from "../lib/gameLayout";
  import type { LayoutGeometry } from "../lib/gameLayout";

  interface Props {
    geometry: LayoutGeometry;
    /** Engine metrics + word-wrap hook; default = the inferred engine rule, no word wrap. */
    wrapMode?: WrapMode | null;
    /** Plain text of each display line in the box. */
    lines: string[];
    /** Index of the line holding the selected row (highlighted), or -1. */
    selectedLine: number;
    maxLines: number;
    bottomLineCap: number;
    art: BoxArtSprite[];
    atlases: BoxAtlas[];
    /** Adopt a bottom-line cap derived from the art (null hides the suggestion). */
    onUseBottomLineCap: ((cap: number) => void) | null;
    /** Speaker shown on the name plate (null = narration). */
    speaker: string | null;
  }

  let { geometry, wrapMode = null, lines, selectedLine, maxLines, bottomLineCap, art, atlases, onUseBottomLineCap, speaker }: Props = $props();

  const ART_PADDING = 8;
  /** Frame pieces (band, name plate) are tinted in-game by the window-transparency
   *  setting (SYSTEM4 0x203 on those sprites); buttons are drawn as-is. */
  const FRAME_MIN_WIDTH = 200;

  let surfaceX = $derived(geometry.x ?? 0);
  let surfaceY = $derived(geometry.y ?? 0);
  let drawable = $derived(art.filter((s) => atlases.some((a) => a.texture === s.texture)));
  // Viewport = surface ∪ art, padded.
  let view = $derived.by(() => {
    let left = surfaceX, top = surfaceY, right = surfaceX + geometry.width, bottom = surfaceY + geometry.height;
    for (const s of drawable) {
      left = Math.min(left, s.dstX);
      top = Math.min(top, s.dstY);
      right = Math.max(right, s.dstX + s.width);
      bottom = Math.max(bottom, s.dstY + s.height);
    }
    return { x: left - ART_PADDING, y: top - ART_PADDING, width: right - left + 2 * ART_PADDING, height: bottom - top + 2 * ART_PADDING };
  });

  let mode = $derived<WrapMode>(wrapMode ?? { metrics: defaultEngineMetrics(geometry), wordWrap: false });
  let placed = $derived(layoutLines(lines, geometry, mode));
  let fits = $derived(visibleLineCount(geometry));
  let capacity = $derived(lineCapacityCells(geometry, mode.metrics));
  let capacityFull = $derived(lineCapacityFullWidth(geometry, mode.metrics));
  let uiZone = $derived(bottomLineUiZone(geometry, maxLines, bottomLineCap));
  let artCap = $derived(bottomLineCapFromArt(geometry, art, maxLines));
  let problems = $derived(placed.filter((l) => l.vertical !== "visible" || l.clippedRight).length);
  let wrappedRows = $derived(placed.filter((l) => l.wrapped).length);
  let underButtons = $derived(bottomRowOverCap(placed, maxLines, bottomLineCap));
  let namePosition = $derived(speakerNamePosition(geometry, drawable, geometry.fontPx));

  function atlasFor(texture: number): BoxAtlas | undefined {
    return atlases.find((a) => a.texture === texture);
  }
</script>

<div class="game-box">
  <svg
    viewBox="{view.x} {view.y} {view.width} {view.height}"
    preserveAspectRatio="xMidYMid meet"
    role="img"
    aria-label="In-game text box preview"
  >
    {#each drawable as sprite}
      {@const atlas = atlasFor(sprite.texture)}
      {#if atlas}
        <!-- Nested viewport = the atlas rectangle, placed where the script draws it. -->
        <svg
          x={sprite.dstX}
          y={sprite.dstY}
          width={sprite.width}
          height={sprite.height}
          viewBox="{sprite.srcX} {sprite.srcY} {sprite.width} {sprite.height}"
          overflow="hidden"
          class:frame={sprite.width >= FRAME_MIN_WIDTH}
        >
          <image href={atlas.dataUrl} x="0" y="0" width={atlas.width} height={atlas.height} />
        </svg>
      {/if}
    {/each}
    <rect class="surface" class:over-art={drawable.length > 0} x={surfaceX} y={surfaceY} width={geometry.width} height={geometry.height} />
    {#if uiZone}
      <rect class="ui-zone" x={surfaceX + uiZone.x} y={surfaceY + uiZone.y} width={uiZone.width} height={uiZone.height}>
        <title>Bottom line shared with UI art (bottom-line cap {bottomLineCap})</title>
      </rect>
    {/if}
    {#if speaker}
      <text
        class="line speaker-name"
        font-size={geometry.fontPx}
        dominant-baseline="text-before-edge"
        x={namePosition.x}
        y={namePosition.y}
      >{speaker}</text>
    {/if}
    <!-- Nested viewport = the text surface: the engine clips glyphs at its edges. -->
    <svg x={surfaceX} y={surfaceY} width={geometry.width} height={geometry.height} overflow="hidden">
      {#each placed as line, index}
        {#if line.sourceLine === selectedLine}
          <rect class="selected-row" x={geometry.textX} y={line.y} width={geometry.width - geometry.textX} height={geometry.fontPx} />
        {/if}
        <text
          class="line"
          class:partial={line.vertical === "partial" || (underButtons && index === maxLines - 1)}
          font-size={geometry.fontPx}
          dominant-baseline="text-before-edge"
          x={line.chars.map((c) => c.x).join(" ")}
          y={line.y}
        >{line.chars.map((c) => c.ch).join("")}</text>
      {/each}
    </svg>
  </svg>
  <div class="game-box-meta">
    <span title="Lines that fit fully above the text surface's bottom edge">{fits} lines × {capacity} half-width / {capacityFull} full-width chars</span>
    {#if wrappedRows > 0}
      <span class="dim" title="The engine wraps a line that reaches the right edge onto the next row (per character)">· {wrappedRows} auto-wrapped</span>
    {/if}
    {#if underButtons}
      <span class="bad" title="Row {maxLines} runs under the window's buttons (bottom-line cap {bottomLineCap})">· row {maxLines} under the buttons</span>
    {/if}
    <span class="dim">· {geometry.fontPx}px, step {geometry.fontPx + geometry.lineSpacing}</span>
    {#if problems > 0}
      <span class="bad">· {problems} line{problems === 1 ? "" : "s"} clipped in-game</span>
    {/if}
    {#if artCap !== null && onUseBottomLineCap && artCap !== bottomLineCap}
      <button
        class="cap-btn"
        onclick={() => onUseBottomLineCap?.(artCap!)}
        title="Line {maxLines} shares its row with the window's buttons: {artCap} half-width characters fit before the first one"
      >Use bottom-line cap {artCap}</button>
    {/if}
  </div>
</div>

<style>
  .game-box {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .game-box > svg {
    width: 100%;
    height: auto;
    display: block;
    background: #101014;
    border: 1px solid var(--border, #333);
    border-radius: 4px;
  }
  .surface {
    fill: #2b2b33;
  }
  .frame {
    filter: brightness(0.4);
    opacity: 0.92;
  }
  .surface.over-art {
    fill: none;
    stroke: rgba(120, 160, 255, 0.35);
    stroke-dasharray: 4 4;
  }
  .ui-zone {
    fill: rgba(255, 140, 40, 0.18);
    stroke: rgba(255, 140, 40, 0.7);
    stroke-dasharray: 6 3;
  }
  .selected-row {
    fill: rgba(90, 140, 255, 0.18);
  }
  .line {
    fill: #f2f2f2;
    font-family: "MS Gothic", "ＭＳ ゴシック", "Osaka-Mono", monospace;
    white-space: pre;
    paint-order: stroke;
    stroke: rgba(0, 0, 0, 0.55);
    stroke-width: 2px;
  }
  .speaker-name {
    fill: #ffe7a8;
  }
  .line.partial {
    fill: #ff8080;
  }
  .game-box-meta {
    font-size: 11px;
    color: var(--text-secondary, #aaa);
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
    align-items: center;
  }
  .dim {
    opacity: 0.7;
  }
  .bad {
    color: #ff7070;
  }
  .cap-btn {
    font-size: 11px;
    padding: 0 6px;
    margin-left: auto;
  }
</style>
