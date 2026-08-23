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

  it('reads as a sentence: column, then operator', () => {
    const { getByText, getByLabelText } = setup(TEXT_FILTER);
    expect(getByText('name')).toBeTruthy();
    expect(getByLabelText('Filter operator for name').textContent).toContain(
      'contains',
    );
  });

  it('offers the type-appropriate operators in the operator menu', async () => {
    const { getByLabelText, getByRole, queryByRole } = setup(TEXT_FILTER);
    await fireEvent.click(getByLabelText('Filter operator for name'));
    expect(getByRole('menuitem', { name: 'does not contain' })).toBeTruthy();
    expect(queryByRole('menuitem', { name: '>' })).toBeNull();
  });

  it('offers comparison operators for a numeric column', async () => {
    const { getByLabelText, getByRole } = setup(
      { id: 'a', column: 'age', operator: 'eq', value: '' },
      'int4',
    );
    await fireEvent.click(getByLabelText('Filter operator for age'));
    expect(getByRole('menuitem', { name: '≥' })).toBeTruthy();
  });

  it('labels the same operator in the language of the column type', () => {
    const { getByLabelText } = setup(
      { id: 'a', column: 'created_at', operator: 'gt', value: '' },
      'timestamptz',
    );
    expect(
      getByLabelText('Filter operator for created_at').textContent,
    ).toContain('after');
  });

  it('uses a menu rather than a native select, for consistent chrome', () => {
    const { queryByRole, getByLabelText } = setup(TEXT_FILTER);
    expect(queryByRole('combobox')).toBeNull();
    expect(
      getByLabelText('Filter operator for name').getAttribute('aria-haspopup'),
    ).toBe('menu');
  });

  it('reports the menu open state to assistive technology', async () => {
    const { getByLabelText } = setup(TEXT_FILTER);
    const trigger = getByLabelText('Filter operator for name');
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    await fireEvent.click(trigger);
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
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
    const { getByLabelText, getByRole, onChange, onCommit } =
      setup(TEXT_FILTER);
    await fireEvent.click(getByLabelText('Filter operator for name'));
    await fireEvent.click(getByRole('menuitem', { name: 'is null' }));
    expect(onChange).toHaveBeenCalledWith({ operator: 'null' });
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it('closes the operator menu once a choice is made', async () => {
    const { getByLabelText, getByRole, queryByRole } = setup(TEXT_FILTER);
    await fireEvent.click(getByLabelText('Filter operator for name'));
    await fireEvent.click(getByRole('menuitem', { name: 'is null' }));
    expect(queryByRole('menu')).toBeNull();
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

  it('sizes the value input to its content instead of truncating', () => {
    const short = setup({ ...TEXT_FILTER, value: 'Ada' });
    const shortWidth = (short.getByRole('textbox') as HTMLElement).style.width;
    cleanup();

    const long = setup({
      ...TEXT_FILTER,
      value: 'a-considerably-longer-value',
    });
    const longWidth = (long.getByRole('textbox') as HTMLElement).style.width;

    expect(parseInt(longWidth, 10)).toBeGreaterThan(parseInt(shortWidth, 10));
  });

  it('caps the value input so one long filter cannot dominate the bar', () => {
    const { getByRole } = setup({ ...TEXT_FILTER, value: 'x'.repeat(400) });
    expect(
      parseInt((getByRole('textbox') as HTMLElement).style.width, 10),
    ).toBe(24);
  });

  it('keeps a floor width so an empty chip is still clickable', () => {
    const { getByRole } = setup(TEXT_FILTER);
    expect(
      parseInt((getByRole('textbox') as HTMLElement).style.width, 10),
    ).toBe(5);
  });
});
