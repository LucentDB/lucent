// Viewport-aware positioning for the grid's popovers (column menu, cell menu,
// column picker). Pure so the flip/clamp rules are testable without layout.
//
// All popovers use position: fixed, because the grid's own container sets
// overflow: hidden and would otherwise clip an absolutely-positioned child.

const MARGIN = 8;

/**
 * Places a popover of `width`×`height` next to an anchor rect, flipping above
 * the anchor when there isn't room below and clamping into the viewport so no
 * edge is ever unreachable.
 *
 * @param {{top: number, bottom: number, left: number}} anchor
 * @param {{width: number, height: number}} popover
 * @param {{width: number, height: number}} viewport
 * @param {{gap?: number, margin?: number}} [opts]
 * @returns {{x: number, y: number, placement: 'below' | 'above'}}
 */
export function anchorPosition(anchor, popover, viewport, opts = {}) {
  const gap = opts.gap ?? 4;
  const margin = opts.margin ?? MARGIN;

  const roomBelow = viewport.height - anchor.bottom - gap - margin;
  const roomAbove = anchor.top - gap - margin;
  // Flip only when below genuinely doesn't fit AND above fits better, so a
  // popover taller than both halves still opens downward and scrolls.
  const placement =
    popover.height > roomBelow && roomAbove > roomBelow ? 'above' : 'below';

  const rawY =
    placement === 'below'
      ? anchor.bottom + gap
      : anchor.top - gap - popover.height;

  const maxX = viewport.width - popover.width - margin;
  const maxY = viewport.height - popover.height - margin;

  return {
    x: clamp(anchor.left, margin, Math.max(margin, maxX)),
    y: clamp(rawY, margin, Math.max(margin, maxY)),
    placement,
  };
}

function clamp(value, min, max) {
  return Math.min(Math.max(value, min), max);
}

/** Largest height a popover may take at `y` before it runs off the viewport. */
export function availableHeight(y, viewport, opts = {}) {
  const margin = opts.margin ?? MARGIN;
  return Math.max(0, viewport.height - y - margin);
}
