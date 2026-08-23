import { describe, it, expect, vi, beforeEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  Channel: class {},
}));

import { QueryClient } from '@tanstack/svelte-query';
import { driversOptions } from './drivers.ts';

const DRIVERS = [
  { id: 'postgres', displayName: 'PostgreSQL', fields: [], hasSecret: true },
  { id: 'duckdb', displayName: 'DuckDB', fields: [], hasSecret: false },
];

/** A fresh client per test so one test's cache cannot satisfy another's fetch. */
function client() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
}

beforeEach(() => {
  invoke.mockReset();
});

describe('driversOptions', () => {
  it('fetches the driver list through list_drivers', async () => {
    invoke.mockResolvedValue(DRIVERS);
    const result = await client().fetchQuery(driversOptions());
    expect(result).toEqual(DRIVERS);
    expect(invoke).toHaveBeenCalledWith('list_drivers', undefined);
  });

  it('serves a second read from cache — drivers are static per build', async () => {
    invoke.mockResolvedValue(DRIVERS);
    const c = client();
    await c.fetchQuery(driversOptions());
    await c.fetchQuery(driversOptions());
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it('propagates a failure instead of swallowing it', async () => {
    invoke.mockRejectedValue('driver registry unavailable');
    await expect(client().fetchQuery(driversOptions())).rejects.toBeTruthy();
  });
});
