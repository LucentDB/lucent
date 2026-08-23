// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';

// Vitest globals are off in this repo, so @testing-library's auto-cleanup
// never registers itself — every component test here cleans up explicitly.
afterEach(cleanup);
import GridHeader from './GridHeader.svelte';
import { createGridEngine } from './engine.svelte.ts';

const COLUMNS = [
  { name: 'id', type_name: 'int4' },
  { name: 'email', type_name: 'text' },
];
const ROWS: unknown[][] = [[1, 'a@x.com']];

/**
 * GridHeader takes a live table instance, and createGridEngine must run inside
 * an effect owner. A harness component is the only honest way to build one for
 * a render test, so the engine is created in HeaderHarness.svelte instead.
 */
function renderHeader(overrides: Record<string, unknown> = {}) {
  return render(HeaderHarness, {
    props: { columns: COLUMNS, rows: ROWS, ...overrides },
  });
}

import HeaderHarness from './HeaderHarness.svelte';

describe('GridHeader', () => {
  it('renders one header cell per column plus the row-number gutter', () => {
    const { container } = renderHeader();
    expect(container.querySelectorAll('thead th')).toHaveLength(3);
    expect(container.querySelector('th.row-num')).toBeTruthy();
  });

  it('shows the column name and type', () => {
    const { getByText } = renderHeader();
    expect(getByText('email')).toBeTruthy();
    expect(getByText('text')).toBeTruthy();
  });

  it('calls onToggleSort with the column id when the header is clicked', async () => {
    const onToggleSort = vi.fn();
    const { getByLabelText } = renderHeader({ onToggleSort });
    await fireEvent.click(getByLabelText('Sort by email'));
    expect(onToggleSort).toHaveBeenCalledWith('1');
  });

  it('calls onOpenMenu from the column actions button', async () => {
    const onOpenMenu = vi.fn();
    const { getByLabelText } = renderHeader({ onOpenMenu });
    await fireEvent.click(getByLabelText('Column actions for email'));
    expect(onOpenMenu).toHaveBeenCalled();
  });

  it('renders a focusable resize handle per column', () => {
    const { container } = renderHeader();
    expect(container.querySelectorAll('.resize-handle')).toHaveLength(2);
  });

  it('applies the supplied width to each header cell', () => {
    const { container } = renderHeader({ columnWidths: { 0: 250 } });
    const first = container.querySelectorAll('thead th')[1] as HTMLElement;
    expect(first.style.width).toBe('250px');
  });

  it('renders the sort indicator the parent supplies', () => {
    const { getByText } = renderHeader({
      sortIndicatorFor: (id: string) => (id === '1' ? ' ▴' : ''),
    });
    expect(getByText(/email ▴/)).toBeTruthy();
  });
});
