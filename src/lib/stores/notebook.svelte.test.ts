import { describe, it, expect, vi } from 'vitest';
import { createNotebookModel } from './notebook.svelte.ts';
import * as notebookSession from './notebook-session.ts';

vi.mock('./notebook-session.ts', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./notebook-session.ts')>();
  return {
    ...actual,
    createNotebookSession: vi.fn(actual.createNotebookSession),
  };
});

describe('NotebookModel', () => {
  it('starts with one empty SQL cell', () => {
    const model = createNotebookModel();
    expect(model.cells.length).toBe(1);
    expect(model.cells[0].kind).toBe('sql');
    expect(model.cells[0].source).toBe('');
  });

  it('addCell inserts at correct position', () => {
    const model = createNotebookModel();
    expect(model.cells.length).toBe(1);
    model.addCell(model.cells[0].id, 'markdown');
    expect(model.cells.length).toBe(2);
    expect(model.cells[1].kind).toBe('markdown');
  });

  it('deleteCell removes cell and marks downstream stale', () => {
    const model = createNotebookModel();
    model.cells = [
      {
        id: 'a1b2c3d4',
        kind: 'sql',
        source: 'SELECT 1',
        status: 'ok',
        collapsed: false,
        outputs: null,
        execution_order: null,
        duration_ms: null,
        error: null,
        stale_since: null,
        ai_state: null,
      },
      {
        id: 'e5f6a7b8',
        kind: 'sql',
        source: 'SELECT * FROM ${a1b2c3d4}',
        status: 'ok',
        collapsed: false,
        outputs: null,
        execution_order: null,
        duration_ms: null,
        error: null,
        stale_since: null,
        ai_state: null,
      },
    ];
    model.deleteCell('a1b2c3d4');
    expect(model.cells.length).toBe(1);
    expect(model.cells[0].status).toBe('stale');
  });

  it('moveCell reorders and marks all stale', () => {
    const model = createNotebookModel();
    model.cells = [
      {
        id: 'a1b2c3d4',
        kind: 'markdown',
        source: '# Title',
        status: 'ok',
        collapsed: false,
        outputs: null,
        execution_order: null,
        duration_ms: null,
        error: null,
        stale_since: null,
        ai_state: null,
      },
      {
        id: 'e5f6a7b8',
        kind: 'sql',
        source: 'SELECT 1',
        status: 'ok',
        collapsed: false,
        outputs: null,
        execution_order: null,
        duration_ms: null,
        error: null,
        stale_since: null,
        ai_state: null,
      },
    ];
    model.moveCell('a1b2c3d4', 1);
    expect(model.cells[0].id).toBe('e5f6a7b8');
    expect(model.cells[0].status).toBe('stale');
    expect(model.cells[1].status).toBe('stale');
  });

  it('isDirty becomes true when source changes via setCellSource', () => {
    const model = createNotebookModel();
    expect(model.isDirty).toBe(false);
    model.setCellSource(model.cells[0].id, 'SELECT 1');
    expect(model.isDirty).toBe(true);
  });

  it('isDirty ignores output, status, and timing changes', () => {
    const model = createNotebookModel();
    model.markSaved();
    expect(model.isDirty).toBe(false);
    model.cells[0].outputs = {
      columns: [{ name: 'n', type_name: 'int4' }],
      rows: [[1], [2], [3]],
      total_count: 3,
      is_truncated: false,
      page_size: 10,
      is_wrappable: true,
    };
    model.cells[0].status = 'ok';
    model.cells[0].execution_order = 1;
    model.cells[0].duration_ms = 820;
    expect(model.isDirty).toBe(false);
  });

  it('markSaved clears dirty after a source edit', () => {
    const model = createNotebookModel();
    model.setCellSource(model.cells[0].id, 'SELECT 1');
    expect(model.isDirty).toBe(true);
    model.markSaved();
    expect(model.isDirty).toBe(false);
  });

  it('cell source change via setCellSource marks downstream refs stale', () => {
    const model = createNotebookModel();
    model.cells = [
      {
        id: 'a1b2c3d4',
        kind: 'sql',
        source: 'SELECT 1',
        status: 'ok',
        collapsed: false,
        outputs: null,
        execution_order: null,
        duration_ms: null,
        error: null,
        stale_since: null,
        ai_state: null,
      },
      {
        id: 'e5f6a7b8',
        kind: 'sql',
        source: 'SELECT * FROM ${a1b2c3d4}',
        status: 'ok',
        collapsed: false,
        outputs: null,
        execution_order: null,
        duration_ms: null,
        error: null,
        stale_since: null,
        ai_state: null,
      },
    ];
    model.setCellSource('a1b2c3d4', 'SELECT 2');
    expect(model.cells[1].status).toBe('stale');
    expect(model.cells[1].stale_since).toBeGreaterThan(0);
  });

  it('session is memoised across accesses', () => {
    const model = createNotebookModel();
    expect(model.session).toBe(model.session);
  });

  it('runCell, cancelCell, and restartSession reuse a single memoised session', async () => {
    vi.mocked(notebookSession.createNotebookSession).mockClear();
    const model = createNotebookModel();
    const id = model.cells[0].id;
    // Non-blank, or runCell short-circuits before ever touching the session.
    model.setCellSource(id, 'SELECT 1');
    // None of these are attached (no sessionKey), so each rejects — that's
    // fine, we only care how many times a session gets constructed.
    await model.runCell(id).catch(() => {});
    await model.runCell(id).catch(() => {});
    await model.cancelCell(id).catch(() => {});
    await model.restartSession().catch(() => {});
    expect(notebookSession.createNotebookSession).toHaveBeenCalledTimes(1);
  });
});

