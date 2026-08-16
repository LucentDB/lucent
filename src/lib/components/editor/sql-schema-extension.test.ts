import { describe, it, expect } from 'vitest';
import {
  dialectFor,
  buildNamespace,
  buildSqlExtension,
} from './sql-schema-extension.ts';
import { PostgreSQL, StandardSQL } from '@codemirror/lang-sql';

describe('dialectFor', () => {
  it('maps postgresql to PostgreSQL', () => {
    expect(dialectFor('postgresql')).toBe(PostgreSQL);
  });

  it('maps duckdb to PostgreSQL (documented Postgres-syntax superset)', () => {
    expect(dialectFor('duckdb')).toBe(PostgreSQL);
  });

  it('maps bigquery to StandardSQL (no dedicated BigQuery dialect ships)', () => {
    expect(dialectFor('bigquery')).toBe(StandardSQL);
  });

  it('falls back to PostgreSQL for unknown or missing dialect strings', () => {
    expect(dialectFor('some-future-dialect')).toBe(PostgreSQL);
    expect(dialectFor(null)).toBe(PostgreSQL);
    expect(dialectFor(undefined)).toBe(PostgreSQL);
  });
});

describe('buildNamespace', () => {
  it('groups columns under schema then table', () => {
    const ns = buildNamespace([
      {
        schema: 'public',
        name: 'customers',
        columns: [
          { name: 'id', type_name: 'int4' },
          { name: 'name', type_name: 'text' },
        ],
      },
    ]);
    expect(ns).toEqual({ public: { customers: ['id', 'name'] } });
  });

  it('groups multiple tables under the same schema', () => {
    const ns = buildNamespace([
      {
        schema: 'public',
        name: 'a',
        columns: [{ name: 'x', type_name: 'int4' }],
      },
      {
        schema: 'public',
        name: 'b',
        columns: [{ name: 'y', type_name: 'int4' }],
      },
    ]);
    expect(Object.keys(ns.public)).toEqual(['a', 'b']);
  });

  it('returns an empty object for an empty table list, never null/undefined', () => {
    expect(buildNamespace([])).toEqual({});
  });

  it('never crashes on a table with zero columns', () => {
    const ns = buildNamespace([
      { schema: 'public', name: 'empty_view', columns: [] },
    ]);
    expect(ns).toEqual({ public: { empty_view: [] } });
  });

  it('falls back to the "public" schema key when schema is an empty string', () => {
    const ns = buildNamespace([{ schema: '', name: 'orphan', columns: [] }]);
    expect(ns).toEqual({ public: { orphan: [] } });
  });
});

describe('buildSqlExtension', () => {
  it('returns a usable CodeMirror extension without throwing for an empty schema', () => {
    const ext = buildSqlExtension({ tables: [], sqlDialect: 'postgresql' });
    expect(ext).toBeTruthy();
  });

  it('returns a usable extension for a populated schema', () => {
    const ext = buildSqlExtension({
      tables: [
        {
          schema: 'public',
          name: 'customers',
          columns: [{ name: 'id', type_name: 'int4' }],
        },
      ],
      sqlDialect: 'postgresql',
      defaultSchema: 'public',
    });
    expect(ext).toBeTruthy();
  });
});
