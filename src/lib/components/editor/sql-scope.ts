import { syntaxTree } from '@codemirror/language';
import type { EditorState, Text } from '@codemirror/state';
import { schemaCompletionSource } from '@codemirror/lang-sql';
import type { SQLDialect } from '@codemirror/lang-sql';
import type {
  Completion,
  CompletionContext,
  CompletionSource,
} from '@codemirror/autocomplete';
import { classifyClause, type SqlClause } from './sql-clause';

export interface TableRef {
  /** [schema, table] for qualified refs, [table] otherwise. */
  path: string[];
  /** Present when the table was aliased (FROM s.t a → alias "a"). */
  alias?: string;
}

/** Clause keywords that terminate the FROM/JOIN table list. */
const END_CLAUSE = new Set([
  'where',
  'group',
  'having',
  'order',
  'limit',
  'offset',
  'union',
  'intersect',
  'except',
  'using',
  'set',
  'values',
  'returning',
  'fetch',
  'for',
]);
const JOIN_KEYWORDS = new Set([
  'join',
  'inner',
  'left',
  'right',
  'full',
  'cross',
  'natural',
]);

function idText(doc: Text, node: { from: number; to: number }): string {
  return doc.sliceString(node.from, node.to);
}

function pathFor(doc: Text, node: any): string[] | null {
  if (node.name === 'CompositeIdentifier') {
    const path: string[] = [];
    for (let ch = node.firstChild; ch; ch = ch.nextSibling) {
      if (ch.name === 'Identifier') path.push(idText(doc, ch));
    }
    return path.length ? path : null;
  }
  if (node.name === 'Identifier' || node.name === 'QuotedIdentifier') {
    return [idText(doc, node)];
  }
  return null;
}

/**
 * Tables referenced in the Statement containing `at`. Scans the WHOLE
 * statement — not just up to the cursor — so `select timezone from
 * bookings.airports_data` resolves while the cursor is still in the SELECT
 * list. Known v2 limitations (documented, not handled): CTE names and
 * subquery aliases resolve to nothing (lookup fails against the schema);
 * quoted identifiers do not match the schema dictionary.
 */
export function tablesInScope(state: EditorState, at: number): TableRef[] {
  const tree = syntaxTree(state);
  const doc = state.doc;
  const node = tree.resolveInner(at, -1);
  let stmt: any = null;
  for (let p: typeof node | null = node; p; p = p.parent) {
    if (p.name === 'Statement') {
      stmt = p;
      break;
    }
  }
  if (!stmt) return [];

  const refs: TableRef[] = [];
  let sawFrom = false;
  let prevId: any = null; // pending table ref — may gain an alias next token
  let inOnCondition = false;
  let skipNextId = false;

  // Register a pending table as a bare (unalised) ref. Registration is
  // deferred until we know the next token is not its alias — an aliased
  // table is recorded ONCE, keyed by the alias, never also by its own name.
  const flush = () => {
    if (prevId) {
      const path = pathFor(doc, prevId);
      if (path) refs.push({ path });
      prevId = null;
    }
  };

  for (let child = stmt.firstChild; child; child = child.nextSibling) {
    const name = child.name;
    const kw =
      name === 'Keyword'
        ? doc.sliceString(child.from, child.to).toLowerCase()
        : null;

    if (kw) {
      if (!sawFrom && kw === 'from') {
        sawFrom = true;
        continue;
      }
      if (sawFrom) {
        if (END_CLAUSE.has(kw)) {
          flush();
          break;
        }
        if (kw === 'on') {
          flush();
          inOnCondition = true;
          continue;
        }
        if (JOIN_KEYWORDS.has(kw) || kw === 'join') {
          flush();
          inOnCondition = false;
          continue;
        }
        if (kw === 'as') continue; // keep prevId → alias resolved below
      }
    }

    if (name === 'Parens') {
      flush();
      skipNextId = true;
      continue;
    }

    const isId =
      name === 'Identifier' ||
      name === 'QuotedIdentifier' ||
      name === 'CompositeIdentifier';

    if (sawFrom && isId && !inOnCondition) {
      if (skipNextId) {
        skipNextId = false;
        flush();
        continue;
      }
      if (prevId) {
        // prevId is a table, this identifier is its alias.
        const path = pathFor(doc, prevId);
        if (path) refs.push({ path, alias: idText(doc, child) });
        prevId = null;
      } else {
        prevId = child; // pending table — may gain an alias next token
      }
    } else if (!isId && name !== 'Keyword') {
      flush();
    }
  }
  flush(); // trailing pending table at end of the statement scan
  return refs;
}

