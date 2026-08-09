import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import ColumnPicker from './ColumnPicker.svelte';

const COLUMNS = [
  { name: 'id', type_name: 'int4' },
  { name: 'full_name', type_name: 'text' },
  { name: 'created_at', type_name: 'timestamptz' },
];

afterEach(cleanup);

function setup(props: Record<string, unknown> = {}) {
  const onPick = vi.fn();
  const onClose = vi.fn();
  const result = render(ColumnPicker, {
    columns: COLUMNS,
    onPick,
    onClose,
    ...props,
  });
  return { ...result, onPick, onClose };
}

describe('ColumnPicker', () => {
  it('lists every column with its type', () => {
    const { getByText } = setup();
    expect(getByText('full_name')).toBeTruthy();
    expect(getByText('timestamptz')).toBeTruthy();
  });

  it('reports the picked column name', async () => {
    const { getByText, onPick, onClose } = setup();
    await fireEvent.click(getByText('full_name'));
    expect(onPick).toHaveBeenCalledWith('full_name');
    expect(onClose).toHaveBeenCalled();
  });

  it('narrows the list as you type, matching anywhere in the name', async () => {
    const { getByPlaceholderText, queryByText } = setup();
    await fireEvent.input(getByPlaceholderText('Search columns…'), {
      target: { value: 'name' },
    });
    expect(queryByText('full_name')).toBeTruthy();
    expect(queryByText('created_at')).toBeNull();
  });

  it('matches case-insensitively', async () => {
    const { getByPlaceholderText, queryByText } = setup();
    await fireEvent.input(getByPlaceholderText('Search columns…'), {
      target: { value: 'NAME' },
    });
    expect(queryByText('full_name')).toBeTruthy();
  });

  it('picks the first match on Enter', async () => {
    const { getByPlaceholderText, onPick } = setup();
    const input = getByPlaceholderText('Search columns…');
    await fireEvent.input(input, { target: { value: 'created' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    expect(onPick).toHaveBeenCalledWith('created_at');
  });

  it('does nothing on Enter when nothing matches', async () => {
    const { getByPlaceholderText, onPick } = setup();
    const input = getByPlaceholderText('Search columns…');
    await fireEvent.input(input, { target: { value: 'zzz' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    expect(onPick).not.toHaveBeenCalled();
  });

  it('shows a message when nothing matches', async () => {
    const { getByPlaceholderText, getByText } = setup();
    await fireEvent.input(getByPlaceholderText('Search columns…'), {
      target: { value: 'zzz' },
    });
    expect(getByText('No matching columns')).toBeTruthy();
  });

  it('closes on Escape', async () => {
    const { getByPlaceholderText, onClose } = setup();
    await fireEvent.keyDown(getByPlaceholderText('Search columns…'), {
      key: 'Escape',
    });
    expect(onClose).toHaveBeenCalled();
  });

  // The picker is positioned from viewport coordinates, which is only coherent
  // with position: fixed — it shipped as position: absolute and was clipped by
  // .results-grid's overflow: hidden. The flip/clamp maths is covered in
  // anchor.test.ts; this checks the coordinates actually reach the element.
  it('positions itself from the supplied viewport rect', () => {
    const { container } = setup({
      anchorRect: { top: 100, bottom: 124, left: 260 },
    });
    const picker = container.querySelector('.column-picker') as HTMLElement;
    expect(picker.style.left).toBe('260px');
    expect(picker.style.top).toBe('128px');
  });

  it('marks the first column active on open', () => {
    const { getByRole } = setup();
    expect(getByRole('option', { selected: true }).textContent).toContain('id');
  });

  it('moves the active option with the arrow keys', async () => {
    const { getByPlaceholderText, getByRole } = setup();
    const input = getByPlaceholderText('Search columns…');
    await fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(getByRole('option', { selected: true }).textContent).toContain(
      'full_name',
    );
    await fireEvent.keyDown(input, { key: 'ArrowUp' });
    expect(getByRole('option', { selected: true }).textContent).toContain('id');
  });

  it('wraps from the last option back to the first', async () => {
    const { getByPlaceholderText, getByRole } = setup();
    const input = getByPlaceholderText('Search columns…');
    for (let i = 0; i < 3; i++) {
      await fireEvent.keyDown(input, { key: 'ArrowDown' });
    }
    expect(getByRole('option', { selected: true }).textContent).toContain('id');
  });

  it('picks the arrowed-to option on Enter, not merely the first match', async () => {
    const { getByPlaceholderText, onPick } = setup();
    const input = getByPlaceholderText('Search columns…');
    await fireEvent.keyDown(input, { key: 'ArrowDown' });
    await fireEvent.keyDown(input, { key: 'Enter' });
    expect(onPick).toHaveBeenCalledWith('full_name');
  });

  it('jumps to the last option on End and the first on Home', async () => {
    const { getByPlaceholderText, getByRole } = setup();
    const input = getByPlaceholderText('Search columns…');
    await fireEvent.keyDown(input, { key: 'End' });
    expect(getByRole('option', { selected: true }).textContent).toContain(
      'created_at',
    );
    await fireEvent.keyDown(input, { key: 'Home' });
    expect(getByRole('option', { selected: true }).textContent).toContain('id');
  });

  it('resets the active option when the query narrows the list', async () => {
    const { getByPlaceholderText, getByRole } = setup();
    const input = getByPlaceholderText('Search columns…');
    await fireEvent.keyDown(input, { key: 'End' });
    await fireEvent.input(input, { target: { value: 'name' } });
    expect(getByRole('option', { selected: true }).textContent).toContain(
      'full_name',
    );
  });

  it('reports the match count so the list is countable without sight', () => {
    const { getByText } = setup();
    expect(getByText(/3 columns/)).toBeTruthy();
  });

  it('names the unmatched query back to the user', async () => {
    const { getByPlaceholderText, getByText } = setup();
    await fireEvent.input(getByPlaceholderText('Search columns…'), {
      target: { value: 'zzz' },
    });
    expect(getByText(/Nothing matches/)).toBeTruthy();
  });

  it('closes on viewport resize rather than floating at a stale position', async () => {
    const { onClose } = setup();
    await fireEvent.resize(window);
    expect(onClose).toHaveBeenCalled();
  });
});
