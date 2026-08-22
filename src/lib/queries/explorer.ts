import {
  createQuery,
  queryOptions,
  type QueryClient,
} from '@tanstack/svelte-query';
import { getDatabases, getSchemas, getSchemaObjects } from '../ipc/client.js';
import { qk } from './keys.ts';

export interface ExplorerDatabase {
  name: string;
  is_current: boolean;
  [key: string]: unknown;
}

export interface ExplorerSchema {
  name: string;
  path: unknown;
  [key: string]: unknown;
}

export function databasesOptions(conn: string) {
  return queryOptions({
    queryKey: qk.databases(conn),
    queryFn: () => getDatabases() as Promise<ExplorerDatabase[]>,
  });
}

/**
 * The catalog IPC is scoped to the active connection, so one schemas request
 * describes the current database tree even when the driver reports several
 * database labels. `enabled` is false until a database branch is expanded.
 */
export function schemasOptions(conn: string, enabled: boolean) {
  return queryOptions({
    queryKey: qk.schemas(conn),
    queryFn: () => getSchemas() as Promise<ExplorerSchema[]>,
    enabled,
  });
}

/**
 * Keyed by namespace PATH, not display name — a dotted display name would be
 * misread as a single segment by multi-segment drivers (DuckDB).
 */
export function objectsOptions(conn: string, path: unknown, enabled: boolean) {
  return queryOptions({
    queryKey: qk.objects(conn, path),
    queryFn: async () => {
      const result = (await getSchemaObjects(path)) as { objects: unknown[] };
      return result.objects;
    },
    enabled,
  });
}

// ─── Component-facing constructors (getters, so branches re-key reactively) ──

export function databasesQuery(getConn: () => string) {
  return createQuery(() => databasesOptions(getConn()));
}

export function schemasQuery(getConn: () => string, getEnabled: () => boolean) {
  return createQuery(() => schemasOptions(getConn(), getEnabled()));
}

export function objectsQuery(
  getConn: () => string,
  getPath: () => unknown,
  getEnabled: () => boolean,
) {
  return createQuery(() => objectsOptions(getConn(), getPath(), getEnabled()));
}

/**
 * The whole refresh button. Prefix invalidation refetches every mounted branch
 * and leaves unmounted ones stale-but-cached; expansion state is untouched, so
 * the tree never collapses.
 */
export function refreshExplorer(client: QueryClient, conn: string) {
  return client.invalidateQueries({ queryKey: qk.explorer(conn) });
}