/** Clauses where bare column names are valid expression positions. */
const COLUMN_CLAUSES: ReadonlySet<SqlClause> = new Set([
  'select',
  'where',
  'having',
  'on',
  'order-by',
  'group-by',
  'case',
  'case-when',
  'case-else',
]);

interface ResolvedColumn {
  name: string;
  tableLabel: string;
}

function resolveColumns(
  refs: TableRef[],
  namespace: Record<string, Record<string, string[]>>,
  defaultSchema: string,
): ResolvedColumn[] {
  const out: ResolvedColumn[] = [];
  for (const ref of refs) {
    const schema = ref.path.length > 1 ? ref.path[0] : defaultSchema;
    const table = ref.path[ref.path.length - 1];
    const columns = namespace[schema]?.[table];
    if (!columns) continue;
    const label = ref.alias ?? table;
    for (const name of columns) out.push({ name, tableLabel: label });
  }
  return out;
}

/**
 * lang-sql's schema completion (alias./table./schema.table. resolution),
 * augmented with in-scope columns for bare-identifier positions. Column
 * names unique across in-scope tables stay bare; ambiguous ones become
 * `table.column` (or `alias.column`). Falls back to lang-sql behavior for
 * dot positions, quoted identifiers, non-column clauses, and empty scope.
 */
export function schemaColumnSource(options: {
  namespace: Record<string, Record<string, string[]>>;
  dialect: SQLDialect;
  defaultSchema?: string;
}): CompletionSource {
  const defaultSchema = options.defaultSchema ?? 'public';
  const base = schemaCompletionSource({
    schema: options.namespace,
    defaultSchema,
    dialect: options.dialect,
  });
  return (context: CompletionContext) => {
    const before = context.state.doc.sliceString(0, context.pos);
    const beforeWord = /[.\w$"`\[]*$/.exec(before)?.[0] ?? '';
    const hasDot = beforeWord.includes('.');
    const quoted = /["`\[]$/.test(before) || /^[`'"\[]/.test(beforeWord);
    if (hasDot || quoted) return base(context);

    const clause = classifyClause(before);
    if (!COLUMN_CLAUSES.has(clause)) return base(context);

    const refs = tablesInScope(context.state, context.pos);
    if (!refs.length) return base(context);

    const resolved = resolveColumns(refs, options.namespace, defaultSchema);
    if (!resolved.length) return base(context);

    const counts = new Map<string, number>();
    for (const r of resolved) counts.set(r.name, (counts.get(r.name) ?? 0) + 1);

    const seen = new Set<string>();
    const columnOptions: Completion[] = [];
    for (const r of resolved) {
      const label =
        counts.get(r.name)! > 1 ? `${r.tableLabel}.${r.name}` : r.name;
      if (seen.has(label)) continue;
      seen.add(label);
      columnOptions.push({ label, type: 'property', boost: 5 });
    }

    const baseResult = base(context);
    // lang-sql's sources resolve synchronously; narrow past the Promise member
    // of the CompletionSource return type (same guard as completion-probe.ts).
    const baseOptions =
      baseResult && 'options' in baseResult ? baseResult.options : [];
    const merged = [
      ...columnOptions,
      ...baseOptions.filter((o) => !seen.has(o.label)),
    ];
    const match = context.matchBefore(/[\w$]*/);
    return {
      from: match?.from ?? context.pos,
      options: merged,
      validFor: /^\w*$/,
    };
  };
}
