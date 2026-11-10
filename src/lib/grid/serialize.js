// Clipboard serialisation for selected cells and rows. Pure and
// dependency-free apart from formatCell, so what lands on the clipboard is
// exactly what the grid rendered.
import { formatCell } from './format.js';

const NEEDS_QUOTING = /[",\r\n]/;
const FORMULA_TRIGGERS = /^[ \t\r\n]*[=+\-@\t\r\n|]/;

/**
 * Neutralize formula trigger characters (=, +, -, @, \t, \r, \n, |) to prevent CWE-1236
 * spreadsheet formula injection (CSV/TSV injection), excluding valid numbers.
 */
function neutralizeFormula(value) {
  const text = formatCell(value);
  if (typeof value !== 'number' && FORMULA_TRIGGERS.test(text)) {
    return `'${text}`;
  }
  return text;
}

/** Tab-separated. The default: it pastes cleanly into every spreadsheet. */
export function toTsv(rows) {
  return rows.map((row) => row.map(neutralizeFormula).join('\t')).join('\n');
}

/** RFC 4180: quote when the field contains a comma, quote, CR or LF. */
function csvField(value) {
  const text = neutralizeFormula(value);
  if (!NEEDS_QUOTING.test(text)) return text;
  return `"${text.replace(/"/g, '""')}"`;
}

export function toCsv(rows) {
  return rows.map((row) => row.map(csvField).join(',')).join('\n');
}
