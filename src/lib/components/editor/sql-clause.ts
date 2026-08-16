import { completeFromList, ifNotIn } from '@codemirror/autocomplete';
import type {
  CompletionContext,
  CompletionSource,
} from '@codemirror/autocomplete';
import type { SQLDialect } from '@codemirror/lang-sql';

export type SqlClause =
  | 'statement-start'
  | 'select'
  | 'from'
  | 'join'
  | 'on'
  | 'where'
  | 'group-by'
  | 'having'
  | 'order-by'
  | 'limit'
  | 'case'
  | 'case-when'
  | 'case-then'
  | 'case-else'
  | 'set'
  | 'values';

/**
 * Keywords offered per clause. Curated for Postgres; intersected with the
 * dialect's own word list at runtime so other dialects degrade gracefully.
 * Deliberately small — typing `whe` in a WHERE clause suggests nothing (the
 * clause is already open), while `whe` after FROM suggests only `where`.
 */
export const CLAUSE_KEYWORDS: Record<SqlClause, string[]> = {
  'statement-start': [
    'select',
    'with',
    'insert',
    'update',
    'delete',
    'create',
    'alter',
    'drop',
    'truncate',
    'explain',
    'vacuum',
    'analyze',
    'set',
    'show',
    'begin',
    'commit',
    'rollback',
    'grant',
    'revoke',
    'copy',
    'call',
    'do',
    'refresh',
    'reindex',
    'reset',
    'values',
  ],
  select: [
    'from',
    'where',
    'group',
    'having',
    'order',
    'limit',
    'offset',
    'as',
    'distinct',
    'all',
    'union',
    'intersect',
    'except',
    'case',
    'when',
    'and',
    'or',
    'not',
    'in',
    'is',
    'null',
    'like',
    'ilike',
    'between',
    'exists',
    'any',
    'some',
    'cast',
    'fetch',
    'for',
    'window',
    'over',
  ],
  from: [
    'where',
    'join',
    'left',
    'right',
    'full',
    'inner',
    'outer',
    'cross',
    'natural',
    'lateral',
    'on',
    'using',
    'as',
    'group',
    'having',
    'order',
    'limit',
    'offset',
    'union',
    'intersect',
    'except',
    'tablesample',
  ],
  join: [
    'on',
    'using',
    'as',
    'and',
    'or',
    'where',
    'group',
    'having',
    'order',
    'limit',
  ],
  on: [
    'and',
    'or',
    'not',
    'in',
    'is',
    'null',
    'between',
    'like',
    'ilike',
    'case',
  ],
  where: [
    'and',
    'or',
    'not',
    'in',
    'is',
    'null',
    'like',
    'ilike',
    'similar',
    'between',
    'exists',
    'any',
    'all',
    'some',
    'case',
    'cast',
    'group',
    'having',
    'order',
    'limit',
    'offset',
    'union',
    'intersect',
    'except',
  ],
  'group-by': [
    'having',
    'order',
    'limit',
    'offset',
    'union',
    'intersect',
    'except',
    'asc',
    'desc',
    'nulls',
  ],
  having: [
    'and',
    'or',
    'not',
    'in',
    'is',
    'null',
    'like',
    'ilike',
    'between',
    'exists',
    'any',
    'all',
    'some',
    'case',
    'cast',
    'order',
    'limit',
    'offset',
  ],
  'order-by': [
    'asc',
    'desc',
    'nulls',
    'first',
    'last',
    'limit',
    'offset',
    'fetch',
    'collate',
    'case',
    'using',
  ],
  limit: ['offset', 'fetch', 'for'],
  case: ['when', 'then', 'else', 'end', 'case'],
  'case-when': [
    'then',
    'else',
    'end',
    'and',
    'or',
    'not',
    'in',
    'is',
    'null',
    'like',
    'ilike',
    'between',
    'exists',
    'any',
    'all',
    'some',
  ],
  'case-then': ['when', 'else', 'end'],
  'case-else': ['when', 'then', 'end'],
  set: ['where', 'returning', 'from'],
  values: ['returning', 'where'],
};

