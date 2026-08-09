import { describe, it, expect, vi, beforeEach } from 'vitest';

vi.mock('../ipc/notebook', () => ({
  notebookAttach: vi.fn(async () => 'session-key-1'),
  notebookDetach: vi.fn(async () => undefined),
  notebookClearOutputs: vi.fn(async () => undefined),
}));

import { notebooks } from './notebooks.svelte.ts';
import * as nb from '../ipc/notebook';

const spec = {
  filePath: null,
  connectionId: 'profile-1',
  database: 'postgres',
};

describe('notebook registry', () => {
  beforeEach(async () => {
    for (const id of ['tab-a', 'tab-b']) {
      if (notebooks.has(id)) await notebooks.release(id);
    }
    vi.clearAllMocks();
  });

  it('returns undefined for an unknown tab', () => {
    expect(notebooks.get('tab-a')).toBeUndefined();
  });

  it('ensure creates a model and is idempotent for the same tab', () => {
    const first = notebooks.ensure('tab-a', spec);
    const second = notebooks.ensure('tab-a', spec);
    expect(first).toBe(second);
    expect(nb.notebookAttach).toHaveBeenCalledTimes(1);
  });

  it('holds independent models per tab', () => {
    const a = notebooks.ensure('tab-a', spec);
    const b = notebooks.ensure('tab-b', spec);
    expect(a).not.toBe(b);
    a.setCellSource(a.cells[0].id, 'SELECT 1');
    expect(b.cells[0].source).toBe('');
  });

  it('preserves a tab model across repeated ensure calls (tab switch simulation)', async () => {
    const first = notebooks.ensure('tab-a', spec);
    first.setCellSource(first.cells[0].id, 'SELECT 42');
    // Wait for the fire-and-forget attach to resolve into sessionKey.
    await Promise.resolve();
    await Promise.resolve();
    expect(first.sessionKey).toBe('session-key-1');

    // Simulate switching to another tab and back: re-ensure the same tabId.
    notebooks.ensure('tab-b', spec);
    const again = notebooks.ensure('tab-a', spec);

    expect(again).toBe(first);
    expect(again.cells[0].source).toBe('SELECT 42');
    expect(again.sessionKey).toBe('session-key-1');
    // Re-ensuring must not re-attach.
    expect(nb.notebookAttach).toHaveBeenCalledTimes(2); // tab-a once, tab-b once
  });

  it('release detaches and forgets the tab', async () => {
    notebooks.ensure('tab-a', spec);
    await notebooks.release('tab-a');
    expect(nb.notebookDetach).toHaveBeenCalled();
    expect(notebooks.get('tab-a')).toBeUndefined();
  });

  it('release is safe for an unknown tab', async () => {
    await expect(notebooks.release('tab-zzz')).resolves.toBeUndefined();
  });

  it('a fresh ensure after release builds a new, blank model', async () => {
    const first = notebooks.ensure('tab-a', spec);
    first.setCellSource(first.cells[0].id, 'SELECT 1');
    await notebooks.release('tab-a');

    const fresh = notebooks.ensure('tab-a', spec);
    expect(fresh).not.toBe(first);
    expect(fresh.cells[0].source).toBe('');
  });
});
