import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import ResultsGrid from './ResultsGrid.svelte';

afterEach(cleanup);

const COLUMNS = [
  { name: 'id', type_name: 'int4' },
  { name: 'name', type_name: 'text' },
  { name: 'active', type_name: 'bool' },
  { name: 'created_at', type_name: 'timestamptz' },
];

const ROWS = [
  [1, 'Ada', true, '2026-01-01T00:00:00Z'],
  [2, 'Grace', false, '2026-01-02T00:00:00Z'],
];

const NAME_FILTER = {
  id: 'f1',
  column: 'name',
  operator: 'contains',
  value: 'Ada',
};

function setup(props = {}) {
  const onStateChange = vi.fn();
  const result = render(ResultsGrid, {
    columns: COLUMNS,
    rows: ROWS,
    fetchedCount: ROWS.length,
    isEnd: true,
    tabId: 'tab-1',
    onStateChange,
    ...props,
  });
  return { ...result, onStateChange };
}

describe('ResultsGrid mounting', () => {
  it('renders column headers and rows', () => {
    const { getByText } = setup();
    expect(getByText('name')).toBeTruthy();
    expect(getByText('Ada')).toBeTruthy();
  });
});

describe('filter bar survival', () => {
  it('keeps the filter chips visible when the result set is empty', () => {
    const { getByText, getByLabelText } = setup({
      rows: [],
      fetchedCount: 0,
      initFilters: [NAME_FILTER],
    });
    expect(getByLabelText('Remove filter on name')).toBeTruthy();
    expect(getByText('Clear all')).toBeTruthy();
  });

  it('explains an empty result caused by filters and offers a way out', () => {
    const { getByText } = setup({
      rows: [],
      fetchedCount: 0,
      initFilters: [NAME_FILTER],
    });
    expect(getByText('No rows match your filters')).toBeTruthy();
    expect(getByText('Clear filters')).toBeTruthy();
  });

  it('keeps the generic empty state when nothing is filtered', () => {
    const { getByText, queryByText } = setup({ rows: [], fetchedCount: 0 });
    expect(getByText('No rows found')).toBeTruthy();
    expect(queryByText('No rows match your filters')).toBeNull();
  });

  it('keeps the filter chips visible when the query errors', () => {
    const { getByLabelText, getByText } = setup({
      rows: [],
      fetchedCount: 0,
      error: 'operator does not exist: uuid > unknown',
      initFilters: [NAME_FILTER],
    });
    expect(getByLabelText('Remove filter on name')).toBeTruthy();
    expect(getByText('Query Failed')).toBeTruthy();
  });

  it('clears filters and asks for a refetch from the empty state', async () => {
    const { getByText, onStateChange } = setup({
      rows: [],
      fetchedCount: 0,
      initFilters: [NAME_FILTER],
    });
    await fireEvent.click(getByText('Clear filters'));
    expect(onStateChange).toHaveBeenCalledTimes(1);
    expect(onStateChange.mock.calls[0][0].filters).toEqual([]);
  });
});

describe('filter bar visibility', () => {
  it('shows the bar for a tab that arrives with filters, without being opened', () => {
    const { getByLabelText } = setup({ initFilters: [NAME_FILTER] });
    expect(getByLabelText('Remove filter on name')).toBeTruthy();
  });

  it('hides the bar when there are no filters and it has not been opened', () => {
    const { queryByText } = setup();
    expect(queryByText('Add filter')).toBeNull();
  });

  it('opens the bar from the Filter button', async () => {
    const { getByTitle, getByText } = setup();
    await fireEvent.click(getByTitle('Filter'));
    expect(getByText('Add filter')).toBeTruthy();
  });

  it('refuses to hide the bar while a filter exists', async () => {
    const { getByTitle, getByText } = setup({ initFilters: [NAME_FILTER] });
    await fireEvent.click(getByTitle('Filter'));
    expect(getByText('Add filter')).toBeTruthy();
  });
});

describe('filter query economy', () => {
  it('does not refetch when a filter needing a value is added', async () => {
    const { getByTitle, getByText, getAllByText, onStateChange } = setup();
    await fireEvent.click(getByTitle('Filter'));
    await fireEvent.click(getByText('Add filter'));
    // Click the first 'text' label (in the picker, before the thead)
    await fireEvent.click(getAllByText('text')[0]);
    expect(onStateChange).not.toHaveBeenCalled();
  });

  it('refetches when a filter needing no value is added', async () => {
    const { getByTitle, getByText, getAllByText, onStateChange } = setup();
    await fireEvent.click(getByTitle('Filter'));
    await fireEvent.click(getByText('Add filter'));
    // Click the first 'bool' label (in the picker, before the thead)
    await fireEvent.click(getAllByText('bool')[0]);
    expect(onStateChange).toHaveBeenCalledTimes(1);
    expect(onStateChange.mock.calls[0][0].filters[0].operator).toBe('istrue');
  });

  it('carries pending filters in the emitted state so they survive a tab switch', async () => {
    const { getByLabelText, onStateChange } = setup({
      initFilters: [
        NAME_FILTER,
        { id: 'f2', column: 'id', operator: 'gte', value: '' },
      ],
    });
    await fireEvent.click(getByLabelText('Remove filter on name'));
    const emitted = onStateChange.mock.calls[0][0].filters;
    expect(emitted).toHaveLength(1);
    expect(emitted[0].value).toBe('');
  });
});

describe('loading state', () => {
  it('keeps the previous rows on screen while refetching', () => {
    const { getByText } = setup({ loading: true });
    expect(getByText('Ada')).toBeTruthy();
  });

  it('marks the table as busy while refetching', () => {
    const { container } = setup({ loading: true });
    expect(container.querySelector('.table-wrapper.loading')).toBeTruthy();
  });
});