describe('blank cells are not executed', () => {
  // Executing a blank cell burned an execution counter and stored the backend's
  // zero-column envelope, which the UI then rendered as "No rows found" on a
  // cell the user had never typed into.
  // Spy on the model's own memoised session rather than the factory: a blank
  // cell never reaches the session at all, so a factory stub queued per test
  // would go unconsumed and leak into the next one.
  function modelWithStubSession() {
    const model = createNotebookModel();
    // The model discards the resolved output, so a bare resolve is enough.
    const runCell = vi
      .spyOn(model.session, 'runCell')
      .mockResolvedValue(undefined as never);
    return { model, runCell };
  }

  it('skips a cell whose source is empty', async () => {
    const { model, runCell } = modelWithStubSession();
    await model.runCell(model.cells[0].id);
    expect(runCell).not.toHaveBeenCalled();
  });

  it('skips a cell whose source is only whitespace', async () => {
    const { model, runCell } = modelWithStubSession();
    model.setCellSource(model.cells[0].id, '   \n\t\n  ');
    await model.runCell(model.cells[0].id);
    expect(runCell).not.toHaveBeenCalled();
  });

  it('runs a cell that has a statement in it', async () => {
    const { model, runCell } = modelWithStubSession();
    const id = model.cells[0].id;
    model.setCellSource(id, 'SELECT 1');
    await model.runCell(id);
    expect(runCell).toHaveBeenCalledWith(id);
  });

  it('leaves the execution counter untouched for a blank cell', async () => {
    const { model } = modelWithStubSession();
    await model.runCell(model.cells[0].id);
    expect(model.cells[0].execution_order).toBeNull();
    expect(model.cells[0].outputs).toBeNull();
  });

  it('runAll skips blank cells and does not count them toward progress', async () => {
    const { model, runCell } = modelWithStubSession();
    const first = model.cells[0].id;
    model.setCellSource(first, 'SELECT 1');
    model.addCell(first, 'sql'); // left blank
    model.addCell(model.cells[1].id, 'sql');
    const third = model.cells[2].id;
    model.setCellSource(third, 'SELECT 2');

    await model.runAll();

    expect(runCell).toHaveBeenCalledTimes(2);
    expect(runCell).toHaveBeenCalledWith(first);
    expect(runCell).toHaveBeenCalledWith(third);
  });
});

