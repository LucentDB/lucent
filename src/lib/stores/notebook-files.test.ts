import { describe, it, expect, vi, beforeEach } from 'vitest';

const confirm = vi.fn();

vi.mock('@tauri-apps/plugin-dialog', () => ({
  confirm: (...a: unknown[]) => confirm(...a),
}));

const invoke = vi.fn<(...args: unknown[]) => Promise<unknown>>(
  async () => '/tmp/demo.lucent',
);
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...a: unknown[]) => invoke(...a),
}));

const notebookSave = vi.fn(async (..._args: unknown[]) => '/tmp/demo.lucent');
vi.mock('../ipc/notebook', () => ({
  notebookSave: (...a: unknown[]) => notebookSave(...a),
  notebookOpen: vi.fn(),
  notebookAttach: vi.fn(async () => 'sk'),
  notebookDetach: vi.fn(),
  notebookClearOutputs: vi.fn(),
}));

import { notebooks } from './notebooks.svelte.ts';
import {
  saveNotebook,
  saveNotebookAs,
  pickNotebookToOpen,
  confirmDiscardIfDirty,
} from './notebook-files.ts';

const spec = { filePath: null, connectionId: 'p1', database: 'postgres' };

describe('notebook file actions', () => {
  beforeEach(async () => {
    if (notebooks.has('t1')) await notebooks.release('t1');
    invoke.mockReset();
    confirm.mockReset();
    notebookSave.mockClear();
    notebookSave.mockResolvedValue('/tmp/demo.lucent');
  });

  it('an untitled notebook prompts for a path before saving', async () => {
    notebooks.ensure('t1', spec);
    invoke.mockResolvedValue('/tmp/demo.lucent');

    const path = await saveNotebook('t1');

    expect(invoke).toHaveBeenCalledWith('choose_save_path', expect.anything());
    expect(notebookSave).toHaveBeenCalled();
    expect(path).toBe('/tmp/demo.lucent');
  });

  it('cancelling the save dialog writes nothing', async () => {
    notebooks.ensure('t1', spec);
    invoke.mockResolvedValue(null);

    const path = await saveNotebook('t1');

    expect(path).toBeNull();
    expect(notebookSave).not.toHaveBeenCalled();
  });

  it('a titled notebook saves without prompting', async () => {
    const model = notebooks.ensure('t1', spec);
    model.filePath = '/tmp/existing.lucent';
    model.sessionKey = 'sk';

    const path = await saveNotebook('t1');

    expect(invoke).not.toHaveBeenCalledWith(
      'choose_save_path',
      expect.anything(),
    );
    expect(notebookSave).toHaveBeenCalled();
    expect(path).toBe('/tmp/demo.lucent');
  });

  it('a notebook imported from a .sql file never overwrites the .sql source', async () => {
    const model = notebooks.ensure('t1', spec);
    model.filePath = '/tmp/imported.SQL';
    model.sessionKey = 'sk';
    invoke.mockResolvedValue('/tmp/imported.lucent');
    notebookSave.mockResolvedValue('/tmp/imported.lucent');

    const path = await saveNotebook('t1');

    expect(invoke).toHaveBeenCalledWith('choose_save_path', expect.anything());
    const savedPaths = notebookSave.mock.calls.map((c) => String(c[1]));
    expect(savedPaths.some((p) => p.toLowerCase().endsWith('.sql'))).toBe(
      false,
    );
    expect(path).toBe('/tmp/imported.lucent');
  });

  it('saveNotebookAs always prompts even when titled', async () => {
    const model = notebooks.ensure('t1', spec);
    model.filePath = '/tmp/existing.lucent';
    invoke.mockResolvedValue('/tmp/copy.lucent');

    await saveNotebookAs('t1');

    expect(invoke).toHaveBeenCalledWith('choose_save_path', expect.anything());
  });

  it('saving clears the dirty flag', async () => {
    const model = notebooks.ensure('t1', spec);
    model.setCellSource(model.cells[0].id, 'SELECT 1');
    expect(model.isDirty).toBe(true);
    invoke.mockResolvedValue('/tmp/demo.lucent');

    await saveNotebook('t1');

    expect(model.isDirty).toBe(false);
  });

  it('pickNotebookToOpen approves through the Rust open dialog', async () => {
    invoke.mockResolvedValue('/tmp/chosen.lucent');
    const picked = await pickNotebookToOpen();
    expect(picked).toBe('/tmp/chosen.lucent');
    expect(invoke).toHaveBeenCalledWith('choose_open_path', {
      filterName: 'Lucent Notebook / SQL File',
      extensions: ['lucent', 'sql'],
    });
  });

  it('pickNotebookToOpen returns null when cancelled', async () => {
    invoke.mockResolvedValue(null);
    expect(await pickNotebookToOpen()).toBeNull();
  });

  it('saving an unknown tab is a no-op', async () => {
    expect(await saveNotebook('nope')).toBeNull();
  });

  it('confirmDiscardIfDirty skips the prompt for a clean notebook', async () => {
    const model = notebooks.ensure('t1', spec);
    model.markSaved();
    expect(await confirmDiscardIfDirty('t1')).toBe('discard');
    expect(confirm).not.toHaveBeenCalled();
  });

  it('confirmDiscardIfDirty asks and maps the answer for a dirty notebook', async () => {
    const model = notebooks.ensure('t1', spec);
    model.setCellSource(model.cells[0].id, 'SELECT 1');
    confirm.mockResolvedValue(true);
    expect(await confirmDiscardIfDirty('t1')).toBe('save');
    confirm.mockResolvedValue(false);
    expect(await confirmDiscardIfDirty('t1')).toBe('discard');
  });
});
