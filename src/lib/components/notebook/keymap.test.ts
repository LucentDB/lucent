import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

vi.mock('../../ipc/notebook', () => ({
  notebookAttach: vi.fn(),
  notebookDetach: vi.fn(),
  notebookClearOutputs: vi.fn(),
  notebookRunCell: vi.fn(async () => ({ content: '' })),
  notebookCancelCell: vi.fn(),
  notebookFetchPage: vi.fn(),
  notebookCountRows: vi.fn(),
}));

import { createNotebookModel } from '../../stores/notebook.svelte.ts';
import { createCommandKeymap } from './keymap.ts';

function key(k: string, mods: Partial<Record<string, boolean>> = {}) {
  return {
    key: k,
    metaKey: !!mods.metaKey,
    ctrlKey: !!mods.ctrlKey,
    shiftKey: !!mods.shiftKey,
    altKey: !!mods.altKey,
  };
}

describe('command-mode keymap', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  function setup() {
    const model = createNotebookModel();
    model.select(model.cells[0].id);
    return { model, handle: createCommandKeymap(model) };
  }

  it('Enter switches to edit mode and is consumed', () => {
    const { model, handle } = setup();
    expect(handle(key('Enter'))).toBe(true);
    expect(model.mode).toBe('edit');
  });

  it('ArrowDown and ArrowUp move selection', () => {
    const { model, handle } = setup();
    const first = model.cells[0].id;
    model.addCell(first, 'sql');
    model.select(first);
    expect(handle(key('ArrowDown'))).toBe(true);
    expect(model.selectedCellId).toBe(model.cells[1].id);
    handle(key('ArrowUp'));
    expect(model.selectedCellId).toBe(first);
  });

  it('a inserts above and b inserts below', () => {
    const { model, handle } = setup();
    const first = model.cells[0].id;
    handle(key('b'));
    expect(model.cells.length).toBe(2);
    expect(model.cells[1].id).toBe(model.selectedCellId);
    model.select(first);
    handle(key('a'));
    expect(model.cells[0].id).toBe(model.selectedCellId);
  });

  it('y converts to sql and m converts to markdown', () => {
    const { model, handle } = setup();
    handle(key('m'));
    expect(model.cells[0].kind).toBe('markdown');
    handle(key('y'));
    expect(model.cells[0].kind).toBe('sql');
  });

  it('Space toggles collapse', () => {
    const { model, handle } = setup();
    expect(handle(key(' '))).toBe(true);
    expect(model.cells[0].collapsed).toBe(true);
    handle(key(' '));
    expect(model.cells[0].collapsed).toBe(false);
  });

  it('dd deletes the cell, single d does not', () => {
    const { model, handle } = setup();
    model.addCell(model.cells[0].id, 'sql');
    model.select(model.cells[0].id);
    expect(model.cells.length).toBe(2);

    handle(key('d'));
    expect(model.cells.length).toBe(2); // one press is not enough
    handle(key('d'));
    expect(model.cells.length).toBe(1);
  });

  it('dd chord expires after the timeout', () => {
    const { model, handle } = setup();
    model.addCell(model.cells[0].id, 'sql');
    model.select(model.cells[0].id);

    handle(key('d'));
    vi.advanceTimersByTime(600);
    handle(key('d'));
    expect(model.cells.length).toBe(2); // second d started a fresh chord
  });

  it('does not consume modifier combinations it has no binding for', () => {
    const { handle } = setup();
    expect(handle(key('k', { metaKey: true }))).toBe(false);
    expect(handle(key('a', { metaKey: true }))).toBe(false);
    expect(handle(key('n', { metaKey: true, shiftKey: true }))).toBe(false);
  });

  it('does nothing when no cell is selected', () => {
    const model = createNotebookModel();
    const handle = createCommandKeymap(model);
    model.select(null);
    expect(handle(key('b'))).toBe(false);
    expect(model.cells.length).toBe(1);
  });
});