describe('column header menu', () => {
  it('exposes the sort control as a button for keyboard users', () => {
    const { getByRole } = setup();
    expect(getByRole('button', { name: /sort by name/i })).toBeTruthy();
  });

  it('sorts on click from the keyboard', async () => {
    const { getByRole, onStateChange } = setup();
    await fireEvent.click(getByRole('button', { name: /sort by name/i }));
    expect(onStateChange.mock.calls[0][0].sortCol).toBe('name');
  });

  it('opens a menu of column actions', async () => {
    const { getByLabelText, getByText } = setup();
    await fireEvent.click(getByLabelText('Column actions for name'));
    expect(getByText('Sort ascending')).toBeTruthy();
    expect(getByText('Filter by this column')).toBeTruthy();
  });

  it('adds a pending filter from the menu without refetching', async () => {
    const { getByLabelText, getByText, onStateChange } = setup();
    await fireEvent.click(getByLabelText('Column actions for name'));
    await fireEvent.click(getByText('Filter by this column'));
    expect(onStateChange).not.toHaveBeenCalled();
    expect(getByLabelText('Remove filter on name')).toBeTruthy();
  });

  it('sorts descending from the menu', async () => {
    const { getByLabelText, getByText, onStateChange } = setup();
    await fireEvent.click(getByLabelText('Column actions for name'));
    await fireEvent.click(getByText('Sort descending'));
    expect(onStateChange.mock.calls[0][0]).toMatchObject({
      sortCol: 'name',
      sortDir: 'desc',
    });
  });

  it('clears the sort from the menu', async () => {
    const { getByLabelText, getByText, onStateChange } = setup({
      initSortCol: 'name',
      initSortDir: 'asc',
    });
    await fireEvent.click(getByLabelText('Column actions for name'));
    await fireEvent.click(getByText('Clear sort'));
    expect(onStateChange.mock.calls[0][0].sortCol).toBeNull();
  });
});

describe('cell context menu', () => {
  async function openCellMenu(
    getAll: (text: string) => Element[],
    text: string,
  ) {
    const allText = getAll(text);
    const el = allText[0];
    const cell = el.tagName === 'TD' ? el : el.closest('td');
    if (!cell) throw new Error('Could not find parent td');
    await fireEvent.contextMenu(cell);
  }

  it('offers value filters on right-click', async () => {
    const { getByText, getAllByText } = setup();
    await openCellMenu(getAllByText, 'Ada');
    expect(getByText('Filter by this value')).toBeTruthy();
    expect(getByText('Filter out this value')).toBeTruthy();
    expect(getByText('Copy value')).toBeTruthy();
  });

  it('filters by the cell value and refetches at once', async () => {
    const { getAllByText, onStateChange } = setup();
    await openCellMenu(getAllByText, 'Ada');
    await fireEvent.click(getAllByText('Filter by this value')[0]);
    expect(onStateChange).toHaveBeenCalledTimes(1);
    expect(onStateChange.mock.calls[0][0].filters[0]).toMatchObject({
      column: 'name',
      operator: 'eq',
      value: 'Ada',
    });
  });

  it('filters out the cell value when negated', async () => {
    const { getAllByText, onStateChange } = setup();
    await openCellMenu(getAllByText, 'Ada');
    await fireEvent.click(getAllByText('Filter out this value')[0]);
    expect(onStateChange.mock.calls[0][0].filters[0].operator).toBe('neq');
  });

  it('offers null checks for a NULL cell', async () => {
    const { getByText, getAllByText } = setup({
      rows: [[1, null, true, '2026-01-01T00:00:00Z']],
      fetchedCount: 1,
    });
    await openCellMenu(getAllByText, 'NULL');
    expect(getByText('Filter by is null')).toBeTruthy();
    expect(getByText('Filter by is not null')).toBeTruthy();
  });

  it('replaces an existing equality filter on the same column', async () => {
    const { getAllByText, onStateChange } = setup({
      initFilters: [
        { id: 'f1', column: 'name', operator: 'eq', value: 'Grace' },
      ],
    });
    await openCellMenu(getAllByText, 'Ada');
    await fireEvent.click(getAllByText('Filter by this value')[0]);
    const emitted = onStateChange.mock.calls[0][0].filters;
    expect(emitted).toHaveLength(1);
    expect(emitted[0].value).toBe('Ada');
  });

  it('opens the filter bar so the new chip is visible', async () => {
    const { getAllByText, getByText } = setup();
    await openCellMenu(getAllByText, 'Ada');
    await fireEvent.click(getAllByText('Filter by this value')[0]);
    expect(getByText('Add filter')).toBeTruthy();
  });
});

describe('keyboard shortcut', () => {
  it('opens the bar and the column picker on Cmd+F', async () => {
    const { getByPlaceholderText } = setup();
    await fireEvent.keyDown(window, { key: 'f', metaKey: true });
    expect(getByPlaceholderText('Search columns…')).toBeTruthy();
  });

  it('also accepts Ctrl+F for non-mac keyboards', async () => {
    const { getByPlaceholderText } = setup();
    await fireEvent.keyDown(window, { key: 'f', ctrlKey: true });
    expect(getByPlaceholderText('Search columns…')).toBeTruthy();
  });

  it('ignores a bare f so typing in a filter value still works', async () => {
    const { queryByPlaceholderText } = setup();
    await fireEvent.keyDown(window, { key: 'f' });
    expect(queryByPlaceholderText('Search columns…')).toBeNull();
  });
});
