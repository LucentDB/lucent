// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import ResultsGrid from './ResultsGrid.svelte';

// vitest globals are off in this repo, so @testing-library/svelte's automatic
// cleanup never registers — same hygiene as GridGutter.test.ts.
afterEach(cleanup);

const writeText = vi.fn(() => Promise.resolve());

beforeEach(() => {
  writeText.mockClear();
  Object.assign(navigator, { clipboard: { writeText } });
});

const props = {
  columns: [
    { name: 'id', type_name: 'int4' },
    { name: 'email', type_name: 'text' },
  ],
  rows: [
    [1, 'a@x.com'],
    [2, 'b@x.com'],
  ],
  fetchedCount: 2,
  isEnd: true,
};

describe('copy', () => {
  it('copies a dragged cell range as TSV', async () => {
    const { container } = render(ResultsGrid, { props });
    const cells = container.querySelectorAll('tbody td:not(.row-num)');
    await fireEvent.mouseDown(cells[0]);
    await fireEvent.mouseEnter(cells[3]);
    await fireEvent.mouseUp(window);
    await fireEvent.keyDown(container.querySelector('.table-wrapper')!, {
      key: 'c',
      metaKey: true,
    });
    expect(writeText).toHaveBeenCalledWith('1\ta@x.com\n2\tb@x.com');
  });

  it('copies a gutter row selection when no cell range is active', async () => {
    const { container } = render(ResultsGrid, { props });
    await fireEvent.click(container.querySelectorAll('.gutter')[1]);
    await fireEvent.keyDown(container.querySelector('.table-wrapper')!, {
      key: 'c',
      metaKey: true,
    });
    expect(writeText).toHaveBeenCalledWith('2\tb@x.com');
  });

  it('does not touch the clipboard with nothing selected', async () => {
    const { container } = render(ResultsGrid, { props });
    await fireEvent.keyDown(container.querySelector('.table-wrapper')!, {
      key: 'c',
      metaKey: true,
    });
    expect(writeText).not.toHaveBeenCalled();
  });
});
