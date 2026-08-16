import { describe, it, expect, vi, beforeEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  Channel: class {},
}));

import { editorSchema } from './editor-schema.svelte.ts';

beforeEach(() => {
  invoke.mockReset();
  editorSchema.tables = [];
  editorSchema.loaded = false;
});

describe('editorSchema.refresh', () => {
  it('populates tables from get_editor_schema on success', async () => {
    invoke.mockResolvedValue([
      {
        schema: 'public',
        name: 'customers',
        columns: [{ name: 'id', type_name: 'int4' }],
      },
    ]);
    await editorSchema.refresh();
    expect(invoke).toHaveBeenCalledWith('get_editor_schema', undefined);
    expect(editorSchema.tables).toEqual([
      {
        schema: 'public',
        name: 'customers',
        columns: [{ name: 'id', type_name: 'int4' }],
      },
    ]);
    expect(editorSchema.loaded).toBe(true);
  });

  it('resets to an empty list and does not throw when the fetch fails', async () => {
    invoke.mockRejectedValue({ kind: 'QueryError', message: 'not connected' });
    await expect(editorSchema.refresh()).resolves.toBeUndefined();
    expect(editorSchema.tables).toEqual([]);
    expect(editorSchema.loaded).toBe(true);
  });

  it('keeps the latest response when overlapping refreshes resolve out of order', async () => {
    let resolveFirst!: (v: unknown) => void;
    let resolveSecond!: (v: unknown) => void;
    invoke
      .mockReturnValueOnce(
        new Promise((r) => {
          resolveFirst = r;
        }),
      )
      .mockReturnValueOnce(
        new Promise((r) => {
          resolveSecond = r;
        }),
      );

    const first = editorSchema.refresh();
    const second = editorSchema.refresh();

    // The second (newest) request resolves first and commits.
    resolveSecond([
      {
        schema: 'public',
        name: 'newer',
        columns: [{ name: 'id', type_name: 'int4' }],
      },
    ]);
    await second;
    // The first (stale) request resolves after — its tables must be dropped.
    resolveFirst([
      {
        schema: 'public',
        name: 'stale',
        columns: [{ name: 'id', type_name: 'int4' }],
      },
    ]);
    await first;

    expect(editorSchema.tables).toEqual([
      {
        schema: 'public',
        name: 'newer',
        columns: [{ name: 'id', type_name: 'int4' }],
      },
    ]);
    expect(editorSchema.loaded).toBe(true);
  });

  it('does not let a stale failure wipe a newer success', async () => {
    let rejectFirst!: (e: unknown) => void;
    invoke
      .mockReturnValueOnce(
        new Promise((_, r) => {
          rejectFirst = r;
        }),
      )
      .mockReturnValueOnce(
        Promise.resolve([
          {
            schema: 'public',
            name: 'newer',
            columns: [{ name: 'id', type_name: 'int4' }],
          },
        ]),
      );

    const first = editorSchema.refresh();
    const second = editorSchema.refresh();
    await second; // the newer request succeeds and commits
    rejectFirst(new Error('stale failure'));
    await first;

    expect(editorSchema.tables).toEqual([
      {
        schema: 'public',
        name: 'newer',
        columns: [{ name: 'id', type_name: 'int4' }],
      },
    ]);
    expect(editorSchema.loaded).toBe(true);
  });
});
