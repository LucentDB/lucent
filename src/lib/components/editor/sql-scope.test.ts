import { describe, it, expect } from 'vitest';
import { EditorState } from '@codemirror/state';
import { CompletionContext } from '@codemirror/autocomplete';
import { PostgreSQL } from '@codemirror/lang-sql';
import { tablesInScope, schemaColumnSource } from './sql-scope.ts';
import { buildNamespace } from './sql-schema-extension.ts';
import { buildSqlExtension } from './sql-schema-extension.ts';
import { FIXTURE_TABLES, suggestionsAt } from './completion-probe.ts';

const namespace = buildNamespace(FIXTURE_TABLES);

describe('tablesInScope', () => {
  function refs(doc: string, pos: number) {
    // The SQL language must be installed for syntaxTree(state) to contain a
    // Statement node — production always has it via buildSqlExtension.
    const state = EditorState.create({
      doc,
      extensions: [
        buildSqlExtension({ tables: FIXTURE_TABLES, sqlDialect: 'postgresql' }),
      ],
    });
    return tablesInScope(state, pos);
  }

  it('finds a schema-qualified table without an alias', () => {
    const doc = 'select timezone from bookings.airports_data';
    expect(refs(doc, doc.length)).toEqual([
      { path: ['bookings', 'airports_data'] },
    ]);
  });

  it('finds a table and its alias', () => {
    const doc = 'select * from bookings.airports_data ad where x = 1';
    expect(refs(doc, doc.length)).toEqual([
      { path: ['bookings', 'airports_data'], alias: 'ad' },
    ]);
  });

  it('finds comma-separated tables', () => {
    const doc = 'select * from users, customers where x = 1';
    expect(refs(doc, doc.length)).toEqual([
      { path: ['users'] },
      { path: ['customers'] },
    ]);
  });

  it('finds joined tables and skips ON-condition identifiers', () => {
    const doc = 'select * from users join customers on users.id = customers.id';
    expect(refs(doc, doc.length)).toEqual([
      { path: ['users'] },
      { path: ['customers'] },
    ]);
  });

  it('stops at where', () => {
    const doc =
      'select * from users where id = (select max(id) from customers)';
    expect(refs(doc, doc.length)).toEqual([{ path: ['users'] }]);
  });

  it('does not treat subquery aliases as tables', () => {
    const doc = 'select * from (select 1) s where x = 1';
    expect(refs(doc, doc.length)).toEqual([]);
  });

  it('finds tables even when the cursor is before the FROM clause', () => {
    const doc = 'select timezone from bookings.airports_data';
    expect(refs(doc, 12)).toEqual([{ path: ['bookings', 'airports_data'] }]);
  });

  it('scans only the statement containing the cursor', () => {
    const doc = 'select 1; select * from users where x = 1';
    expect(refs(doc, doc.indexOf('users'))).toEqual([{ path: ['users'] }]);
  });
});

describe('schemaColumnSource (in-scope columns)', () => {
  const extension = () => [
    buildSqlExtension({ tables: FIXTURE_TABLES, sqlDialect: 'postgresql' }),
  ];

  it('suggests columns in the SELECT list with a qualified FROM and no alias', () => {
    const got = suggestionsAt(
      extension(),
      'select timez from bookings.airports_data',
      12,
    );
    expect(got).toContain('timezone');
  });

  it('suggests columns in WHERE with an unqualified table', () => {
    const got = suggestionsAt(extension(), 'select * from users where em', 28);
    expect(got).toContain('email');
  });

  it('qualifies ambiguous column names across in-scope tables', () => {
    const got = suggestionsAt(
      extension(),
      'select * from users, customers where i',
      38,
    );
    expect(got).toContain('users.id');
    expect(got).toContain('customers.id');
    expect(got).toContain('email');
  });

  it('keeps alias-dot behavior unchanged', () => {
    const got = suggestionsAt(
      extension(),
      'select * from bookings.airports_data ad where ad.',
      49,
    );
    expect(got).toEqual(
      expect.arrayContaining(['airport_code', 'timezone', 'city']),
    );
  });

  it('keeps schema.table. behavior unchanged', () => {
    const got = suggestionsAt(
      extension(),
      'select bookings.airports_data.',
      30,
    );
    expect(got).toEqual(
      expect.arrayContaining(['airport_code', 'timezone', 'city']),
    );
  });

  it('suggests nothing column-ish for a subquery alias', () => {
    const got = suggestionsAt(
      extension(),
      'select * from (select 1) s where em',
      35,
    );
    expect(got).not.toContain('email');
  });

  it('works across statements', () => {
    const got = suggestionsAt(
      extension(),
      'select 1; select * from users where em',
      38,
    );
    expect(got).toContain('email');
  });
});

describe('schemaColumnSource direct', () => {
  it('delegates to lang-sql after a dot', () => {
    const source = schemaColumnSource({
      namespace,
      dialect: PostgreSQL,
      defaultSchema: 'public',
    });
    const state = EditorState.create({
      doc: 'select * from users u where u.',
      extensions: [
        buildSqlExtension({ tables: FIXTURE_TABLES, sqlDialect: 'postgresql' }),
      ],
    });
    const ctx = new CompletionContext(state, state.doc.length, false);
    const r = source(ctx);
    const labels = r && 'options' in r ? r.options.map((o) => o.label) : [];
    expect(labels).toEqual(expect.arrayContaining(['id', 'email']));
  });
});
