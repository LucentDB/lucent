import { describe, it, expect, afterEach, vi } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import ExportDropdown from './ExportDropdown.svelte';

afterEach(cleanup);

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

describe('ExportDropdown', () => {
  it('renders button with accessible ARIA attributes', () => {
    const { getByRole } = render(ExportDropdown, {
      disabled: false,
      columns: [{ name: 'id', typeName: 'int4' }],
      rows: [[1]],
    });

    const button = getByRole('button', { name: 'Export results' });
    expect(button).toBeDefined();
    expect(button.getAttribute('aria-haspopup')).toBe('true');
    expect(button.getAttribute('aria-expanded')).toBe('false');
  });

  it('toggles aria-expanded state when clicked', async () => {
    const { getByRole } = render(ExportDropdown, {
      disabled: false,
      columns: [{ name: 'id', typeName: 'int4' }],
      rows: [[1]],
    });

    const button = getByRole('button', { name: 'Export results' });
    expect(button.getAttribute('aria-expanded')).toBe('false');

    await fireEvent.click(button);
    expect(button.getAttribute('aria-expanded')).toBe('true');

    await fireEvent.click(button);
    expect(button.getAttribute('aria-expanded')).toBe('false');
  });
});
