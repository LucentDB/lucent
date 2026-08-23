// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';
import ResultsGrid from './ResultsGrid.svelte';

afterEach(cleanup);

const columns = [{ name: 'n', type_name: 'int4' }];

/** N rows of one column, values 0..N-1, so a test can identify data by value. */
function rowsOf(n: number): unknown[][] {
  return Array.from({ length: n }, (_, i) => [i]);
}

/** The first batch shifted past 1000, so page-one cells prove WHICH buffer rendered. */
function secondBatch(n: number): unknown[][] {
  return Array.from({ length: n }, (_, i) => [i + 1000]);
}

/**
 * Spec §8.2 streaming-staleness guard — the failure mode it covers is silent:
 * a value-passed reactive input (spec §3.2) never throws; the engine's effect
 * simply never re-fires and the grid keeps showing the first batch while
 * fetches accumulate. So this test mounts the real component, streams a second
 * batch through the same props path App uses, and asserts at the RENDERED
 * level — not on the row model — that both the fetched count moved and the
 * visible cells come from the new buffer.
 */
describe('streaming staleness (spec §8.2)', () => {
  it('renders the new buffer when a second streamed batch arrives', async () => {
    const pageSize = 50;
    const grid = render(ResultsGrid, {
      props: {
        columns,
        rows: rowsOf(50),
        fetchedCount: 50,
        isEnd: false,
        pageSize,
      },
    });

    // Page one of the first batch is on screen.
    const before = grid.container.querySelectorAll('tbody tr');
    expect(before).toHaveLength(50);
    expect(before[0].textContent).toContain('0');

    // A second streamed batch arrives: the accumulated buffer doubles and
    // every value changes. Same mount, new props — exactly how App feeds it.
    await grid.rerender({
      columns,
      rows: secondBatch(100),
      fetchedCount: 100,
      isEnd: false,
      pageSize,
    });

    // (a) Rendered row count matches the new page slice…
    const after = grid.container.querySelectorAll('tbody tr');
    expect(after).toHaveLength(50);
    // …and (b) the cells come from the NEW array: page-one row 0 shows the
    // second batch's marker value (1000), which the stale first batch could
    // never produce. This is the assertion that catches a value-passed
    // regression — the count alone would pass either way.
    expect(after[0].textContent).toContain('1000');
    expect(after[49].textContent).toContain('1049');

    // (c) The footer reflects that fetchedCount moved: two pages are now
    // fetched, so the pager reports "of 100 fetched".
    expect(grid.container.querySelector('.page-info')?.textContent).toContain(
      '100',
    );
  });
});
