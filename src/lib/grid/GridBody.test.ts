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
    props: {
      columns: COLUMNS,
      rows: ROWS,
      pageRows: ROWS,
      pageOffset: 0,
      ...overrides,
    },
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
    const nullCell = container
      .querySelectorAll('tbody tr')[2]
      .querySelectorAll('td')[1];
    expect(nullCell.className).toContain('cell-null');
    expect(nullCell.textContent?.trim()).toBe('');
  });

  it('renders booleans as plain text, not a status badge', () => {
    // A tinted pill claimed a significance `false` does not have; in a
    // database grid a boolean is just a value.
    const { container, getByText } = renderBody();
    expect(container.querySelectorAll('.bool-badge')).toHaveLength(0);
    expect(getByText('true')).toBeTruthy();
    expect(getByText('false')).toBeTruthy();
  });

  it('fills a lone selected cell solid and leaves a range washed', () => {
    const solo = renderBody({ selBounds: { r0: 0, r1: 0, c0: 0, c1: 0 } });
    const cells = solo.container.querySelectorAll('tbody td:not(.row-num)');
    expect(cells[0].className).toContain('sel-solid');
    cleanup();

    const range = renderBody({ selBounds: { r0: 0, r1: 1, c0: 0, c1: 1 } });
    const ranged = range.container.querySelectorAll('tbody td:not(.row-num)');
    expect(ranged[0].className).toContain('sel');
    expect(ranged[0].className).not.toContain('sel-solid');
  });

  it('draws the range marquee only on its outer edges', () => {
    const { container } = renderBody({
      selBounds: { r0: 0, r1: 1, c0: 0, c1: 1 },
    });
    const cells = [
      ...container.querySelectorAll('tbody td:not(.row-num)'),
    ] as HTMLElement[];
    // Top-left of a 2x2: top and left only, never bottom or right.
    expect(cells[0].style.boxShadow).toContain('inset 0 1px 0 0');
    expect(cells[0].style.boxShadow).toContain('inset 1px 0 0 0');
    expect(cells[0].style.boxShadow).not.toContain('inset -1px 0 0 0');
  });

  it('marks no cell selected when there is no selection', () => {
    const { container } = renderBody();
    expect(container.querySelectorAll('td.sel')).toHaveLength(0);
  });

  it('reports the cell coordinates on mousedown', async () => {
    const onCellMouseDown = vi.fn();
    const { container } = renderBody({ onCellMouseDown });
    const cells = container.querySelectorAll('tbody td:not(.row-num)');
    await fireEvent.mouseDown(cells[1]);
    expect(onCellMouseDown).toHaveBeenCalledWith(expect.anything(), 0, 1);
  });

  it('reports hover coordinates so a drag can extend the range', async () => {
    const onCellMouseEnter = vi.fn();
    const { container } = renderBody({ onCellMouseEnter });
    const cells = container.querySelectorAll('tbody td:not(.row-num)');
    await fireEvent.mouseEnter(cells[2]);
    expect(onCellMouseEnter).toHaveBeenCalledWith(1, 0);
  });

  it('reports the absolute row index when a checkbox is toggled', async () => {
    const onToggleCheck = vi.fn();
    // Page 2 of a 200-row page size: local row 0 is absolute row 200.
    const { container } = renderBody({ pageOffset: 200, onToggleCheck });
    const box = container.querySelectorAll('tbody input[type=checkbox]')[0];
    await fireEvent.change(box);
    expect(onToggleCheck).toHaveBeenCalledWith(200);
  });

  it('reflects checked state from the absolute index set', () => {
    const { container } = renderBody({
      pageOffset: 200,
      checkedRows: new Set([201]),
    });
    const boxes = container.querySelectorAll('tbody input[type=checkbox]');
    expect((boxes[0] as HTMLInputElement).checked).toBe(false);
    expect((boxes[1] as HTMLInputElement).checked).toBe(true);
  });

  it('passes the column index and value to the context menu handler', async () => {
    const onCellContextMenu = vi.fn();
    const { container } = renderBody({ onCellContextMenu });
    const cell = container
      .querySelectorAll('tbody tr')[0]
      .querySelectorAll('td')[1];
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
