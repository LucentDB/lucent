// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/svelte';

import ExportDropdown from './ExportDropdown.svelte';

afterEach(cleanup);

describe('ExportDropdown accessibility', () => {
  it('has accessible name and expanded state for the toggle button', async () => {
    render(ExportDropdown, {
      disabled: false,
      columns: [{ name: 'id', typeName: 'int4' }],
      rows: [[1]],
    });

    const button = screen.getByRole('button', { name: 'Export results' });
    expect(button).toBeTruthy();
    expect(button.getAttribute('aria-expanded')).toBe('false');

    await fireEvent.click(button);

    expect(button.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByRole('menu')).toBeTruthy();

    const menuItems = screen.getAllByRole('menuitem');
    expect(menuItems.length).toBe(3);
  });
});
