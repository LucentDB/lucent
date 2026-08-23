import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import FilterBar from './FilterBar.svelte';

afterEach(cleanup);

const COLUMNS = [
  { name: 'id', type_name: 'int4' },
  { name: 'name', type_name: 'text' },
  { name: 'active', type_name: 'bool' },
];

function setup(props = {}) {
  const onFiltersChange = vi.fn();
  const onPickerOpenChange = vi.fn();
  const result = render(FilterBar, {
    columns: COLUMNS,
    filters: [],
    onFiltersChange,
    onPickerOpenChange,
    ...props,
  });
  return { ...result, onFiltersChange, onPickerOpenChange };
}

describe('FilterBar', () => {
  it('offers an Add filter button', () => {
    const { getByText } = setup();
    expect(getByText('Add filter')).toBeTruthy();
  });

  it('opens the column picker from Add filter', async () => {
    const { getByText, onPickerOpenChange } = setup();
    await fireEvent.click(getByText('Add filter'));
    expect(onPickerOpenChange).toHaveBeenCalledWith(true);
  });

  it('shows the picker when told to', () => {
    const { getByPlaceholderText } = setup({ pickerOpen: true });
    expect(getByPlaceholderText('Search columns…')).toBeTruthy();
  });

  it('adds a pending filter without asking for a refetch', async () => {
    const { getByText, onFiltersChange } = setup({ pickerOpen: true });
    await fireEvent.click(getByText('name'));
    expect(onFiltersChange).toHaveBeenCalledTimes(1);
    const [filters, opts] = onFiltersChange.mock.calls[0];
    expect(filters).toHaveLength(1);
    expect(filters[0].column).toBe('name');
    expect(opts.commit).toBe(false);
  });

  it('commits at once when the added filter needs no value', async () => {
    const { getByText, onFiltersChange } = setup({ pickerOpen: true });
    await fireEvent.click(getByText('active'));
    expect(onFiltersChange.mock.calls[0][1].commit).toBe(true);
  });

  it('renders a chip per filter with the word and between them', () => {
    const { getByText } = setup({
      filters: [
        { id: 'a', column: 'name', operator: 'contains', value: 'Ada' },
        { id: 'b', column: 'id', operator: 'gte', value: '5' },
      ],
    });
    expect(getByText('name')).toBeTruthy();
    expect(getByText('id')).toBeTruthy();
    expect(getByText('and')).toBeTruthy();
  });

  it('removes a chip and commits', async () => {
    const { getByLabelText, onFiltersChange } = setup({
      filters: [
        { id: 'a', column: 'name', operator: 'contains', value: 'Ada' },
      ],
    });
    await fireEvent.click(getByLabelText('Remove filter on name'));
    const [filters, opts] = onFiltersChange.mock.calls[0];
    expect(filters).toEqual([]);
    expect(opts.commit).toBe(true);
  });

  it('clears every filter at once', async () => {
    const { getByText, onFiltersChange } = setup({
      filters: [
        { id: 'a', column: 'name', operator: 'contains', value: 'Ada' },
        { id: 'b', column: 'id', operator: 'gte', value: '5' },
      ],
    });
    await fireEvent.click(getByText('Clear all'));
    expect(onFiltersChange).toHaveBeenCalledWith([], { commit: true });
  });

  it('hides Clear all when there is nothing to clear', () => {
    const { queryByText } = setup();
    expect(queryByText('Clear all')).toBeNull();
  });

  it('hides the SQL toggle when no describe function is available', () => {
    const { queryByText } = setup({
      filters: [{ id: 'a', column: 'name', operator: 'eq', value: 'Ada' }],
      onDescribeFilters: null,
    });
    expect(queryByText('SQL')).toBeNull();
  });

  it('shows the predicate returned by the describe function', async () => {
    const onDescribeFilters = vi.fn().mockResolvedValue(`WHERE "name" = 'Ada'`);
    const { getByText, findByText } = setup({
      filters: [{ id: 'a', column: 'name', operator: 'eq', value: 'Ada' }],
      onDescribeFilters,
    });
    await fireEvent.click(getByText('SQL'));
    expect(await findByText(`WHERE "name" = 'Ada'`)).toBeTruthy();
    expect(onDescribeFilters).toHaveBeenCalledWith([
      { column: 'name', operator: 'eq', value: 'Ada' },
    ]);
  });

  it('asks only for applyable filters when describing the SQL', async () => {
    const onDescribeFilters = vi.fn().mockResolvedValue('');
    const { getByText } = setup({
      filters: [
        { id: 'a', column: 'name', operator: 'eq', value: '' },
        { id: 'b', column: 'id', operator: 'gte', value: '5' },
      ],
      onDescribeFilters,
    });
    await fireEvent.click(getByText('SQL'));
    expect(onDescribeFilters).toHaveBeenCalledWith([
      { column: 'id', operator: 'gte', value: '5' },
    ]);
  });
});