describe('selection, mode, and conversion', () => {
  it('selects a cell and defaults to command mode', () => {
    const model = createNotebookModel();
    expect(model.mode).toBe('command');
    model.select(model.cells[0].id);
    expect(model.selectedCellId).toBe(model.cells[0].id);
  });

  it('selectRelative moves selection and clamps at both ends', () => {
    const model = createNotebookModel();
    const first = model.cells[0].id;
    model.addCell(first, 'sql');
    const second = model.cells[1].id;

    model.select(first);
    model.selectRelative(1);
    expect(model.selectedCellId).toBe(second);
    model.selectRelative(1);
    expect(model.selectedCellId).toBe(second); // clamped at the bottom
    model.selectRelative(-1);
    expect(model.selectedCellId).toBe(first);
    model.selectRelative(-1);
    expect(model.selectedCellId).toBe(first); // clamped at the top
  });

  it('mode transitions between command and edit', () => {
    const model = createNotebookModel();
    model.select(model.cells[0].id);
    model.enterEditMode();
    expect(model.mode).toBe('edit');
    model.enterCommandMode();
    expect(model.mode).toBe('command');
  });

  it('insertCell above and below places the cell and selects it', () => {
    const model = createNotebookModel();
    const first = model.cells[0].id;
    const below = model.insertCell(first, 'below', 'markdown');
    expect(model.cells[1].id).toBe(below);
    expect(model.selectedCellId).toBe(below);

    const above = model.insertCell(first, 'above', 'ai');
    expect(model.cells[0].id).toBe(above);
    expect(model.cells[0].kind).toBe('ai');
  });

  it('convertCell preserves id, source, and alias but clears execution state', () => {
    const model = createNotebookModel();
    const id = model.cells[0].id;
    model.setCellSource(id, '# heading');
    model.cells[0].status = 'ok';
    model.cells[0].execution_order = 4;
    model.cells[0].duration_ms = 100;
    model.cells[0].outputs = {
      columns: [],
      rows: [],
      total_count: 0,
      is_truncated: false,
      page_size: 10,
      is_wrappable: true,
    };

    model.convertCell(id, 'markdown');

    const cell = model.cells[0];
    expect(cell.id).toBe(id);
    expect(cell.kind).toBe('markdown');
    expect(cell.source).toBe('# heading');
    expect(cell.status).toBe('pending');
    expect(cell.execution_order).toBeNull();
    expect(cell.outputs).toBeNull();
    expect(cell.duration_ms).toBeNull();
    expect(cell.ai_state).toBeNull();
  });

  it('convertCell to ai initializes ai_state', () => {
    const model = createNotebookModel();
    const id = model.cells[0].id;
    model.convertCell(id, 'ai');
    expect(model.cells[0].ai_state).not.toBeNull();
    expect(model.cells[0].ai_state?.final_sql).toBeNull();
  });

  it('convertCell marks the notebook dirty', () => {
    const model = createNotebookModel();
    model.markSaved();
    model.convertCell(model.cells[0].id, 'markdown');
    expect(model.isDirty).toBe(true);
  });

  it('runAndAdvance selects the next cell, or appends one on the last', async () => {
    const model = createNotebookModel();
    const first = model.cells[0].id;
    model.addCell(first, 'sql');
    const second = model.cells[1].id;
    // Both need a statement, or runCell skips them as blank and never reaches
    // the session that produces the expected rejection.
    model.setCellSource(first, 'SELECT 1');
    model.setCellSource(second, 'SELECT 2');

    // Middle cell: advance selects the next cell (run rejects — no session).
    await expect(model.runAndAdvance(first)).rejects.toThrow('not attached');
    expect(model.selectedCellId).toBe(second);

    // Last cell: appends a new SQL cell below and selects it.
    await expect(model.runAndAdvance(second)).rejects.toThrow('not attached');
    expect(model.cells.length).toBe(3);
    expect(model.selectedCellId).toBe(model.cells[2].id);
    expect(model.cells[2].kind).toBe('sql');
  });
});

describe('runAndAdvance contract (edit-mode bindings delegate here)', () => {
  it('runAndAdvance appends a new cell when run from the last cell', async () => {
    const model = createNotebookModel();
    const only = model.cells[0].id;
    model.select(only);
    vi.spyOn(model, 'runCell').mockResolvedValue(undefined);

    await model.runAndAdvance(only);

    expect(model.cells.length).toBe(2);
    expect(model.selectedCellId).toBe(model.cells[1].id);
    expect(model.mode).toBe('command');
  });

  it('runAndAdvance moves to the next cell when one exists', async () => {
    const model = createNotebookModel();
    const first = model.cells[0].id;
    model.addCell(first, 'sql');
    const second = model.cells[1].id;
    model.select(first);
    vi.spyOn(model, 'runCell').mockResolvedValue(undefined);

    await model.runAndAdvance(first);

    expect(model.cells.length).toBe(2);
    expect(model.selectedCellId).toBe(second);
  });
});
