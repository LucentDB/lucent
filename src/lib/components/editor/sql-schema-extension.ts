import { LanguageSupport } from '@codemirror/language';
import { PostgreSQL, StandardSQL } from '@codemirror/lang-sql';
import type { SQLDialect } from '@codemirror/lang-sql';
import { clauseKeywordSource } from './sql-clause';
import { schemaColumnSource } from './sql-scope';

export interface EditorColumnInput {
  name: string;
  type_name: string;
}

export interface EditorTableInput {
  schema: string;
  name: string;
  columns: EditorColumnInput[];
}

const DIALECT_MAP: Record<string, SQLDialect> = {
  postgresql: PostgreSQL,
  duckdb: PostgreSQL,
  bigquery: StandardSQL,
};

export function dialectFor(sqlDialect: string | null | undefined): SQLDialect {
  return DIALECT_MAP[sqlDialect ?? 'postgresql'] ?? PostgreSQL;
}

export function buildNamespace(
  tables: EditorTableInput[],
): Record<string, Record<string, string[]>> {
  const namespace: Record<string, Record<string, string[]>> = {};
  for (const table of tables) {
    const schemaKey = table.schema || 'public';
    namespace[schemaKey] ??= {};
    namespace[schemaKey][table.name] = table.columns.map((c) => c.name);
  }
  return namespace;
}

export function buildSqlExtension(options: {
  tables: EditorTableInput[];
  sqlDialect: string | null | undefined;
  defaultSchema?: string;
}) {
  const dialect = dialectFor(options.sqlDialect);
  const namespace = buildNamespace(options.tables);
  return new LanguageSupport(dialect.language, [
    dialect.language.data.of({
      autocomplete: schemaColumnSource({
        namespace,
        defaultSchema: options.defaultSchema || 'public',
        dialect,
      }),
    }),
    dialect.language.data.of({ autocomplete: clauseKeywordSource(dialect) }),
  ]);
}
