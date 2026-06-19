/**
 * Svelte action: keep a popup/menu inside the viewport. After the node mounts
 * we measure it and, if it spills past the right/bottom (or top/left) edge,
 * nudge it back with a transform. Positioning-context-agnostic (works for both
 * `position: fixed` and `absolute` menus since the delta is applied in viewport
 * space via translate).
 *
 * Usage: <div use:clampToViewport style="left:{x}px; top:{y}px"> … </div>
 * Because the menus are gated behind {#if}, the action re-runs each time the
 * menu reopens at a fresh position.
 */
export function clampToViewport(node: HTMLElement, margin = 6) {
  function clamp() {
    // Reset any prior nudge before re-measuring.
    node.style.transform = "";
    const r = node.getBoundingClientRect();
    let dx = 0;
    let dy = 0;
    if (r.right > window.innerWidth - margin) dx = window.innerWidth - margin - r.right;
    if (r.bottom > window.innerHeight - margin) dy = window.innerHeight - margin - r.bottom;
    // Never push past the top/left edges (small menus on tiny windows).
    if (r.left + dx < margin) dx = margin - r.left;
    if (r.top + dy < margin) dy = margin - r.top;
    if (dx !== 0 || dy !== 0) node.style.transform = `translate(${dx}px, ${dy}px)`;
  }
  requestAnimationFrame(clamp);
  return {};
}
