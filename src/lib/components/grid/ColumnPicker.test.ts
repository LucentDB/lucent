import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import ColumnPicker from './ColumnPicker.svelte';

const COLUMNS = [
  { name: 'id', type_name: 'int4' },
  { name: 'full_name', type_name: 'text' },
  { name: 'created_at', type_name: 'timestamptz' },
];

afterEach(cleanup);

function setup() {
  const onPick = vi.fn();
  const onClose = vi.fn();
  const result = render(ColumnPicker, { columns: COLUMNS, onPick, onClose });
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
});
