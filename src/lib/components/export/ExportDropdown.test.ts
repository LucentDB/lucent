import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import ExportDropdown from './ExportDropdown.svelte';

describe('ExportDropdown accessibility and keyboard interactions', () => {
  afterEach(() => {
    cleanup();
  });

  it('renders export button with accessible label and correct expanded state', async () => {
    render(ExportDropdown, { disabled: false });

    const btn = screen.getByRole('button', { name: 'Export results' });
    expect(btn).not.toBeNull();
    expect(btn.getAttribute('aria-expanded')).toBe('false');

    await fireEvent.click(btn);
    expect(btn.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByText('Export as CSV')).not.toBeNull();
  });

  it('closes dropdown when Escape key is pressed', async () => {
    const { container } = render(ExportDropdown, { disabled: false });

    const btn = screen.getByRole('button', { name: 'Export results' });
    await fireEvent.click(btn);
    expect(screen.getByText('Export as CSV')).not.toBeNull();

    const dropdown = container.querySelector('.export-dropdown');
    expect(dropdown).not.toBeNull();
    if (dropdown) {
      await fireEvent.keyDown(dropdown, { key: 'Escape' });
    }

    expect(screen.queryByText('Export as CSV')).toBeNull();
    expect(btn.getAttribute('aria-expanded')).toBe('false');
  });
});
