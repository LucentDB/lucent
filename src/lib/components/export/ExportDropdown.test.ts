// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import ExportDropdown from './ExportDropdown.svelte';

afterEach(cleanup);

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

describe('ExportDropdown', () => {
  it('renders export button with accessible label and collapsed state', () => {
    const { getByRole } = render(ExportDropdown, {
      props: { disabled: false, columns: [], rows: [] },
    });
    const btn = getByRole('button', { name: 'Export results' });
    expect(btn).toBeTruthy();
    expect(btn.getAttribute('aria-expanded')).toBe('false');
  });

  it('opens menu with proper roles when clicked', async () => {
    const { getByRole, getAllByRole } = render(ExportDropdown, {
      props: { disabled: false, columns: [], rows: [] },
    });
    const btn = getByRole('button', { name: 'Export results' });
    await fireEvent.click(btn);

    expect(btn.getAttribute('aria-expanded')).toBe('true');
    const menu = getByRole('menu');
    expect(menu).toBeTruthy();

    const items = getAllByRole('menuitem');
    expect(items.length).toBe(3);
    expect(items[0].textContent).toContain('Export as CSV');
  });

  it('closes menu when clicking backdrop', async () => {
    const { getByRole, queryByRole } = render(ExportDropdown, {
      props: { disabled: false, columns: [], rows: [] },
    });
    const btn = getByRole('button', { name: 'Export results' });
    await fireEvent.click(btn);

    const backdrop = getByRole('button', { name: 'Close export options' });
    await fireEvent.click(backdrop);

    expect(btn.getAttribute('aria-expanded')).toBe('false');
    expect(queryByRole('menu')).toBeNull();
  });
});
