import { describe, expect, it, vi } from 'vitest';
import { fetchExplorerSnapshot } from './sidebar-refresh';

describe('fetchExplorerSnapshot', () => {
  it('refreshes every schema and object in the active explorer', async () => {
    const getDatabases = vi
      .fn()
      .mockResolvedValue([{ name: 'analytics', is_current: true }]);
    const getSchemas = vi.fn().mockResolvedValue([
      { name: 'analytics.main', path: ['analytics', 'main'] },
      { name: 'analytics.reporting', path: ['analytics', 'reporting'] },
    ]);
    const getSchemaObjects = vi
      .fn()
      .mockImplementation(async (path: string[]) => ({
        objects: [{ name: path.at(-1), kind: 'table' }],
      }));

    const snapshot = await fetchExplorerSnapshot({
      getDatabases,
      getSchemas,
      getSchemaObjects,
    });

    expect(getDatabases).toHaveBeenCalledOnce();
    expect(getSchemas).toHaveBeenCalledOnce();
    expect(getSchemaObjects).toHaveBeenCalledTimes(2);
    expect(getSchemaObjects).toHaveBeenCalledWith(['analytics', 'main']);
    expect(getSchemaObjects).toHaveBeenCalledWith(['analytics', 'reporting']);
    expect(snapshot.schemasByDb).toEqual({
      analytics: [
        { name: 'analytics.main', path: ['analytics', 'main'] },
        { name: 'analytics.reporting', path: ['analytics', 'reporting'] },
      ],
    });
    expect(snapshot.objectsBySchema).toEqual({
      'analytics.main': [{ name: 'main', kind: 'table' }],
      'analytics.reporting': [{ name: 'reporting', kind: 'table' }],
    });
  });

  it('rejects when the catalog cannot be refreshed', async () => {
    const error = new Error('catalog unavailable');

    await expect(
      fetchExplorerSnapshot({
        getDatabases: vi.fn().mockRejectedValue(error),
        getSchemas: vi.fn(),
        getSchemaObjects: vi.fn(),
      }),
    ).rejects.toBe(error);
  });
});
