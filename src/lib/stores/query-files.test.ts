import { describe, it, expect, vi, beforeEach } from 'vitest';

const confirm = vi.fn();
vi.mock('@tauri-apps/plugin-dialog', () => ({
  confirm: (...a: unknown[]) => confirm(...a),
}));
const invoke = vi.fn<(...args: unknown[]) => Promise<unknown>>(
  async () => undefined,
);
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...a: unknown[]) => invoke(...a),
}));

import { saveQueryTab, saveQueryTabAs } from './query-files.ts';

const tab = { name: 'query_1.sql', baseSql: 'select 1;' };

beforeEach(() => {
  confirm.mockReset();
  invoke.mockReset();
  invoke.mockImplementation(async () => undefined);
});

describe('saveQueryTab', () => {
  it('writes to the existing filePath without a dialog', async () => {
    const path = await saveQueryTab({ ...tab, filePath: '/tmp/q.sql' });
    expect(path).toBe('/tmp/q.sql');
    expect(invoke).toHaveBeenCalledWith('save_sql_file', {
      path: '/tmp/q.sql',
      content: 'select 1;',
    });
    expect(invoke).not.toHaveBeenCalledWith(
      'choose_save_path',
      expect.anything(),
    );
  });

  it('falls through to Save As when there is no filePath', async () => {
    invoke.mockResolvedValue('/tmp/query_1.sql');
    const path = await saveQueryTab(tab);
    expect(path).toBe('/tmp/query_1.sql');
    // The destination must first be chosen via the Rust-side dialog…
    expect(invoke).toHaveBeenCalledWith('choose_save_path', {
      defaultName: 'query_1.sql',
      filterName: 'SQL File',
      extensions: ['sql'],
    });
    // …then written through the approved-path gate.
    expect(invoke).toHaveBeenCalledWith('save_sql_file', {
      path: '/tmp/query_1.sql',
      content: 'select 1;',
    });
  });

  it('returns null when the dialog is cancelled', async () => {
    invoke.mockResolvedValue(null);
    const path = await saveQueryTab(tab);
    expect(path).toBeNull();
    expect(invoke).not.toHaveBeenCalledWith('save_sql_file', expect.anything());
  });
});

describe('saveQueryTabAs', () => {
  it('always opens the dialog and writes the chosen path', async () => {
    invoke.mockResolvedValue('/tmp/other.sql');
    const path = await saveQueryTabAs({ ...tab, filePath: '/tmp/q.sql' });
    expect(path).toBe('/tmp/other.sql');
    expect(invoke).toHaveBeenCalledWith('choose_save_path', {
      defaultName: 'query_1.sql',
      filterName: 'SQL File',
      extensions: ['sql'],
    });
    expect(invoke).toHaveBeenCalledWith('save_sql_file', {
      path: '/tmp/other.sql',
      content: 'select 1;',
    });
  });

  it('shows an error dialog and rethrows on write failure', async () => {
    invoke.mockImplementation(async (cmd: unknown) => {
      if (cmd === 'choose_save_path') return '/tmp/q.sql';
      throw new Error('disk full');
    });
    await expect(saveQueryTabAs(tab)).rejects.toThrow('disk full');
    expect(confirm).toHaveBeenCalledWith(
      expect.stringContaining('disk full'),
      expect.objectContaining({ title: 'Save failed', kind: 'error' }),
    );
  });
});
