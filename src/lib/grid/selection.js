// Cell selection for the result grid.
//
// A native table treats a cell as an object: you click it, it fills solid, the
// arrow keys walk the cursor, and Cmd+C copies what is selected. The browser's
// own text selection does something quite different — it drags a ragged
// character range across cell boundaries, which is the single clearest sign
// that a table is a web page rather than an application.
//
// So body cells set `user-select: none` and this module owns the selection
// instead. Pure functions over a plain value: every one returns a new
// selection rather than mutating the one it was given, and nothing here knows
// about Svelte or the DOM. ResultsGrid holds the current value in $state.
//
// Coordinates are (row, col) where `row` indexes the visible page's rows and
// `col` indexes the *visible* columns, left to right. Neither is a tuple index
// into the raw row: columns can be reordered and hidden, so the mapping to
// tuple indexes is passed in explicitly wherever it matters.

import { formatCell } from './format.js';

/**
 * @typedef {{ row: number, col: number }} Cell
 * @typedef {{ anchor: Cell | null, cursor: Cell | null }} Selection
 * @typedef {{ r0: number, r1: number, c0: number, c1: number }} Bounds
 * @typedef {{ rows: number, cols: number, extend?: boolean }} WalkOptions
 * @typedef {{ top: boolean, bottom: boolean, left: boolean, right: boolean }} Edges
 */

const NO_EDGES = { top: false, bottom: false, left: false, right: false };

/** Nothing selected. @type {Selection} */
export const EMPTY = Object.freeze({ anchor: null, cursor: null });

function clamp(n, max) {
  if (max <= 0) return 0;
  return Math.max(0, Math.min(n, max - 1));
}

/**
 * A selection of exactly one cell.
 * @param {number} row
 * @param {number} col
 * @returns {Selection}
 */
export function cellAt(row, col) {
  return { anchor: { row, col }, cursor: { row, col } };
}

/**
 * Every column of one row, as a row click does in a native table.
 * @param {number} row
 * @param {number} colCount
 * @returns {Selection}
 */
export function rowAt(row, colCount) {
  return {
    anchor: { row, col: 0 },
    cursor: { row, col: Math.max(0, colCount - 1) },
  };
}

/**
 * Grows the range from its existing anchor out to (row, col).
 *
 * `anchor` is where the selection began and stays put; `cursor` is the end
 * that moves. Keeping both, rather than a normalised rectangle, is what lets
 * Shift+Arrow shrink a range from the side it grew from — a normalised box has
 * forgotten which corner the user started at.
 *
 * @param {Selection} sel
 * @param {number} row
 * @param {number} col
 * @returns {Selection}
 */
export function extendTo(sel, row, col) {
  if (sel.anchor === null) return cellAt(row, col);
  return { anchor: sel.anchor, cursor: { row, col } };
}

/**
 * Walks the cursor by (dr, dc), clamped to the grid. With `extend` the anchor
 * stays put so the range grows; without it the selection collapses to the cell
 * walked to. Arrowing into an untouched grid lands on its first cell rather
 * than doing nothing.
 *
 * @param {Selection} sel
 * @param {number} dr
 * @param {number} dc
 * @param {WalkOptions} opts
 * @returns {Selection}
 */
export function moveBy(sel, dr, dc, { rows, cols, extend = false }) {
  const started = sel.cursor !== null;
  const row = clamp(started ? sel.cursor.row + dr : 0, rows);
  const col = clamp(started ? sel.cursor.col + dc : 0, cols);
  if (extend && sel.anchor !== null) {
    return { anchor: sel.anchor, cursor: { row, col } };
  }
  return cellAt(row, col);
}

/**
 * The normalised rectangle, or null when nothing is selected.
 * @param {Selection} sel
 * @returns {Bounds | null}
 */
export function boundsOf(sel) {
  if (sel.anchor === null || sel.cursor === null) return null;
  return {
    r0: Math.min(sel.anchor.row, sel.cursor.row),
    r1: Math.max(sel.anchor.row, sel.cursor.row),
    c0: Math.min(sel.anchor.col, sel.cursor.col),
    c1: Math.max(sel.anchor.col, sel.cursor.col),
  };
}

/** @param {Selection} sel */
export function isActive(sel) {
  return sel.anchor !== null && sel.cursor !== null;
}

/** @param {Selection} sel @param {number} row @param {number} col */
export function hasCell(sel, row, col) {
  const b = boundsOf(sel);
  if (b === null) return false;
  return row >= b.r0 && row <= b.r1 && col >= b.c0 && col <= b.c1;
}

/**
 * The one cell the keyboard would move from.
 * @param {Selection} sel @param {number} row @param {number} col
 */
export function isCursor(sel, row, col) {
  return (
    sel.cursor !== null && sel.cursor.row === row && sel.cursor.col === col
  );
}

/**
 * Which sides of a cell lie on the range's boundary.
 *
 * The range is drawn as a single marquee, so each cell contributes only its
 * outer sides. Giving every selected cell a full border instead would draw a
 * grid of little boxes, which is not what a selected region looks like.
 *
 * @param {Bounds | null} bounds
 * @param {number} row
 * @param {number} col
 * @returns {Edges}
 */
export function edgesOf(bounds, row, col) {
  if (bounds === null) return NO_EDGES;
  const { r0, r1, c0, c1 } = bounds;
  if (row < r0 || row > r1 || col < c0 || col > c1) return NO_EDGES;
  return {
    top: row === r0,
    bottom: row === r1,
    left: col === c0,
    right: col === c1,
  };
}

/**
 * The selected region as tab-separated text, which is what every spreadsheet
 * and SQL console pastes correctly. A single cell copies as its bare value,
 * with no trailing tab or newline, so pasting one id into a query works.
 *
 * `colIndexes` maps visible column position to its index in the row tuple, so
 * a copy follows what the user sees rather than the query's column order.
 *
 * @param {readonly unknown[][]} rows
 * @param {readonly number[]} colIndexes
 * @param {Bounds | null} bounds
 * @returns {string}
 */
export function selectionToTsv(rows, colIndexes, bounds) {
  if (bounds === null) return '';
  const { r0, r1, c0, c1 } = bounds;
  const lines = [];
  for (let r = r0; r <= r1; r++) {
    const row = rows[r];
    // A range can outrun what is loaded when rows stream in behind it.
    if (row === undefined) continue;
    const cells = [];
    for (let c = c0; c <= c1; c++) {
      const index = colIndexes[c];
      cells.push(index === undefined ? '' : formatCell(row[index]));
    }
    lines.push(cells.join('\t'));
  }
  return lines.join('\n');
}
