// Clipboard serialisation for selected cells and rows. Pure and
// dependency-free apart from formatCell, so what lands on the clipboard is
// exactly what the grid rendered.
import { formatCell } from './format.js';

/** Tab-separated. The default: it pastes cleanly into every spreadsheet. */
export function toTsv(rows) {
  return rows.map((row) => row.map(formatCell).join('\t')).join('\n');
}

const NEEDS_QUOTING = /[",\r\n]/;
const FORMULA_TRIGGERS = /^[=+\-@\t\r]/;

/** RFC 4180: quote when the field contains a comma, quote, CR or LF.
 * Neutralize formula trigger characters (=, +, -, @, \t, \r) to prevent CWE-1236 CSV injection,
 * excluding valid numbers.
 */
function csvField(value) {
  let text = formatCell(value);
  if (FORMULA_TRIGGERS.test(text) && typeof value !== 'number') {
    text = `'${text}`;
  }
  if (!NEEDS_QUOTING.test(text)) return text;
  return `"${text.replace(/"/g, '""')}"`;
}

export function toCsv(rows) {
  return rows.map((row) => row.map(csvField).join(',')).join('\n');
}
