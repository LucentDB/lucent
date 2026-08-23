import { describe, it, expect } from 'vitest';
import { computeVirtualRange, shouldFetchMore } from './virtualRange.js';

describe('computeVirtualRange', () => {
  it('starts at 0 with no overscan underflow when scrolled to the top', () => {
    const r = computeVirtualRange({
      scrollTop: 0,
      viewportHeight: 600,
      rowHeight: 33,
      itemCount: 1000,
      overscan: 10,
    });
    expect(r.startIndex).toBe(0);
    expect(r.topSpacerHeight).toBe(0);
  });

  it('subtracts overscan from the scrolled-to start index', () => {
    const r = computeVirtualRange({
      scrollTop: 3300,
      viewportHeight: 600,
      rowHeight: 33,
      itemCount: 1000,
      overscan: 10,
    });
    // scrolled to row 100 (3300/33); overscan pulls the window back 10 rows
    expect(r.startIndex).toBe(90);
    expect(r.topSpacerHeight).toBe(90 * 33);
  });

  it('never lets startIndex go negative near the top', () => {
    const r = computeVirtualRange({
      scrollTop: 66,
      viewportHeight: 600,
      rowHeight: 33,
      itemCount: 1000,
      overscan: 10,
    });
    expect(r.startIndex).toBe(0);
  });

  it('endIndex never exceeds itemCount', () => {
    const r = computeVirtualRange({
      scrollTop: 32800,
      viewportHeight: 600,
      rowHeight: 33,
      itemCount: 1000,
      overscan: 10,
    });
    expect(r.endIndex).toBeLessThanOrEqual(1000);
  });

  it('bottomSpacerHeight is zero when the visible window reaches the end', () => {
    const r = computeVirtualRange({
      scrollTop: 32800,
      viewportHeight: 600,
      rowHeight: 33,
      itemCount: 1000,
      overscan: 10,
    });
    expect(r.bottomSpacerHeight).toBe(0);
  });

  it('bottomSpacerHeight accounts for all rows below the visible window', () => {
    const r = computeVirtualRange({
      scrollTop: 0,
      viewportHeight: 600,
      rowHeight: 33,
      itemCount: 1000,
      overscan: 10,
    });
    expect(r.bottomSpacerHeight).toBe((1000 - r.endIndex) * 33);
  });

  it('handles itemCount of 0 without negative ranges', () => {
    const r = computeVirtualRange({
      scrollTop: 0,
      viewportHeight: 600,
      rowHeight: 33,
      itemCount: 0,
      overscan: 10,
    });
    expect(r.startIndex).toBe(0);
    expect(r.endIndex).toBe(0);
    expect(r.topSpacerHeight).toBe(0);
    expect(r.bottomSpacerHeight).toBe(0);
  });
});

describe('shouldFetchMore', () => {
  it('returns false when far from the fetched edge', () => {
    const result = shouldFetchMore({
      scrollTop: 0,
      viewportHeight: 600,
      rowHeight: 33,
      itemCount: 1000,
      fetchThreshold: 100,
    });
    expect(result).toBe(false);
  });

  it('returns true when within the threshold of the fetched edge', () => {
    const result = shouldFetchMore({
      scrollTop: 30000,
      viewportHeight: 600,
      rowHeight: 33,
      itemCount: 1000,
      fetchThreshold: 100,
    });
    expect(result).toBe(true);
  });

  it('returns true when already past the fetched edge', () => {
    const result = shouldFetchMore({
      scrollTop: 33000,
      viewportHeight: 600,
      rowHeight: 33,
      itemCount: 1000,
      fetchThreshold: 100,
    });
    expect(result).toBe(true);
  });
});