describe('FilterBar SQL preview freshness', () => {
  const ADA = { id: 'a', column: 'name', operator: 'eq', value: 'Ada' };

  function sqlSetup(
    filters: Record<string, unknown>[],
    describe: (specs: unknown) => Promise<string>,
  ) {
    const onDescribeFilters = describe;
    const result = render(FilterBar, {
      columns: COLUMNS,
      filters,
      onFiltersChange: vi.fn(),
      onPickerOpenChange: vi.fn(),
      onDescribeFilters,
    });
    return { ...result, onDescribeFilters };
  }

  // The preview previously refreshed only on open, so editing a chip left SQL
  // on screen that no longer matched the query being run.
  it('re-describes the filters when they change while the panel is open', async () => {
    const describe = vi
      .fn()
      .mockResolvedValueOnce(`WHERE "name" = 'Ada'`)
      .mockResolvedValueOnce(`WHERE "name" = 'Grace'`);

    const { getByText, findByText, rerender } = sqlSetup([ADA], describe);
    await fireEvent.click(getByText('SQL'));
    expect(await findByText(`WHERE "name" = 'Ada'`)).toBeTruthy();

    await rerender({ filters: [{ ...ADA, value: 'Grace' }] });
    expect(await findByText(`WHERE "name" = 'Grace'`)).toBeTruthy();
    expect(describe).toHaveBeenCalledTimes(2);
  });

  it('does not re-describe when an edit leaves the applied set unchanged', async () => {
    const describe = vi.fn().mockResolvedValue(`WHERE "name" = 'Ada'`);
    const { getByText, findByText, rerender } = sqlSetup([ADA], describe);
    await fireEvent.click(getByText('SQL'));
    await findByText(`WHERE "name" = 'Ada'`);

    // A pending chip is not part of the query, so the SQL cannot have changed.
    await rerender({
      filters: [ADA, { id: 'b', column: 'id', operator: 'eq', value: '' }],
    });
    expect(describe).toHaveBeenCalledTimes(1);
  });

  it('does not query at all while the panel is closed', async () => {
    const describe = vi.fn().mockResolvedValue('');
    const { rerender } = sqlSetup([ADA], describe);
    await rerender({ filters: [{ ...ADA, value: 'Grace' }] });
    expect(describe).not.toHaveBeenCalled();
  });

  it('recovers to empty text when describing fails', async () => {
    const describe = vi.fn().mockRejectedValue(new Error('not connected'));
    const { getByText, findByText } = sqlSetup([ADA], describe);
    await fireEvent.click(getByText('SQL'));
    expect(await findByText('No filters applied')).toBeTruthy();
  });

  it('blocks Copy when there is nothing to copy', async () => {
    const describe = vi.fn().mockResolvedValue('');
    const { getByText } = sqlSetup([ADA], describe);
    await fireEvent.click(getByText('SQL'));
    expect((getByText('Copy') as HTMLButtonElement).disabled).toBe(true);
  });

  it('confirms a successful copy', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    });
    const describe = vi.fn().mockResolvedValue(`WHERE "name" = 'Ada'`);
    const { getByText, findByText } = sqlSetup([ADA], describe);
    await fireEvent.click(getByText('SQL'));
    await findByText(`WHERE "name" = 'Ada'`);
    await fireEvent.click(getByText('Copy'));
    expect(writeText).toHaveBeenCalledWith(`WHERE "name" = 'Ada'`);
    expect(await findByText('Copied')).toBeTruthy();
  });
});
