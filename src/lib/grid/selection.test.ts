import { describe, it, expect } from 'vitest';
import {
  EMPTY,
  boundsOf,
  cellAt,
  edgesOf,
  extendTo,
  hasCell,
  isActive,
  isCursor,
  moveBy,
  rowAt,
  selectionToTsv,
} from './selection.js';

describe('cell selection model', () => {
  it('starts with nothing selected', () => {
    expect(isActive(EMPTY)).toBe(false);
    expect(hasCell(EMPTY, 0, 0)).toBe(false);
    expect(boundsOf(EMPTY)).toBeNull();
  });

  it('selects a single cell', () => {
    const sel = cellAt(2, 3);
    expect(isActive(sel)).toBe(true);
    expect(hasCell(sel, 2, 3)).toBe(true);
    expect(hasCell(sel, 2, 4)).toBe(false);
    expect(isCursor(sel, 2, 3)).toBe(true);
    expect(boundsOf(sel)).toEqual({ r0: 2, r1: 2, c0: 3, c1: 3 });
  });

  it('extends to a rectangle, normalising the corners', () => {
    // Dragging up and to the left: the anchor stays put, bounds normalise.
    const sel = extendTo(cellAt(4, 5), 2, 3);
    expect(boundsOf(sel)).toEqual({ r0: 2, r1: 4, c0: 3, c1: 5 });
    expect(hasCell(sel, 3, 4)).toBe(true);
    expect(hasCell(sel, 1, 3)).toBe(false);
    expect(hasCell(sel, 2, 2)).toBe(false);
    // The cursor is where the drag ended; the anchor is where it began.
    expect(isCursor(sel, 2, 3)).toBe(true);
    expect(isCursor(sel, 4, 5)).toBe(false);
  });

  it('extending an empty selection just selects that cell', () => {
    expect(boundsOf(extendTo(EMPTY, 1, 1))).toEqual({
      r0: 1,
      r1: 1,
      c0: 1,
      c1: 1,
    });
  });

  it('never mutates the selection it was given', () => {
    const first = cellAt(0, 0);
    const second = extendTo(first, 3, 3);
    expect(boundsOf(first)).toEqual({ r0: 0, r1: 0, c0: 0, c1: 0 });
    expect(boundsOf(second)).toEqual({ r0: 0, r1: 3, c0: 0, c1: 3 });
    expect(second).not.toBe(first);
  });

  it('walks the cursor and clamps at the edges', () => {
    const grid = { rows: 3, cols: 3 };
    expect(boundsOf(moveBy(cellAt(0, 0), 1, 0, grid))).toEqual({
      r0: 1,
      r1: 1,
      c0: 0,
      c1: 0,
    });
    // Already at the first cell: moving up and left must stay put, not go
    // negative.
    expect(boundsOf(moveBy(cellAt(0, 0), -1, -1, grid))).toEqual({
      r0: 0,
      r1: 0,
      c0: 0,
      c1: 0,
    });
    // And at the far corner it stops there.
    expect(boundsOf(moveBy(cellAt(2, 2), 5, 5, grid))).toEqual({
      r0: 2,
      r1: 2,
      c0: 2,
      c1: 2,
    });
  });

  it('grows the range instead of moving it when extending', () => {
    const grid = { rows: 4, cols: 4, extend: true };
    const down = moveBy(cellAt(1, 1), 1, 0, grid);
    expect(boundsOf(down)).toEqual({ r0: 1, r1: 2, c0: 1, c1: 1 });
    expect(boundsOf(moveBy(down, 0, 1, grid))).toEqual({
      r0: 1,
      r1: 2,
      c0: 1,
      c1: 2,
    });
  });

  it('shrinks a range from the side it grew from', () => {
    // The anchor is remembered, so Shift+Up after Shift+Down undoes it rather
    // than growing the other way.
    const grid = { rows: 5, cols: 5, extend: true };
    const grown = moveBy(moveBy(cellAt(2, 0), 1, 0, grid), 1, 0, grid);
    expect(boundsOf(grown)).toEqual({ r0: 2, r1: 4, c0: 0, c1: 0 });
    expect(boundsOf(moveBy(grown, -1, 0, grid))).toEqual({
      r0: 2,
      r1: 3,
      c0: 0,
      c1: 0,
    });
  });

  it('arrowing into an untouched grid lands on its first cell', () => {
    expect(boundsOf(moveBy(EMPTY, 1, 1, { rows: 4, cols: 4 }))).toEqual({
      r0: 0,
      r1: 0,
      c0: 0,
      c1: 0,
    });
  });

  it('clamps to zero on an empty grid rather than returning -1', () => {
    expect(boundsOf(moveBy(EMPTY, 1, 1, { rows: 0, cols: 0 }))).toEqual({
      r0: 0,
      r1: 0,
      c0: 0,
      c1: 0,
    });
  });

  it('selects every column of one row', () => {
    const sel = rowAt(2, 4);
    expect(boundsOf(sel)).toEqual({ r0: 2, r1: 2, c0: 0, c1: 3 });
    expect(hasCell(sel, 2, 0)).toBe(true);
    expect(hasCell(sel, 2, 3)).toBe(true);
    expect(hasCell(sel, 1, 0)).toBe(false);
  });
});

