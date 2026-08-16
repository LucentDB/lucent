import { describe, it, expect } from 'vitest';
import { classifyClause } from './sql-clause.ts';
import { buildSqlExtension } from './sql-schema-extension.ts';
import { FIXTURE_TABLES, suggestionsAt } from './completion-probe.ts';

const extension = () => [
  buildSqlExtension({ tables: FIXTURE_TABLES, sqlDialect: 'postgresql' }),
];

describe('classifyClause', () => {
  it('classifies statement start and partial input', () => {
    expect(classifyClause('')).toBe('statement-start');
    expect(classifyClause('sel')).toBe('statement-start');
    expect(classifyClause('whe')).toBe('statement-start');
  });

  it('classifies after each clause keyword', () => {
    expect(classifyClause('select ')).toBe('select');
    expect(classifyClause('select * from ')).toBe('from');
    expect(classifyClause('select * from users where ')).toBe('where');
    expect(classifyClause('select * from users where id = 1 and ')).toBe(
      'where',
    );
    expect(classifyClause('select * from users having ')).toBe('having');
    expect(classifyClause('select * from users group by ')).toBe('group-by');
    expect(classifyClause('select * from users order by ')).toBe('order-by');
    expect(classifyClause('select * from users limit ')).toBe('limit');
    expect(classifyClause('update users set ')).toBe('set');
  });

  it('tracks CASE depth', () => {
    expect(classifyClause('select case ')).toBe('case');
    expect(classifyClause('select case when x > 1 ')).toBe('case-when');
    expect(classifyClause('select case when x > 1 then ')).toBe('case-then');
    expect(classifyClause('select case when x > 1 then 1 else ')).toBe(
      'case-else',
    );
    expect(classifyClause('select case when x > 1 then 1 end ')).toBe('select');
  });

  it('pins nested-CASE behavior: the inner end exits CASE state early', () => {
    // Documented current behavior of the token-scan heuristic (not a claim of
    // correctness): the inner `end` flips `inCase` off, so the outer `else` /
    // `end` are ignored and the clause collapses back to `select`. Pinning this
    // so a future classifier rewrite must consciously change it.
    expect(
      classifyClause(
        'select case when x then case when y then 1 else 2 end else 3 end ',
      ),
    ).toBe('select');
  });

  it('resets to select after set-ops', () => {
    expect(classifyClause('select 1 union ')).toBe('select');
    expect(classifyClause('select 1 intersect ')).toBe('select');
  });
});

describe('clauseKeywordSource (via buildSqlExtension)', () => {
  it('never suggests when/whenever for a whe prefix', () => {
    const got = suggestionsAt(extension(), 'select * from users where whe', 29);
    expect(got).not.toContain('when');
    expect(got).not.toContain('whenever');
  });

  it('suggests where after FROM', () => {
    const got = suggestionsAt(extension(), 'select * from users whe', 23);
    expect(got).toContain('where');
    expect(got).not.toContain('when');
  });

  it('filters statement-start noise', () => {
    const got = suggestionsAt(extension(), 'sel', 3);
    expect(got).toContain('select');
    expect(got).not.toContain('selective');
    expect(got).not.toContain('self');
  });

  it('suggests only expression keywords in WHERE', () => {
    const got = suggestionsAt(
      extension(),
      'select * from users where id is n',
      33,
    );
    expect(got).toContain('null');
    expect(got).toContain('not');
    expect(got).not.toContain('where');
    expect(got).not.toContain('select');
  });

  it('suggests when/then/else/end only inside CASE', () => {
    const inCase = suggestionsAt(
      extension(),
      'select case when x then 1 e',
      27,
    );
    expect(inCase).toContain('end');
    expect(inCase).not.toContain('where');
    const outside = suggestionsAt(extension(), 'select e', 7);
    expect(outside).not.toContain('end');
  });

  it('falls back to curated keywords for dialects with an empty spec (bigquery/StandardSQL)', () => {
    // StandardSQL.spec is {} — no keywords/types/builtin — so the dialect-word
    // intersection must fall back to the curated clause lists instead of
    // collapsing to nothing. Regression: bigquery lost all keyword suggestions.
    const bigquery = () => [
      buildSqlExtension({ tables: FIXTURE_TABLES, sqlDialect: 'bigquery' }),
    ];
    expect(suggestionsAt(bigquery(), 'sel', 3)).toContain('select');
    expect(suggestionsAt(bigquery(), 'select * from users whe', 23)).toContain(
      'where',
    );
  });
});
