import { describe, it, expect, vi, beforeEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  Channel: class {},
}));

import { QueryClient } from '@tanstack/svelte-query';
import { connectionsOptions, saveConnection, deleteConnection } from './connections.ts';
import { qk } from './keys.ts';

const PROFILE = {
  id: 'p1',
  name: 'local',
  driver: 'postgres',
  alias: null,
  params: { host: 'localhost' },
  sshTunnelId: null,
  group: null,
  color: null,
  icon: null,
  lastUsed: null,
  createdAt: '2026-01-01',
  updatedAt: '2026-01-01',
};

function client() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } });
}

/** Counts calls to one IPC command, so a test can assert a refetch happened. */
function callsTo(cmd: string) {
  return invoke.mock.calls.filter(([c]) => c === cmd).length;
}

beforeEach(() => {
  invoke.mockReset();
});

describe('connectionsOptions', () => {
  it('lists profiles through list_connections', async () => {
    invoke.mockResolvedValue([PROFILE]);
    const result = await client().fetchQuery(connectionsOptions());
    expect(result).toEqual([PROFILE]);
    expect(callsTo('list_connections')).toBe(1);
  });
});

describe('save/delete invalidation cascade', () => {
  it('save_connection marks the connection list stale', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_connections') return [PROFILE];
      if (cmd === 'save_connection') return PROFILE;
      throw new Error(`unexpected ${cmd}`);
    });
    const c = client();
    await c.fetchQuery(connectionsOptions());
    expect(callsTo('list_connections')).toBe(1);

    await saveConnection(c, { profile: PROFILE, password: null });

    // Invalidation marks the entry stale; the next read refetches instead of
    // serving the pre-save list.
    await c.fetchQuery(connectionsOptions());
    expect(callsTo('list_connections')).toBe(2);
  });

  it('delete_connection marks the connection list stale', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_connections') return [];
      if (cmd === 'delete_connection') return null;
      throw new Error(`unexpected ${cmd}`);
    });
    const c = client();
    await c.fetchQuery(connectionsOptions());
    await deleteConnection(c, { id: 'p1' });
    await c.fetchQuery(connectionsOptions());
    expect(callsTo('list_connections')).toBe(2);
  });

  it('a failed save does not invalidate the list', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_connections') return [PROFILE];
      if (cmd === 'save_connection') throw 'duplicate name';
      throw new Error(`unexpected ${cmd}`);
    });
    const c = client();
    await c.fetchQuery(connectionsOptions());
    await expect(saveConnection(c, { profile: PROFILE })).rejects.toBeTruthy();
    expect(c.getQueryState(qk.connections())?.isInvalidated).toBe(false);
  });
});
