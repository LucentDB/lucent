// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/svelte';
import CopyDropdown from './CopyDropdown.svelte';

afterEach(cleanup);

describe('CopyDropdown accessibility and interaction', () => {
  it('has correct ARIA attributes on trigger button', () => {
    render(CopyDropdown, { disabled: false, columns: [], rows: [] });

    const btn = screen.getByRole('button', { name: 'Copy results' });
    expect(btn).toBeTruthy();
    expect(btn.getAttribute('aria-expanded')).toBe('false');
    expect(btn.getAttribute('aria-haspopup')).toBe('menu');
  });

  it('opens menu on click and updates aria-expanded', async () => {
    render(CopyDropdown, { disabled: false, columns: [], rows: [] });

    const btn = screen.getByRole('button', { name: 'Copy results' });
    await fireEvent.click(btn);

    expect(btn.getAttribute('aria-expanded')).toBe('true');
    const menu = screen.getByRole('menu');
    expect(menu).toBeTruthy();

    const items = screen.getAllByRole('menuitem');
    expect(items.length).toBe(3);
    expect(items[0].textContent).toContain('Copy as CSV');
    expect(items[1].textContent).toContain('Copy as JSON');
    expect(items[2].textContent).toContain('Copy as INSERTs');
  });

  it('closes menu on Escape keypress', async () => {
    render(CopyDropdown, { disabled: false, columns: [], rows: [] });

    const btn = screen.getByRole('button', { name: 'Copy results' });
    await fireEvent.click(btn);
    expect(screen.getByRole('menu')).toBeTruthy();

    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(btn.getAttribute('aria-expanded')).toBe('false');
  });

  it('closes menu when clicking the backdrop', async () => {
    render(CopyDropdown, { disabled: false, columns: [], rows: [] });

    const btn = screen.getByRole('button', { name: 'Copy results' });
    await fireEvent.click(btn);

    const backdrop = screen.getByRole('button', {
      name: 'Close copy options',
    });
    await fireEvent.click(backdrop);

    expect(btn.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByRole('menu')).toBeNull();
  });
});
