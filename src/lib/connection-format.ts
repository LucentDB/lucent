import type { ConnectionProfile } from './stores/connections.svelte';

/**
 * Driver-aware one-line description of a connection profile.
 *
 * Host-based drivers (Postgres) read host/port/user/database from the profile
 * params; file-based drivers (DuckDB) describe themselves by path.
 */
export function connectionSubtitle(
  profile: Pick<ConnectionProfile, 'driver' | 'params'>,
): string {
  if (profile.driver === 'duckdb') {
    return profile.params['path'] ?? 'In-memory database';
  }
  const user = profile.params['user'] ?? 'postgres';
  const host = profile.params['host'] ?? '';
  const port = profile.params['port'] ?? '5432';
  const database = profile.params['database'] ?? '';
  return `${user}@${host}:${port}/${database}`;
}

/** Compact endpoint for tight spaces (e.g. the sidebar connection switcher). */
export function connectionEndpoint(
  profile: Pick<ConnectionProfile, 'driver' | 'params'>,
): string {
  if (profile.driver === 'duckdb') {
    return profile.params['path'] ?? 'In-memory';
  }
  const host = profile.params['host'] ?? '';
  const port = profile.params['port'] ?? '5432';
  return `${host}:${port}`;
}
