// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import BodyHarness from './BodyHarness.svelte';

// vitest globals are off in this repo, so @testing-library/svelte's automatic
// cleanup never registers — clean up explicitly between cases.
afterEach(cleanup);

const COLUMNS = [
  { name: 'id', type_name: 'int4' },
  { name: 'flag', type_name: 'bool' },
];

const ROWS: unknown[][] = [
  [1, true],
  [2, false],
  [null, null],
];

function renderBody(overrides: Record<string, unknown> = {}) {
  return render(BodyHarness, {
    props: { columns: COLUMNS, rows: ROWS, pageRows: ROWS, pageOffset: 0, ...overrides },
  });
}

describe('GridBody', () => {
  it('renders one row per page row', () => {
    const { container } = renderBody();
    expect(container.querySelectorAll('tbody tr')).toHaveLength(3);
  });

  it('renders numbers verbatim through formatCell', () => {
    const { getByText } = renderBody({
      pageRows: [[4200000000000, false]],
    });
    expect(getByText('4200000000000')).toBeTruthy();
  });

  it('renders a null cell as empty with the null class', () => {
    const { container } = renderBody();
    const nullCell = container.querySelectorAll('tbody tr')[2].querySelectorAll('td')[1];
    expect(nullCell.className).toContain('cell-null');
    expect(nullCell.textContent?.trim()).toBe('');
  });

  it('renders booleans as a badge, not bare text', () => {
    const { container } = renderBody();
    expect(container.querySelectorAll('.bool-badge').length).toBeGreaterThan(0);
  });

  it('reports the absolute row index when a gutter is clicked', async () => {
    const onSelectRow = vi.fn();
    // Page 2 of a 200-row page size: local row 0 is absolute row 200.
    const { container } = renderBody({ pageOffset: 200, onSelectRow });
    await fireEvent.click(container.querySelectorAll('.gutter')[0]);
    expect(onSelectRow).toHaveBeenCalledWith(200, { extend: false, toggle: false });
  });

  it('reflects selection from the absolute index set', () => {
    const { container } = renderBody({ pageOffset: 200, selectedRows: new Set([201]) });
    const gutters = container.querySelectorAll('.gutter');
    expect(gutters[0].className).not.toContain('selected');
    expect(gutters[1].className).toContain('selected');
  });

  it('numbers rows absolutely, so page 2 starts at 201', () => {
    const { container } = renderBody({ pageOffset: 200 });
    expect(container.querySelectorAll('.gutter')[0].textContent?.trim()).toBe('201');
  });

  it('passes the column index and value to the context menu handler', async () => {
    const onCellContextMenu = vi.fn();
    const { container } = renderBody({ onCellContextMenu });
    const cell = container.querySelectorAll('tbody tr')[0].querySelectorAll('td')[1];
    await fireEvent.contextMenu(cell);
    expect(onCellContextMenu).toHaveBeenCalledWith(expect.anything(), 0, 1);
  });

  it('stripes rows by absolute index so striping survives page changes', () => {
    const { container } = renderBody({ pageOffset: 1 });
    const rows = container.querySelectorAll('tbody tr');
    expect(rows[0].className).not.toContain('even');
    expect(rows[1].className).toContain('even');
  });
});

describe('pinned layout', () => {
  /** The adapter syncs table atoms on the microtask queue; settle before asserting. */
  const settle = () => new Promise((r) => setTimeout(r, 0));

  it('renders three cell groups', async () => {
    // start/center always render; end exists once a column is pinned right.
    const { container } = renderBody({ pinRight: ['1'] });
    await settle();
    expect(container.querySelector('td.cell-start')).toBeTruthy();
    expect(container.querySelector('td.cell-center')).toBeTruthy();
    expect(container.querySelector('td.cell-end')).toBeTruthy();
  });

  it('puts a left-pinned column in the start group', async () => {
    const { container } = renderBody({ pinLeft: ['0'] });
    await settle();
    const firstRow = container.querySelectorAll('tbody tr')[0];
    const startText = [...firstRow.querySelectorAll('td.cell-start')]
      .map((td) => td.textContent)
      .join('');
    expect(startText).toContain('1');
    expect(startText).not.toContain('true');
  });

  it('marks the inner edge of the start group for the shadow divider', async () => {
    const { container } = renderBody({ pinLeft: ['0'] });
    await settle();
    expect(
      container.querySelectorAll('tbody td.pinned-edge').length,
    ).toBeGreaterThan(0);
  });

  it('renders no pinned-edge marker when nothing is pinned', async () => {
    const { container } = renderBody();
    await settle();
    expect(container.querySelector('tbody td.pinned-edge')).toBeNull();
  });

  it('keeps a pinned cell aligned with its header column', async () => {
    const { container } = renderBody({ pinLeft: ['0'] });
    await settle();
    const firstRowStart = container
      .querySelectorAll('tbody tr')[0]
      .querySelectorAll('td.cell-start');
    // The row-number gutter is also a start-group member, so the id column
    // is the second one.
    expect(firstRowStart).toHaveLength(2);
    expect(firstRowStart[1].textContent?.trim()).toBe('1');
  });
});

describe('cell selection rendering', () => {
  /** Selection state crosses table atoms synced on the microtask queue. */
  const settle = () => new Promise((r) => setTimeout(r, 0));

  it('marks selected cells', async () => {
    const { container } = renderBody({ selectCell: { rowIndex: 0, columnId: '0' } });
    await settle();
    expect(container.querySelector('td[data-selected="true"]')).toBeTruthy();
  });

  it('starts a range on mousedown', async () => {
    const onCellMouseDown = vi.fn();
    const { container } = renderBody({ onCellMouseDown });
    const cell = container.querySelectorAll('tbody td')[1];
    await fireEvent.mouseDown(cell);
    expect(onCellMouseDown).toHaveBeenCalledWith(0, '0', expect.anything());
  });

  it('extends the range on mouseenter while dragging', async () => {
    const onCellMouseEnter = vi.fn();
    const { container } = renderBody({ onCellMouseEnter });
    await fireEvent.mouseEnter(container.querySelectorAll('tbody td')[2]);
    expect(onCellMouseEnter).toHaveBeenCalledWith(0, '1');
  });

  it('gives exactly one cell a tabindex of 0 so tab reaches the grid once', async () => {
    const { container } = renderBody({ selectCell: { rowIndex: 0, columnId: '0' } });
    await settle();
    const focusable = [...container.querySelectorAll('tbody td')].filter(
      (td) => td.getAttribute('tabindex') === '0',
    );
    expect(focusable).toHaveLength(1);
  });
});

describe('pinned cell offsets', () => {
  it('offsets left-pinned cells past the gutter', () => {
    const { container } = renderBody({ pinLeft: ['0'] });
    const firstRow = container.querySelectorAll('tbody tr')[0];
    const startCells = firstRow.querySelectorAll('.cell-start');
    // [gutter, first data column]: gutter pins at 0, data clears it.
    expect(startCells[0].getAttribute('style')).toContain('left: 0');
    expect(startCells[1].getAttribute('style')).toContain('left: 44px');
  });

  it('gives stacked right-pinned cells cumulative insets', () => {
    const { container } = renderBody({ pinRight: ['1', '0'] });
    const endCells =
      container.querySelectorAll('tbody tr')[0].querySelectorAll('.cell-end');
    const rights = [...endCells].map((td) =>
      Number(td.getAttribute('style')?.match(/right: ([\d.]+)px/)?.[1]),
    );
    expect([...rights].sort((a, b) => a - b)).toEqual([0, 150]);
  });
});
