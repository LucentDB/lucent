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

  it('reports the absolute row index when a checkbox is toggled', async () => {
    const onToggleCheck = vi.fn();
    // Page 2 of a 200-row page size: local row 0 is absolute row 200.
    const { container } = renderBody({ pageOffset: 200, onToggleCheck });
    const box = container.querySelectorAll('tbody input[type=checkbox]')[0];
    await fireEvent.change(box);
    expect(onToggleCheck).toHaveBeenCalledWith(200);
  });

  it('reflects checked state from the absolute index set', () => {
    const { container } = renderBody({ pageOffset: 200, checkedRows: new Set([201]) });
    const boxes = container.querySelectorAll('tbody input[type=checkbox]');
    expect((boxes[0] as HTMLInputElement).checked).toBe(false);
    expect((boxes[1] as HTMLInputElement).checked).toBe(true);
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
