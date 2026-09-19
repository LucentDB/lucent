import { describe, it, expect } from 'vitest';
import { computeTurnLayout, ANCHOR_PAD } from './chat-scroll.ts';

describe('computeTurnLayout — streaming turn scroll choreography', () => {
  it('reserves the leftover viewport below a short response so the anchor stays put', () => {
    // 400px viewport; the anchored message starts 900px down the thread and
    // only 60px of response sits below it.
    const layout = computeTurnLayout(
      {
        viewportHeight: 400,
        scrollHeight: 960,
        spacerHeight: 0,
        anchorTop: 900,
      },
      true,
    );
    // The spacer absorbs everything left over, minus the breathing pad.
    expect(layout.spacerHeight).toBe(400 - 60 - ANCHOR_PAD);
    // The scroll lands on the anchor: anchorTop - pad.
    expect(layout.scrollTop).toBe(900 - ANCHOR_PAD);
  });

  it('follows the tail once the response outgrows the viewport', () => {
    const layout = computeTurnLayout(
      {
        viewportHeight: 400,
        scrollHeight: 1500,
        spacerHeight: 0,
        anchorTop: 900,
      },
      true,
    );
    expect(layout.spacerHeight).toBe(0);
    // Bottom of the content.
    expect(layout.scrollTop).toBe(1500 - 400);
  });

  it('subtracts the spacer already in the DOM before measuring the thread', () => {
    // 300px of spacer is already applied: the thread itself is 960px tall.
    const layout = computeTurnLayout(
      {
        viewportHeight: 400,
        scrollHeight: 1260,
        spacerHeight: 300,
        anchorTop: 900,
      },
      true,
    );
    expect(layout.spacerHeight).toBe(400 - 60 - ANCHOR_PAD);
    expect(layout.scrollTop).toBe(900 - ANCHOR_PAD);
  });

  it('resizes the spacer but leaves the scroll alone when the reader scrolled away', () => {
    const layout = computeTurnLayout(
      {
        viewportHeight: 400,
        scrollHeight: 960,
        spacerHeight: 0,
        anchorTop: 900,
      },
      false,
    );
    expect(layout.spacerHeight).toBe(400 - 60 - ANCHOR_PAD);
    expect(layout.scrollTop).toBeNull();
  });

  it('never asks for a negative spacer or scroll', () => {
    const layout = computeTurnLayout(
      { viewportHeight: 0, scrollHeight: 0, spacerHeight: 0, anchorTop: 0 },
      true,
    );
    expect(layout.spacerHeight).toBe(0);
    expect(layout.scrollTop).toBe(0);
  });
});
