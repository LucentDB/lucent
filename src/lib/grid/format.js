// Cell display helpers. Deliberately dependency-free: no Svelte, no DOM, so
// both the grid body and its tests can use them without mounting anything.

/**
 * Render one cell for display.
 *
 * Numbers are rendered verbatim — NO locale separators. This is a database
 * client: users select and copy these values, and `4,200,000,000,000` is not
 * a number anyone can paste back into a query. `String(n)` also renders
 * floats at their shortest round-trippable form, which is what we want.
 */
export function formatCell(value) {
  if (value === null || value === undefined) return '';
  if (typeof value === 'boolean') return String(value);
  if (typeof value === 'number') return String(value);
  return String(value);
}

/** The CSS class that styles one cell by its value's type. */
export function cellClass(value) {
  if (value === null || value === undefined) return 'cell-null';
  if (typeof value === 'boolean') return 'cell-bool';
  if (typeof value === 'number') return 'cell-number';
  return '';
}
