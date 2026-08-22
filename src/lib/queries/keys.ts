// Every query key in the app is built here. Inline key literals in components
// are how prefix invalidation silently stops matching, so there are none.

/** The three inputs that change which history rows the backend returns. */
export interface HistoryFilter {
  connectionId: string | null;
  search: string | null;
  favoriteOnly: boolean;
}

export const qk = {
  /** Driver descriptors. Static per build. */
  drivers: () => ['drivers'] as const,

  /** Saved connection profiles. */
  connections: () => ['connections'] as const,

  /** Query history for one filter combination. */
  history: (filter: HistoryFilter) => ['history', filter] as const,

  /**
   * Prefix for every catalog branch of one connection. Invalidating this
   * refetches all mounted branches and leaves unmounted ones stale-but-cached.
   * `conn` is the active profile id, or 'inline' for a profile-less connection.
   */
  explorer: (conn: string) => ['explorer', conn] as const,

  databases: (conn: string) => ['explorer', conn, 'databases'] as const,

  schemas: (conn: string) => ['explorer', conn, 'schemas'] as const,

  /** Keyed by namespace PATH, not display name — multi-segment drivers (DuckDB). */
  objects: (conn: string, path: unknown) =>
    ['explorer', conn, 'objects', path] as const,
};
