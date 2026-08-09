import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, cleanup, fireEvent } from '@testing-library/svelte';

const fetchPage = vi.fn();

vi.mock('../../ipc/notebook', () => ({
  notebookAttach: vi.fn(),
  notebookDetach: vi.fn(),
  notebookClearOutputs: vi.fn(),
  notebookRunCell: vi.fn(),
  notebookCancelCell: vi.fn(),
  notebookFetchPage: (...a: unknown[]) => fetchPage(...a),
  notebookCountRows: vi.fn(),
}));

import Cell from './Cell.svelte';
import { createNotebookModel } from '../../stores/notebook.svelte.ts';

afterEach(cleanup);

function modelWithTableOutput(rowCount: number) {
  const model = createNotebookModel();
  model.cells[0].status = 'ok';
  model.cells[0].execution_order = 1;
  model.cells[0].source = 'select n from generate_series(1, 100) n';
  model.cells[0].outputs = {
    columns: [{ name: 'n', type_name: 'int4' }],
    rows: Array.from({ length: rowCount }, (_, i) => [i + 1]),
    total_count: null,
    is_truncated: false,
    page_size: 10,
    is_wrappable: true,
  };
  return model;
}

describe('Cell output', () => {
  beforeEach(() => fetchPage.mockReset());

  it('renders table output through the shared results grid', () => {
    const model = modelWithTableOutput(3);
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.results-grid.embedded')).toBeTruthy();
  });

  it('pages at the cell page size rather than showing every row', () => {
    const model = modelWithTableOutput(25);
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelectorAll('tbody tr').length).toBe(10);
  });

  it('offers the 5/10/25 page-size selector', () => {
    const model = modelWithTableOutput(25);
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    const select = container.querySelector(
      '.page-size-select',
    ) as HTMLSelectElement;
    expect(select).toBeTruthy();
    expect([...select.options].map((o) => o.value)).toEqual(['5', '10', '25']);
  });

  it('renders text output as preformatted text, not a grid', () => {
    const model = createNotebookModel();
    model.cells[0].status = 'ok';
    model.cells[0].outputs = { content: 'plain result' };
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.results-grid')).toBeNull();
    expect(container.querySelector('.text-output')?.textContent).toContain(
      'plain result',
    );
  });

  it('shows a stale badge when the cell is stale', () => {
    const model = modelWithTableOutput(3);
    model.cells[0].status = 'stale';
    model.cells[0].stale_since = Date.now();
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.stale-badge')).toBeTruthy();
  });

  it('re-renders the grid after a page fetch', async () => {
    // Regression: the grid view must be derived from the REACTIVE cell.view
    // mirror (written by cellView.put), not only from cellView's internal plain
    // Map — a $derived over the Map alone never re-evaluates after fetchMore,
    // leaving the grid frozen on page 1.
    //
    // Seeded with a FULL page: a short first page is already known to be the
    // end of the result, so there would be nothing left to fetch.
    const model = modelWithTableOutput(10);
    model.sessionKey = 'sk';
    fetchPage.mockResolvedValue({
      columns: [{ name: 'n', type_name: 'int4' }],
      rows: [[11], [12], [13]],
      total_count: null,
      is_truncated: false,
      page_size: 10,
      is_wrappable: true,
    });
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelectorAll('tbody tr').length).toBe(10);

    await model.cellView.fetchMore(model.cells[0].id);

    // The newly fetched rows land on page 2, so advancing proves they rendered
    // rather than being stranded in the non-reactive Map.
    const next = [...container.querySelectorAll('.page-btn')].find((b) =>
      (b.textContent ?? '').includes('Next'),
    ) as HTMLButtonElement;
    expect(next.disabled).toBe(false);
    await fireEvent.click(next);
    expect(container.querySelectorAll('tbody tr').length).toBe(3);
  });
});

describe('Cell output suppression', () => {
  // Regression: an empty cell that got run stored the backend's zero-column
  // envelope, and the grid rendered it as a full-height "No rows found" panel
  // on a cell the user had never typed a query into.
  it('renders nothing for a zero-column result envelope', () => {
    const model = createNotebookModel();
    model.cells[0].status = 'ok';
    model.cells[0].execution_order = 4;
    model.cells[0].outputs = {
      columns: [],
      rows: [],
      total_count: null,
      is_truncated: false,
      page_size: 10,
      is_wrappable: true,
    };
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.cell-output')).toBeNull();
    expect(container.textContent).not.toContain('No rows found');
  });

  it('still renders an empty grid when a real query returns zero rows', () => {
    // Columns present, rows empty: the query ran and legitimately matched
    // nothing, which the grid should say out loud.
    const model = modelWithTableOutput(0);
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.cell-output')).toBeTruthy();
    expect(container.textContent).toContain('No rows found');
  });

  it('renders no output area for a cell that has never run', () => {
    const model = createNotebookModel();
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.cell-output')).toBeNull();
  });

  it('ignores an empty text payload', () => {
    const model = createNotebookModel();
    model.cells[0].status = 'ok';
    model.cells[0].outputs = { content: '' };
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.cell-output')).toBeNull();
  });
});

describe('Cell collapsed state', () => {
  // A collapsed cell used to render an empty body: no editor, no output, no
  // way to tell what the cell contained.
  it('summarises the source instead of rendering an empty body', () => {
    const model = modelWithTableOutput(3);
    model.cells[0].source = '-- a comment\nselect n from series\nwhere n > 1';
    model.cells[0].collapsed = true;
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    // First meaningful line, skipping the leading comment.
    expect(container.querySelector('.summary-source')?.textContent).toBe(
      'select n from series',
    );
  });

  it('summarises the result size so a folded cell still reports its output', () => {
    const model = modelWithTableOutput(3);
    model.cells[0].collapsed = true;
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.summary-output')?.textContent).toBe(
      '3 rows',
    );
  });

  it('singularises a one-row result', () => {
    const model = modelWithTableOutput(1);
    model.cells[0].collapsed = true;
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.summary-output')?.textContent).toBe(
      '1 row',
    );
  });

  it('labels an empty collapsed cell rather than showing a blank row', () => {
    const model = createNotebookModel();
    model.cells[0].collapsed = true;
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    const summary = container.querySelector('.summary-source');
    expect(summary?.textContent).toBe('Empty cell');
    expect(summary?.classList.contains('empty')).toBe(true);
  });

  it('expands again when the summary is clicked', async () => {
    const model = modelWithTableOutput(3);
    model.cells[0].collapsed = true;
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    await fireEvent.click(container.querySelector('.collapsed-summary')!);
    expect(model.cells[0].collapsed).toBe(false);
  });

  it('hides the output grid while collapsed', () => {
    const model = modelWithTableOutput(3);
    model.cells[0].collapsed = true;
    const { container } = render(Cell, {
      props: { cell: model.cells[0], model },
    });
    expect(container.querySelector('.results-grid')).toBeNull();
  });
});
