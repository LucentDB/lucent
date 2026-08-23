import { describe, it, expect } from 'vitest';
import { anchorPosition, availableHeight } from './anchor.js';

const VIEWPORT = { width: 1200, height: 800 };
const POPOVER = { width: 260, height: 200 };

describe('anchorPosition', () => {
  it('opens below the anchor when there is room', () => {
    const r = anchorPosition(
      { top: 100, bottom: 120, left: 300 },
      POPOVER,
      VIEWPORT,
    );
    expect(r.placement).toBe('below');
    expect(r.y).toBe(124);
    expect(r.x).toBe(300);
  });

  it('flips above the anchor when below would overflow', () => {
    const r = anchorPosition(
      { top: 700, bottom: 720, left: 300 },
      POPOVER,
      VIEWPORT,
    );
    expect(r.placement).toBe('above');
    expect(r.y).toBe(496);
  });

  it('stays below when neither side fits, so the popover can scroll instead', () => {
    const tall = { width: 260, height: 780 };
    const r = anchorPosition(
      { top: 380, bottom: 400, left: 300 },
      tall,
      VIEWPORT,
    );
    expect(r.placement).toBe('below');
  });

  it('pulls a popover back inside the right edge', () => {
    const r = anchorPosition(
      { top: 100, bottom: 120, left: 1150 },
      POPOVER,
      VIEWPORT,
    );
    expect(r.x).toBe(1200 - 260 - 8);
  });

  it('never positions left of the margin', () => {
    const r = anchorPosition(
      { top: 100, bottom: 120, left: -50 },
      POPOVER,
      VIEWPORT,
    );
    expect(r.x).toBe(8);
  });

  it('never positions above the margin', () => {
    const r = anchorPosition(
      { top: 4, bottom: 8, left: 300 },
      POPOVER,
      VIEWPORT,
    );
    expect(r.y).toBeGreaterThanOrEqual(8);
  });

  it('keeps a popover taller than the viewport at the top margin', () => {
    const huge = { width: 260, height: 2000 };
    const r = anchorPosition(
      { top: 100, bottom: 120, left: 300 },
      huge,
      VIEWPORT,
    );
    expect(r.y).toBe(8);
  });

  it('respects a custom gap', () => {
    const r = anchorPosition(
      { top: 100, bottom: 120, left: 300 },
      POPOVER,
      VIEWPORT,
      { gap: 12 },
    );
    expect(r.y).toBe(132);
  });
});

describe('availableHeight', () => {
  it('reports the room left below a position', () => {
    expect(availableHeight(200, VIEWPORT)).toBe(592);
  });

  it('never reports a negative height', () => {
    expect(availableHeight(900, VIEWPORT)).toBe(0);
  });
});
