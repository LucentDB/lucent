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

describe('select-all-on-page', () => {
  it('keeps already-selected rows instead of toggling them off', async () => {
    const { container } = render(ResultsGrid, { props });
    // Partial selection: only row 0 is selected.
    await fireEvent.click(container.querySelectorAll('.gutter')[0]);
    // Select-all must UNION, not toggle each row (which would drop row 0).
    await fireEvent.click(
      container.querySelector('thead input[type="checkbox"]')!,
    );
    const states = [...container.querySelectorAll('.gutter')].map((g) =>
      g.className.includes('selected'),
    );
    expect(states).toEqual([true, true]);
  });

  it('clears when every page row is already selected', async () => {
    const { container } = render(ResultsGrid, { props });
    await fireEvent.click(
      container.querySelector('thead input[type="checkbox"]')!,
    );
    await fireEvent.click(
      container.querySelector('thead input[type="checkbox"]')!,
    );
    const states = [...container.querySelectorAll('.gutter')].map((g) =>
      g.className.includes('selected'),
    );
    expect(states).toEqual([false, false]);
  });
});

describe('focus follows selection (WebKit never focuses buttons on click)', () => {
  it('moves focus to the grid after a gutter click, so Cmd+C reaches the grid handler', async () => {
    const { container } = render(ResultsGrid, { props });
    await fireEvent.click(container.querySelectorAll('.gutter')[1]);
    // In WKWebView/Safari a mousedown does not focus <button>s — focus would
    // stay on <body> and the Cmd+C keydown would never pass through the grid.
    expect(document.activeElement).toBe(
      container.querySelector('.table-wrapper'),
    );
  });

  it('moves focus to the grid after starting a cell selection', async () => {
    const { container } = render(ResultsGrid, { props });
    const cells = container.querySelectorAll('tbody td:not(.row-num)');
    await fireEvent.mouseDown(cells[0]);
    expect(document.activeElement).toBe(
      container.querySelector('.table-wrapper'),
    );
  });
});
