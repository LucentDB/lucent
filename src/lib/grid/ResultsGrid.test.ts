// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import ResultsGrid from './ResultsGrid.svelte';

// Minimal props the grid needs to mount.
const props = {
  columns: [{ name: 'a', dataType: 'int' }],
  rows: [[1]],
  pageRows: [[1]],
  columnWidths: {},
  page: 1,
  pageSize: 100,
  totalRows: 1,
  rowCount: 1,
  truncated: false,
};

afterEach(() => {
  document.body.style.cursor = '';
  document.body.style.userSelect = '';
  vi.restoreAllMocks();
});

describe('resizer listener teardown', () => {
  it('removes document listeners when unmounted mid-drag', async () => {
    const { container, unmount } = render(ResultsGrid, props);
    // Start a column drag on the resize handle inside the header cell.
    const handle = container.querySelector('.resize-handle');
    expect(handle).toBeTruthy();
    await fireEvent.mouseDown(handle as HTMLButtonElement, { clientX: 100 });
    // Drag is now active: the body cursor must reflect the resize state.
    expect(document.body.style.cursor).toBe('col-resize');
    // Unmount while the drag is still active (listeners attached).
    unmount();
    // A stray mousemove after unmount must not touch the document body —
    // if the listener leaked, this would keep resizing (and leave the cursor
    // stuck at col-resize).
    document.dispatchEvent(new MouseEvent('mousemove', { clientX: 300 }));
    expect(document.body.style.cursor).toBe('');
  });
});