describe('edgesOf', () => {
  // The range is drawn as one outline, so each cell needs to know which of
  // its sides lie on the boundary. Per-cell borders would draw a grid of
  // boxes instead of a single marquee.
  it('marks all four sides for a single-cell selection', () => {
    expect(edgesOf({ r0: 1, r1: 1, c0: 1, c1: 1 }, 1, 1)).toEqual({
      top: true,
      bottom: true,
      left: true,
      right: true,
    });
  });

  it('marks only the outer sides of a rectangle', () => {
    const b = { r0: 0, r1: 2, c0: 0, c1: 2 };
    expect(edgesOf(b, 0, 1)).toEqual({
      top: true,
      bottom: false,
      left: false,
      right: false,
    });
    expect(edgesOf(b, 1, 1)).toEqual({
      top: false,
      bottom: false,
      left: false,
      right: false,
    });
    expect(edgesOf(b, 2, 0)).toEqual({
      top: false,
      bottom: true,
      left: true,
      right: false,
    });
  });

  it('returns no edges for a cell outside the range', () => {
    expect(edgesOf({ r0: 0, r1: 1, c0: 0, c1: 1 }, 5, 5)).toEqual({
      top: false,
      bottom: false,
      left: false,
      right: false,
    });
  });

  it('returns no edges when nothing is selected', () => {
    expect(edgesOf(null, 0, 0)).toEqual({
      top: false,
      bottom: false,
      left: false,
      right: false,
    });
  });
});

describe('selectionToTsv', () => {
  const rows = [
    ['AAA', 'Anaa', 1],
    ['AAC', 'El Arish', 2],
    ['AAE', 'Rabah Bitat', 3],
  ];
  // Column order on screen need not match the row tuple's order, so the
  // mapping from visible column to tuple index is explicit.
  const colIndexes = [0, 1, 2];

  it('copies a single cell as its bare value', () => {
    expect(
      selectionToTsv(rows, colIndexes, { r0: 1, r1: 1, c0: 1, c1: 1 }),
    ).toBe('El Arish');
  });

  it('joins a row range with tabs', () => {
    expect(
      selectionToTsv(rows, colIndexes, { r0: 0, r1: 0, c0: 0, c1: 2 }),
    ).toBe('AAA\tAnaa\t1');
  });

  it('joins a column range with newlines', () => {
    expect(
      selectionToTsv(rows, colIndexes, { r0: 0, r1: 2, c0: 0, c1: 0 }),
    ).toBe('AAA\nAAC\nAAE');
  });

  it('lays a rectangle out as tab-separated lines', () => {
    expect(
      selectionToTsv(rows, colIndexes, { r0: 0, r1: 1, c0: 0, c1: 1 }),
    ).toBe('AAA\tAnaa\nAAC\tEl Arish');
  });

  it('honours the visible column order, not the tuple order', () => {
    expect(selectionToTsv(rows, [2, 0], { r0: 0, r1: 0, c0: 0, c1: 1 })).toBe(
      '1\tAAA',
    );
  });

  it('renders null through the shared cell formatter', () => {
    expect(selectionToTsv([[null]], [0], { r0: 0, r1: 0, c0: 0, c1: 0 })).toBe(
      '',
    );
  });

  it('is empty when nothing is selected', () => {
    expect(selectionToTsv(rows, colIndexes, null)).toBe('');
  });

  it('skips rows that are not loaded rather than emitting undefined', () => {
    expect(
      selectionToTsv(rows, colIndexes, { r0: 2, r1: 9, c0: 0, c1: 0 }),
    ).toBe('AAE');
  });
});
