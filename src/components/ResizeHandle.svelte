<script lang="ts">
  /**
   * Vertical drag handle for resizing side panels.
   *
   * Sits on the LEFT edge of the parent panel (parent must be position: relative
   * or fixed). Dragging the handle leftward widens the panel; rightward shrinks.
   *
   * The parent owns the width value — we just call `onResize(newWidth)` on every
   * mousemove, and `onResizeEnd()` once on mouseup so the caller can persist.
   */
  interface Props {
    width: number;
    minWidth?: number;
    maxWidth?: number;
    onResize: (newWidth: number) => void;
    onResizeEnd?: () => void;
  }

  let { width, minWidth = 200, maxWidth = 900, onResize, onResizeEnd }: Props = $props();

  let dragging = $state(false);

  function handleMouseDown(e: MouseEvent) {
    e.preventDefault();
    dragging = true;
    const startX = e.clientX;
    const startWidth = width;

    function onMove(ev: MouseEvent) {
      // Dragging left (negative deltaX) should grow the panel since the handle
      // is on the panel's LEFT edge.
      const deltaX = ev.clientX - startX;
      const next = Math.max(minWidth, Math.min(maxWidth, startWidth - deltaX));
      onResize(next);
    }
    function onUp() {
      dragging = false;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      onResizeEnd?.();
    }
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="resize-handle"
  class:dragging
  onmousedown={handleMouseDown}
  role="separator"
  tabindex="0"
  aria-orientation="vertical"
  title="Drag to resize"
></div>

<style>
  .resize-handle {
    position: absolute;
    left: -3px;
    top: 0;
    bottom: 0;
    width: 6px;
    cursor: ew-resize;
    z-index: 60;
    background: transparent;
    transition: background 0.15s;
  }
  .resize-handle:hover,
  .resize-handle.dragging {
    background: #5a7aff;
  }
</style>