/**
 * Token-scan classifier: tracks the last clause keyword before the cursor,
 * CASE … END depth, and GROUP/ORDER BY two-word pairs. Heuristic, not a
 * parser — good enough to gate keyword suggestions.
 */
export function classifyClause(textBeforeCursor: string): SqlClause {
  const tokens = textBeforeCursor
    .toLowerCase()
    .replace(/[^a-z\s]/g, ' ')
    .split(/\s+/)
    .filter(Boolean);
  let clause: SqlClause = 'statement-start';
  let inCase = false;
  let afterCase: SqlClause = 'statement-start';
  let prev = '';
  for (const t of tokens) {
    if (inCase) {
      if (t === 'when') clause = 'case-when';
      else if (t === 'then') clause = 'case-then';
      else if (t === 'else') clause = 'case-else';
      else if (t === 'end') {
        inCase = false;
        clause = afterCase;
      }
      continue;
    }
    if (t === 'case') {
      inCase = true;
      afterCase = clause;
      clause = 'case';
      continue;
    }
    if (t === 'group' || t === 'order') {
      prev = t;
      continue;
    }
    if ((prev === 'group' || prev === 'order') && t === 'by') {
      clause = prev === 'group' ? 'group-by' : 'order-by';
      prev = '';
      continue;
    }
    prev = '';
    switch (t) {
      case 'select':
      case 'union':
      case 'intersect':
      case 'except':
        clause = 'select';
        break;
      case 'from':
        clause = 'from';
        break;
      case 'where':
        clause = 'where';
        break;
      case 'having':
        clause = 'having';
        break;
      case 'limit':
      case 'offset':
        clause = 'limit';
        break;
      case 'on':
        clause = 'on';
        break;
      case 'set':
        clause = 'set';
        break;
      case 'values':
        clause = 'values';
        break;
      case 'join':
        clause = 'join';
        break;
    }
  }
  return clause;
}

/**
 * Completion source replacing lang-sql's context-blind all-keyword source.
 * Same ifNotIn guard lang-sql uses (no suggestions in strings/comments/after
 * dots), but options are filtered to the current clause's curated list and
 * intersected with the dialect's actual word set. The word set mirrors
 * lang-sql's own `keywords()` composition (keywords + types + builtins + the
 * null/true/false/unknown constants) so curated keywords that lang-sql
 * classifies as builtin (e.g. `null`) survive the intersection. Dialects with
 * an empty spec (StandardSQL / bigquery) fall back to the union of the
 * curated clause lists instead of collapsing to nothing.
 */
export function clauseKeywordSource(dialect: SQLDialect): CompletionSource {
  const specWords = [
    ...(dialect.spec.keywords ?? '').split(' '),
    ...(dialect.spec.types ?? '').split(' '),
    ...(dialect.spec.builtin ?? '').split(' '),
  ].filter(Boolean);
  // StandardSQL (used for bigquery) defines no keywords/types/builtin — its
  // spec is empty — so the intersection would collapse to nothing. Fall back
  // to the union of the curated clause lists: every clause still suggests its
  // Postgres-flavored words, and the per-clause filter keeps them appropriate.
  const fallback =
    specWords.length === 0 ? Object.values(CLAUSE_KEYWORDS).flat() : [];
  const dialectWords = new Set([
    ...specWords,
    ...fallback,
    'true',
    'false',
    'null',
    'unknown',
  ]);
  return ifNotIn(
    ['QuotedIdentifier', 'String', 'LineComment', 'BlockComment', '.'],
    (context: CompletionContext) => {
      const clause = classifyClause(
        context.state.doc.sliceString(0, context.pos),
      );
      const options = (CLAUSE_KEYWORDS[clause] ?? [])
        .filter((k) => dialectWords.has(k))
        .map((label) => ({ label, type: 'keyword' as const, boost: -1 }));
      return completeFromList(options)(context);
    },
  );
}
