// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';

// Vitest globals are off in this repo, so @testing-library's auto-cleanup
// never registers itself — every component test here cleans up explicitly.
afterEach(cleanup);

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
    await fireEvent.click(getByLabelText(/Sort by email/));
    expect(onToggleSort).toHaveBeenCalledWith('1', expect.anything());
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
});

describe('sort affordances', () => {
  it('shows an ascending arrow and no badge for a single sort', () => {
    const { container, getByText } = renderHeader({
      sortDirectionOf: (id: string) => (id === '1' ? 'asc' : false),
      sortIndexOf: (id: string) => (id === '1' ? 0 : -1),
    });
    expect(getByText('▴')).toBeTruthy();
    // One key needs no ordinal — a lone "1" badge is noise.
    expect(container.querySelector('.sort-badge')).toBeNull();
  });

  it('shows order badges once a second key exists', () => {
    const { container } = renderHeader({
      // Columns render in fixed table order (id is column 0), while badges
      // carry SORT positions. Column 0 is the first key here, so the badges
      // read 1, 2 across the row.
      sortDirectionOf: (id: string) => (id === '0' ? 'asc' : 'desc'),
      sortIndexOf: (id: string) => (id === '0' ? 0 : 1),
    });
    const badges = [...container.querySelectorAll('.sort-badge')].map((b) =>
      b.textContent?.trim(),
    );
    expect(badges).toEqual(['1', '2']);
  });

  it('shows a descending arrow for a descending key', () => {
    const { getByText } = renderHeader({
      sortDirectionOf: (id: string) => (id === '0' ? 'desc' : false),
      sortIndexOf: (id: string) => (id === '0' ? 0 : -1),
    });
    expect(getByText('▾')).toBeTruthy();
  });

  // The label carries the shift-click hint, so the query is a regex — an
  // exact match can never hit once Step 3 appends the hint sentence.
  it('passes the click event through so the parent can see shiftKey', async () => {
    const onToggleSort = vi.fn();
    const { getByLabelText } = renderHeader({ onToggleSort });
    await fireEvent.click(getByLabelText(/Sort by email/), { shiftKey: true });
    expect(onToggleSort).toHaveBeenCalledWith(
      '1',
      expect.objectContaining({ shiftKey: true }),
    );
  });

  it('marks a sorted header for styling', () => {
    const { container } = renderHeader({
      sortDirectionOf: (id: string) => (id === '1' ? 'asc' : false),
      sortIndexOf: (id: string) => (id === '1' ? 0 : -1),
    });
    const ths = container.querySelectorAll('thead th');
    expect(ths[2].className).toContain('active');
    expect(ths[1].className).not.toContain('active');
  });

  it('describes multi-sort in the button label so it is discoverable', () => {
    const { getByLabelText } = renderHeader();
    // Shift-click is invisible otherwise; screen-reader users get no hint at all.
    expect(getByLabelText(/Sort by email.*shift.*additional/i)).toBeTruthy();
  });
});

describe('pinned layout', () => {
  /** The adapter syncs table atoms on the microtask queue; settle before asserting. */
  const settle = () => new Promise((r) => setTimeout(r, 0));

  it('renders three header groups', async () => {
    const { container } = renderHeader({ pinRight: ['1'] });
    await settle();
    expect(container.querySelector('.hdr-start')).toBeTruthy();
    expect(container.querySelector('.hdr-center')).toBeTruthy();
    expect(container.querySelector('.hdr-end')).toBeTruthy();
  });

  it('puts a left-pinned column in the start group', async () => {
    const { container } = renderHeader({ pinLeft: ['0'] });
    await settle();
    // The row-number gutter is also a start-group member, so match on the
    // whole group's text rather than the first cell.
    const start = [...container.querySelectorAll('.hdr-start')]
      .map((el) => el.textContent)
      .join('');
    expect(start).toContain('id');
    expect(start).not.toContain('email');
  });

  it('marks the inner edge of the start group for the shadow divider', async () => {
    const { container } = renderHeader({ pinLeft: ['0'] });
    await settle();
    expect(container.querySelector('.pinned-edge')).toBeTruthy();
  });

  it('renders no pinned-edge marker when nothing is pinned', async () => {
    const { container } = renderHeader();
    await settle();
    expect(container.querySelector('.pinned-edge')).toBeNull();
  });
});

describe('pinned offsets', () => {
  /** The adapter syncs table atoms on the microtask queue; settle before asserting. */
  const settle = () => new Promise((r) => setTimeout(r, 0));

  it('offsets the first left-pinned column past the gutter', () => {
    const { container } = renderHeader({ pinLeft: ['0'] });
    const idTh = [...container.querySelectorAll('thead th')].find((el) =>
      el.textContent?.includes('id'),
    );
    // getStart('start') alone reports 0 inside the region; the gutter th
    // renders before every start cell, so the inset must include its width.
    expect(idTh?.getAttribute('style')).toContain('left: 34px');
  });

  it('keeps the gutter header itself pinned at left 0', () => {
    const { container } = renderHeader({ pinLeft: ['0'] });
    const gutter = container.querySelector('th.row-num');
    expect(gutter?.getAttribute('style')).toContain('left: 0');
  });

  it('stacks multiple right-pinned columns cumulatively', () => {
    const { container } = renderHeader({ pinRight: ['1', '0'] });
    const rights = [...container.querySelectorAll('thead th')]
      .filter((el) => el.className.includes('hdr-end'))
      .map((el) =>
        Number(el.getAttribute('style')?.match(/right: ([\d.]+)px/)?.[1]),
      );
    // Two pinned columns, default size 150 each: the inner edge sits at 0,
    // the outer at one column's width — regardless of internal order.
    expect([...rights].sort((a, b) => a - b)).toEqual([0, 150]);
  });

  it('recomputes pinned offsets after a column resize', async () => {
    const result = renderHeader({ pinLeft: ['0', '1'] });
    const secondStart = () =>
      [...result.container.querySelectorAll('thead th')].find(
        (el) =>
          el.className.includes('hdr-start') &&
          !el.className.includes('row-num') &&
          el.textContent?.includes('email'),
      );
    // 34px gutter + first column's default 150px.
    expect(secondStart()?.getAttribute('style')).toContain('left: 184px');
    result.component.resizeTo('0', 250);
    await settle();
    expect(secondStart()?.getAttribute('style')).toContain('left: 284px');
  });
});
