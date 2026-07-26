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
