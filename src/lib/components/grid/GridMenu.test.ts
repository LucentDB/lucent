import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import GridMenu from './GridMenu.svelte';

afterEach(cleanup);

const ITEMS = [
  { id: 'asc', label: 'Sort ascending' },
  { separator: true },
  { id: 'filter', label: 'Filter by this column' },
  { id: 'nope', label: 'Disabled thing', disabled: true },
];

function setup(overrides = {}) {
  const onSelect = vi.fn();
  const onClose = vi.fn();
  const result = render(GridMenu, {
    x: 10,
    y: 20,
    items: ITEMS,
    onSelect,
    onClose,
    ...overrides,
  });
  return { ...result, onSelect, onClose };
}

describe('GridMenu', () => {
  it('renders each item label', () => {
    const { getByText } = setup();
    expect(getByText('Sort ascending')).toBeTruthy();
    expect(getByText('Filter by this column')).toBeTruthy();
  });

  it('reports the selected item id and closes', async () => {
    const { getByText, onSelect, onClose } = setup();
    await fireEvent.click(getByText('Filter by this column'));
    expect(onSelect).toHaveBeenCalledWith('filter');
    expect(onClose).toHaveBeenCalled();
  });

  it('ignores clicks on a disabled item', async () => {
    const { getByText, onSelect } = setup();
    await fireEvent.click(getByText('Disabled thing'));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('closes on Escape', async () => {
    const { onClose } = setup();
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });

  it('selects the focused item on Enter after arrowing down', async () => {
    const { onSelect } = setup();
    await fireEvent.keyDown(window, { key: 'ArrowDown' });
    await fireEvent.keyDown(window, { key: 'Enter' });
    expect(onSelect).toHaveBeenCalledWith('asc');
  });

  it('skips separators and disabled items when arrowing', async () => {
    const { onSelect } = setup();
    await fireEvent.keyDown(window, { key: 'ArrowDown' });
    await fireEvent.keyDown(window, { key: 'ArrowDown' });
    await fireEvent.keyDown(window, { key: 'Enter' });
    expect(onSelect).toHaveBeenCalledWith('filter');
  });

  it('exposes the menu role for assistive technology', () => {
    const { getByRole } = setup();
    expect(getByRole('menu')).toBeTruthy();
  });

  it('moves real DOM focus onto the item as you arrow, not just a highlight', async () => {
    const { getByText } = setup();
    await fireEvent.keyDown(window, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(getByText('Sort ascending'));
  });

  it('wraps to the last selectable item when arrowing up from the start', async () => {
    const { getByText } = setup();
    await fireEvent.keyDown(window, { key: 'ArrowUp' });
    expect(document.activeElement).toBe(getByText('Filter by this column'));
  });

  it('jumps to the first item on Home and the last on End', async () => {
    const { getByText } = setup();
    await fireEvent.keyDown(window, { key: 'End' });
    expect(document.activeElement).toBe(getByText('Filter by this column'));
    await fireEvent.keyDown(window, { key: 'Home' });
    expect(document.activeElement).toBe(getByText('Sort ascending'));
  });

  it('does not select twice when Enter fires on an already-focused item', async () => {
    const { getByText, onSelect } = setup();
    await fireEvent.keyDown(window, { key: 'ArrowDown' });
    // Enter on the focused button: the native click is what should select.
    await fireEvent.keyDown(getByText('Sort ascending'), { key: 'Enter' });
    await fireEvent.click(getByText('Sort ascending'));
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it('restores focus to the trigger when it closes', async () => {
    const trigger = document.createElement('button');
    document.body.appendChild(trigger);
    trigger.focus();

    const { unmount } = setup();
    await fireEvent.keyDown(window, { key: 'ArrowDown' });
    expect(document.activeElement).not.toBe(trigger);

    unmount();
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });

  it('closes on viewport resize rather than floating at a stale position', async () => {
    const { onClose } = setup();
    await fireEvent.resize(window);
    expect(onClose).toHaveBeenCalled();
  });
});
