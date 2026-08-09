import { sql } from '@codemirror/lang-sql';
import { classHighlighter, highlightTree } from '@lezer/highlight';

export type SqlToken = { text: string; cls: string };

/**
 * Statically highlights SQL for read-only display.
 *
 * Uses the same Lezer grammar the SQL editor is built on, so highlighting in a
 * read-only block and in the live editor cannot drift apart. Cheaper than
 * mounting a second CodeMirror instance just to colour text nobody can edit.
 *
 * The parser is built once at module scope: constructing the SQL language costs
 * real work and it holds no per-document state.
 */
const sqlLanguage = sql().language;

export function tokenizeSql(code: string): SqlToken[] {
  if (!code) return [];

  const tokens: SqlToken[] = [];
  let pos = 0;

  /** Emits code[pos, to) under `cls` and advances. */
  function emit(to: number, cls: string) {
    if (to <= pos) return;
    tokens.push({ text: code.slice(pos, to), cls });
    pos = to;
  }

  try {
    const tree = sqlLanguage.parser.parse(code);
    highlightTree(tree, classHighlighter, (from, to, classes) => {
      emit(from, ''); // whitespace and unclassified text between tokens
      emit(to, classes);
    });
  } catch (e) {
    // A grammar failure must not blank out the user's SQL.
    console.error('[notebook] SQL highlighting failed, showing plain text', e);
    return [{ text: code, cls: '' }];
  }

  emit(code.length, '');
  return tokens;
}
