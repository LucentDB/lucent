import { describe, it, expect, vi, beforeEach } from 'vitest';

const fetchPage = vi.fn();
const countRows = vi.fn();

vi.mock('../ipc/notebook', () => ({
  notebookFetchPage: (...a: unknown[]) => fetchPage(...a),
  notebookCountRows: (...a: unknown[]) => countRows(...a),
  notebookAttach: vi.fn(),
  notebookDetach: vi.fn(),
  notebookClearOutputs: vi.fn(),
}));

import { createNotebookModel } from './notebook.svelte.ts';
import { createCellView, defaultViewState } from './notebook-view.ts';

function page(rows: unknown[][]) {
  return {
    columns: [{ name: 'n', type_name: 'int4' }],
    rows,
    total_count: null,
    is_truncated: false,
    page_size: 10,
    is_wrappable: true,
  };
}

/** A full page: the only seeded state from which more rows may exist. */
function fullPage() {
  return page(Array.from({ length: 10 }, (_, i) => [i + 1]));
}

describe('cell view state', () => {
  beforeEach(() => {
    fetchPage.mockReset();
    countRows.mockReset();
  });

  it('defaults to page size 10 with no filters or sort', () => {
    const v = defaultViewState();
    expect(v.pageSize).toBe(10);
    expect(v.filters).toEqual([]);
    expect(v.sortCol).toBeNull();
    expect(v.totalCount).toBeNull();
  });

  it('stateFor lazily creates state seeded from the cell output', () => {
    const model = createNotebookModel();
    const id = model.cells[0].id;
    model.cells[0].outputs = page([[1], [2]]);
    const view = createCellView(model);
    const state = view.stateFor(id);
    expect(state.rows).toEqual([[1], [2]]);
    expect(state.pageSize).toBe(10);
  });

  it('applyState refetches from offset 0 with the new sort', async () => {
    const model = createNotebookModel();
    model.sessionKey = 'sk';
    const id = model.cells[0].id;
    model.cells[0].outputs = page([[1]]);
    fetchPage.mockResolvedValue(page([[9]]));

    const view = createCellView(model);
    await view.applyState(id, { filters: [], sortCol: 'n', sortDir: 'desc' });

    const args = fetchPage.mock.calls[0];
    expect(args[3]).toBe(10); // limit
    expect(args[4]).toBe(0); // offset — a sort change must restart paging
    expect(args[5]).toEqual({ column: 'n', direction: 'desc' });
    expect(view.stateFor(id).rows).toEqual([[9]]);
  });

  it('fetchMore appends the next window and marks the end when short', async () => {
    const model = createNotebookModel();
    model.sessionKey = 'sk';
    const id = model.cells[0].id;
    // Seeded full page: a short seed is already the end, so there would be
    // nothing to fetch.
    model.cells[0].outputs = fullPage();
    fetchPage.mockResolvedValue(page([[11]])); // 1 row < pageSize 10 → end

    const view = createCellView(model);
    await view.fetchMore(id);

    const state = view.stateFor(id);
    expect(state.fetchedCount).toBe(11);
    expect(state.rows[10]).toEqual([11]);
    expect(state.isEnd).toBe(true);
  });

  describe('seeded end-of-result detection', () => {
    // The grid hides its paging controls only when it knows the result ended.
    // Without this the count(*) cell showed "Rows 1–1 of 1+" plus Prev/Next.
    it('marks a short first page as the end', () => {
      const model = createNotebookModel();
      model.cells[0].outputs = page([[1]]);
      expect(createCellView(model).stateFor(model.cells[0].id).isEnd).toBe(
        true,
      );
    });

    it('leaves a full first page open, since more rows may follow', () => {
      const model = createNotebookModel();
      model.cells[0].outputs = fullPage();
      expect(createCellView(model).stateFor(model.cells[0].id).isEnd).toBe(
        false,
      );
    });

    it('treats an unpageable result as ended, since it cannot be paged', () => {
      const model = createNotebookModel();
      model.cells[0].outputs = { ...fullPage(), is_wrappable: false };
      expect(createCellView(model).stateFor(model.cells[0].id).isEnd).toBe(
        true,
      );
    });

    it('does not fetch beyond a result already known to have ended', async () => {
      const model = createNotebookModel();
      model.sessionKey = 'sk';
      model.cells[0].outputs = page([[1]]);
      const view = createCellView(model);
      await view.fetchMore(model.cells[0].id);
      expect(fetchPage).not.toHaveBeenCalled();
    });
  });

  it('countAll stores the returned total', async () => {
    const model = createNotebookModel();
    model.sessionKey = 'sk';
    const id = model.cells[0].id;
    model.cells[0].outputs = page([[1]]);
    countRows.mockResolvedValue(1203);

    const view = createCellView(model);
    await view.countAll(id);
    expect(view.stateFor(id).totalCount).toBe(1203);
  });

  it('setPageSize refetches from offset 0 with the new limit', async () => {
    const model = createNotebookModel();
    model.sessionKey = 'sk';
    const id = model.cells[0].id;
    model.cells[0].outputs = page([[1]]);
    fetchPage.mockResolvedValue(page([[1], [2], [3], [4], [5]]));

    const view = createCellView(model);
    await view.setPageSize(id, 5);

    const args = fetchPage.mock.calls[0];
    expect(args[3]).toBe(5);
    expect(args[4]).toBe(0);
    expect(view.stateFor(id).pageSize).toBe(5);
  });

  it('does nothing when the notebook has no session', async () => {
    const model = createNotebookModel();
    model.sessionKey = null;
    const id = model.cells[0].id;
    // Full page, so the session guard is what stops the fetch, not isEnd.
    model.cells[0].outputs = fullPage();
    const view = createCellView(model);
    await view.fetchMore(id);
    expect(fetchPage).not.toHaveBeenCalled();
  });
});
