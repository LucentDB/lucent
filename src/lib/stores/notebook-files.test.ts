import { describe, it, expect, vi, beforeEach } from 'vitest';

const save = vi.fn();
const open = vi.fn();
const confirm = vi.fn();

vi.mock('@tauri-apps/plugin-dialog', () => ({
  save: (...a: unknown[]) => save(...a),
  open: (...a: unknown[]) => open(...a),
  confirm: (...a: unknown[]) => confirm(...a),
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
    save.mockReset();
    open.mockReset();
    confirm.mockReset();
    notebookSave.mockClear();
    notebookSave.mockResolvedValue('/tmp/demo.lucent');
  });

  it('an untitled notebook prompts for a path before saving', async () => {
    notebooks.ensure('t1', spec);
    save.mockResolvedValue('/tmp/demo.lucent');

    const path = await saveNotebook('t1');

    expect(save).toHaveBeenCalled();
    expect(notebookSave).toHaveBeenCalled();
    expect(path).toBe('/tmp/demo.lucent');
  });

  it('cancelling the save dialog writes nothing', async () => {
    notebooks.ensure('t1', spec);
    save.mockResolvedValue(null);

    const path = await saveNotebook('t1');

    expect(path).toBeNull();
    expect(notebookSave).not.toHaveBeenCalled();
  });

  it('a titled notebook saves without prompting', async () => {
    const model = notebooks.ensure('t1', spec);
    model.filePath = '/tmp/existing.lucent';
    model.sessionKey = 'sk';

    const path = await saveNotebook('t1');

    expect(save).not.toHaveBeenCalled();
    expect(notebookSave).toHaveBeenCalled();
    expect(path).toBe('/tmp/demo.lucent');
  });

  it('saveNotebookAs always prompts even when titled', async () => {
    const model = notebooks.ensure('t1', spec);
    model.filePath = '/tmp/existing.lucent';
    save.mockResolvedValue('/tmp/copy.lucent');

    await saveNotebookAs('t1');

    expect(save).toHaveBeenCalled();
  });

  it('saving clears the dirty flag', async () => {
    const model = notebooks.ensure('t1', spec);
    model.setCellSource(model.cells[0].id, 'SELECT 1');
    expect(model.isDirty).toBe(true);
    save.mockResolvedValue('/tmp/demo.lucent');

    await saveNotebook('t1');

    expect(model.isDirty).toBe(false);
  });

  it('pickNotebookToOpen filters to .lucent and returns the choice', async () => {
    open.mockResolvedValue('/tmp/chosen.lucent');
    const picked = await pickNotebookToOpen();
    expect(picked).toBe('/tmp/chosen.lucent');
    const opts = open.mock.calls[0][0] as {
      filters: { extensions: string[] }[];
    };
    expect(opts.filters[0].extensions).toEqual(['lucent']);
  });

  it('pickNotebookToOpen returns null when cancelled', async () => {
    open.mockResolvedValue(null);
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
