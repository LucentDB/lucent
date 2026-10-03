import { describe, test, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import AddCellButton from './AddCellButton.svelte';

afterEach(() => {
  cleanup();
});

describe('AddCellButton', () => {
  test('renders toggle button with accessible label', () => {
    const { getByRole } = render(AddCellButton);
    const btn = getByRole('button', { name: 'Add cell' });
    expect(btn).toBeDefined();
    expect(btn.getAttribute('aria-expanded')).toBe('false');
  });

  test('opens menu popup on click with tabindex="0"', async () => {
    const { getByRole } = render(AddCellButton);
    const btn = getByRole('button', { name: 'Add cell' });
    await fireEvent.click(btn);

    expect(btn.getAttribute('aria-expanded')).toBe('true');
    const menu = getByRole('menu');
    expect(menu).toBeDefined();
    expect(menu.getAttribute('tabindex')).toBe('0');
  });

  test('calls onAdd when an option is clicked', async () => {
    const onAdd = vi.fn();
    const { getByRole, getByText } = render(AddCellButton, {
      props: { onAdd },
    });

    const btn = getByRole('button', { name: 'Add cell' });
    await fireEvent.click(btn);

    const sqlOption = getByText('SQL Cell');
    await fireEvent.click(sqlOption);

    expect(onAdd).toHaveBeenCalledWith('sql');
  });

  test('navigates options with arrow keys and escape', async () => {
    const { getByRole, getAllByRole } = render(AddCellButton);
    const btn = getByRole('button', { name: 'Add cell' });
    await fireEvent.click(btn);

    const menu = getByRole('menu');
    const options = getAllByRole('menuitem');

    // ArrowDown from menu container
    await fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(options[0]);

    // ArrowDown to second option
    await fireEvent.keyDown(options[0], { key: 'ArrowDown' });
    expect(document.activeElement).toBe(options[1]);

    // Escape closes menu
    await fireEvent.keyDown(options[1], { key: 'Escape' });
    expect(btn.getAttribute('aria-expanded')).toBe('false');
  });
});
