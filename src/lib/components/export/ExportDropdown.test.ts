// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/svelte';
import ExportDropdown from './ExportDropdown.svelte';

afterEach(cleanup);

describe('ExportDropdown accessibility and interaction', () => {
  it('has correct ARIA attributes on trigger button', () => {
    render(ExportDropdown, { disabled: false, columns: [], rows: [] });

    const btn = screen.getByRole('button', { name: 'Export results' });
    expect(btn).toBeTruthy();
    expect(btn.getAttribute('aria-expanded')).toBe('false');
    expect(btn.getAttribute('aria-haspopup')).toBe('menu');
  });

  it('opens menu on click and updates aria-expanded', async () => {
    render(ExportDropdown, { disabled: false, columns: [], rows: [] });

    const btn = screen.getByRole('button', { name: 'Export results' });
    await fireEvent.click(btn);

    expect(btn.getAttribute('aria-expanded')).toBe('true');
    const menu = screen.getByRole('menu');
    expect(menu).toBeTruthy();

    const items = screen.getAllByRole('menuitem');
    expect(items.length).toBe(3);
    expect(items[0].textContent).toContain('Export as CSV');
    expect(items[1].textContent).toContain('Export as JSON');
    expect(items[2].textContent).toContain('Export as INSERTs');
  });

  it('closes menu on Escape keypress', async () => {
    render(ExportDropdown, { disabled: false, columns: [], rows: [] });

    const btn = screen.getByRole('button', { name: 'Export results' });
    await fireEvent.click(btn);
    expect(screen.getByRole('menu')).toBeTruthy();

    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(btn.getAttribute('aria-expanded')).toBe('false');
  });
});
