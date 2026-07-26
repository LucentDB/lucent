import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import FilterChip from './FilterChip.svelte';

afterEach(cleanup);

function setup(
  filter: {
    id: string;
    column: string;
    operator: string;
    value: string | null;
  },
  typeName = 'text',
) {
  const onChange = vi.fn();
  const onCommit = vi.fn();
  const onRemove = vi.fn();
  const result = render(FilterChip, {
    filter,
    typeName,
    onChange,
    onCommit,
    onRemove,
  });
  return { ...result, onChange, onCommit, onRemove };
}

const TEXT_FILTER = {
  id: 'a',
  column: 'name',
  operator: 'contains',
  value: '',
};

describe('FilterChip', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('shows the column name and the operator options for the type', () => {
    const { getByText, getByRole } = setup(TEXT_FILTER);
    expect(getByText('name')).toBeTruthy();
    const select = getByRole('combobox') as HTMLSelectElement;
    const values = Array.from(select.options).map(
      (o: HTMLOptionElement) => o.value,
    );
    expect(values).toContain('ncontains');
    expect(values).not.toContain('gt');
  });

  it('offers comparison operators for a numeric column', () => {
    const { getByRole } = setup(
      { id: 'a', column: 'age', operator: 'eq', value: '' },
      'int4',
    );
    const select = getByRole('combobox') as HTMLSelectElement;
    expect(
      Array.from(select.options).map((o: HTMLOptionElement) => o.value),
    ).toContain('gte');
  });

  it('hides the value input for an operator that needs no value', () => {
    const { queryByRole } = setup({
      id: 'a',
      column: 'deleted_at',
      operator: 'null',
      value: null,
    });
    expect(queryByRole('textbox')).toBeNull();
  });

  it('reports each keystroke immediately but commits only after the debounce', async () => {
    const { getByRole, onChange, onCommit } = setup(TEXT_FILTER);
    await fireEvent.input(getByRole('textbox'), { target: { value: 'Ada' } });
    expect(onChange).toHaveBeenCalledWith({ value: 'Ada' });
    expect(onCommit).not.toHaveBeenCalled();
    vi.advanceTimersByTime(275);
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it('collapses rapid keystrokes into a single commit', async () => {
    const { getByRole, onCommit } = setup(TEXT_FILTER);
    const input = getByRole('textbox');
    for (const v of ['A', 'Ad', 'Ada']) {
      await fireEvent.input(input, { target: { value: v } });
      vi.advanceTimersByTime(100);
    }
    vi.advanceTimersByTime(275);
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it('commits immediately on Enter without waiting for the debounce', async () => {
    const { getByRole, onCommit } = setup(TEXT_FILTER);
    const input = getByRole('textbox');
    await fireEvent.input(input, { target: { value: 'Ada' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    expect(onCommit).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(275);
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it('commits an operator change at once', async () => {
    const { getByRole, onChange, onCommit } = setup(TEXT_FILTER);
    await fireEvent.change(getByRole('combobox'), {
      target: { value: 'null' },
    });
    expect(onChange).toHaveBeenCalledWith({ operator: 'null' });
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it('removes the chip on Escape when it has no value yet', async () => {
    const { getByRole, onRemove } = setup(TEXT_FILTER);
    await fireEvent.keyDown(getByRole('textbox'), { key: 'Escape' });
    expect(onRemove).toHaveBeenCalled();
  });

  it('reverts rather than removes on Escape when a value is present', async () => {
    const filter = {
      id: 'a',
      column: 'name',
      operator: 'contains',
      value: 'Ada',
    };
    const { getByRole, onRemove, onChange } = setup(filter);
    const input = getByRole('textbox');
    await fireEvent.input(input, { target: { value: 'AdaX' } });
    onChange.mockClear();
    await fireEvent.keyDown(input, { key: 'Escape' });
    expect(onRemove).not.toHaveBeenCalled();
    expect(onChange).toHaveBeenCalledWith({ value: 'Ada' });
  });

  it('reports removal when the remove button is clicked', async () => {
    const { getByLabelText, onRemove } = setup(TEXT_FILTER);
    await fireEvent.click(getByLabelText('Remove filter on name'));
    expect(onRemove).toHaveBeenCalled();
  });

  it('marks an incomplete chip as pending for styling and assistive tech', () => {
    const { container } = setup(TEXT_FILTER);
    expect(container.querySelector('.filter-chip.pending')).toBeTruthy();
  });

  it('does not mark a complete chip as pending', () => {
    const { container } = setup({ ...TEXT_FILTER, value: 'Ada' });
    expect(container.querySelector('.filter-chip.pending')).toBeNull();
  });
});
