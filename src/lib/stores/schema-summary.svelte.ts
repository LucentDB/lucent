// A lightweight, cached summary of the connected database's tables, used only
// to ground the AI landing screen's suggestion chips in real table names.
// Deliberately not a general schema store — the sidebar owns the full lazily
// loaded tree. This is a couple of cheap queries, cached until disconnect.

import { getSchemas, getSchemaObjects } from '../ipc/client.js';
import type { SchemaTable } from '../components/chat/suggestions.ts';

/**
 * Tried first when present. Most databases put their tables here, but plenty
 * don't — so this is a preference, not an assumption (see pickSchemas).
 */
const PREFERRED_SCHEMA = 'public';

/**
 * How many schemas to probe before giving up. Guards against a database with
 * dozens of empty schemas turning a landing screen into a query storm.
 */
const MAX_SCHEMA_PROBES = 3;

interface SchemaInfo {
  name: string;
  /** Tables + views + functions + sequences. Snake_case on the wire. */
  object_count: number;
}

/**
 * Schemas worth probing, best candidate first: the preferred schema, then the
 * most populated ones. Empty schemas are skipped entirely — probing them can
 * only return nothing.
 */
export function pickSchemas(schemas: SchemaInfo[]): string[] {
  const populated = schemas.filter((s) => (s.object_count ?? 0) > 0);
  const preferred = populated.filter((s) => s.name === PREFERRED_SCHEMA);
  const rest = populated
    .filter((s) => s.name !== PREFERRED_SCHEMA)
    .sort((a, b) => (b.object_count ?? 0) - (a.object_count ?? 0));
  return [...preferred, ...rest].slice(0, MAX_SCHEMA_PROBES).map((s) => s.name);
}

function tablesFrom(result: unknown): SchemaTable[] {
  const objects =
    (
      result as {
        objects?: { name: string; kind: string; row_count: unknown }[];
      }
    )?.objects ?? [];
  return objects
    .filter((o) => o.kind === 'table')
    .map((o) => ({
      name: o.name,
      // The wire sends an i64; anything non-numeric becomes null rather than
      // flowing downstream to render as NaN.
      rowCount: typeof o.row_count === 'number' ? o.row_count : null,
    }));
}

class SchemaSummaryStore {
  tables = $state<SchemaTable[]>([]);
  /** The schema the cached tables actually came from. */
  schema = $state<string | null>(null);
  /**
   * Which database the cached tables belong to. Suggestions naming tables
   * from a database you're no longer connected to would be worse than
   * generic ones, so this is part of the cache key.
   */
  database = $state<string | null>(null);
  loading = $state(false);
  /** True once a load has settled, whether it succeeded or not. */
  loaded = $state(false);

  /**
   * Finds a schema with tables in `database` and caches its table names and
   * row estimates. A no-op when that database is already loaded or a load is
   * in flight, so callers can invoke it freely — including from an effect that
   * re-runs on connection changes.
   *
   * Failures are swallowed on purpose: this only powers suggestion chips,
   * which fall back to generic prompts when `tables` is empty. A landing
   * screen must not surface an error for a decorative feature.
   */
  async load(database: string | null): Promise<void> {
    if (this.loading) return;
    if (this.loaded && this.database === database) return;

    this.loading = true;
    let tables: SchemaTable[] = [];
    let schema: string | null = null;

    try {
      const candidates = pickSchemas(await getSchemas());
      for (const name of candidates) {
        const found = tablesFrom(await getSchemaObjects(name));
        if (found.length > 0) {
          tables = found;
          schema = name;
          break;
        }
      }
    } catch {
      // Leave `tables` empty — buildSuggestions() handles that case.
    } finally {
      this.tables = tables;
      this.schema = schema;
      this.database = database;
      this.loading = false;
      this.loaded = true;
    }
  }

  /** Clears the cache so the next connection re-reads its own schema. */
  reset(): void {
    this.tables = [];
    this.schema = null;
    this.database = null;
    this.loading = false;
    this.loaded = false;
  }
}

export const schemaSummary = new SchemaSummaryStore();
