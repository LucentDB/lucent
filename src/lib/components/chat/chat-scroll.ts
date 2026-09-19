/**
 * Scroll geometry for one assistant turn.
 *
 * The chat pane pins the newest user message to the top of the viewport when
 * it is sent and lets the thinking/response stream into the empty space below
 * it. That space is a spacer element at the end of the thread: it absorbs the
 * viewport height the turn has not used yet, so "scroll to the bottom" keeps
 * the message where it is instead of pushing it off-screen. Once the response
 * outgrows the viewport the spacer is zero and the same "bottom" scroll
 * follows the newest text.
 */

/** Breathing room kept above the anchored message. */
export const ANCHOR_PAD = 8;

export interface TurnLayoutMetrics {
  /** Visible height of the message viewport. */
  viewportHeight: number;
  /** Full scrollable height, including the spacer currently applied. */
  scrollHeight: number;
  /** Height the spacer currently occupies (0 when absent). */
  spacerHeight: number;
  /** Offset of the anchored message's top within the scrollable content. */
  anchorTop: number;
}

export interface TurnLayout {
  /** Height the tail spacer should take. */
  spacerHeight: number;
  /**
   * Where to put the scroll container, or null to leave it where the reader
   * left it (they scrolled away from the turn).
   */
  scrollTop: number | null;
}

export function computeTurnLayout(
  m: TurnLayoutMetrics,
  followTail: boolean,
  pad = ANCHOR_PAD,
): TurnLayout {
  const contentHeight = Math.max(0, m.scrollHeight - m.spacerHeight);
  const contentBelow = Math.max(0, contentHeight - m.anchorTop);
  const spacerHeight = Math.max(0, m.viewportHeight - contentBelow - pad);
  const scrollTop = Math.max(
    0,
    contentHeight + spacerHeight - m.viewportHeight,
  );
  return { spacerHeight, scrollTop: followTail ? scrollTop : null };
}
