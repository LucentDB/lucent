import { describe, it, expect, afterEach } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';
import ResultsGrid from './ResultsGrid.svelte';

afterEach(cleanup);

const columns = [{ name: 'n', type_name: 'int4' }];
const rows = Array.from({ length: 12 }, (_, i) => [i + 1]);

function mount(props = {}) {
  return render(ResultsGrid, {
    props: {
      columns,
      rows,
      fetchedCount: rows.length,
      totalCount: rows.length,
      isEnd: true,
      ...props,
    },
  });
}

describe('ResultsGrid pageSize', () => {
  it('defaults to 200 rows per page, so the query editor is unchanged', () => {
    const { container } = mount();
    expect(container.querySelectorAll('tbody tr').length).toBe(12);
    // All rows fit one page, so the pager is hidden.
    expect(container.querySelector('.pagination')).toBeNull();
  });

  it('renders only pageSize rows when pageSize is 5', () => {
    const { container } = mount({ pageSize: 5 });
    expect(container.querySelectorAll('tbody tr').length).toBe(5);
    expect(container.querySelector('.pagination')).toBeTruthy();
  });

  it('disables Prev on the first page and enables Next', () => {
    const { container } = mount({ pageSize: 5 });
    const buttons = [
      ...container.querySelectorAll('.page-btn'),
    ] as HTMLButtonElement[];
    const prev = buttons.find((b) => b.textContent?.includes('Prev'))!;
    const next = buttons.find((b) => b.textContent?.includes('Next'))!;
    expect(prev.disabled).toBe(true);
    expect(next.disabled).toBe(false);
  });

  it('embedded mode adds the embedded class', () => {
    const { container } = mount({ embedded: true, pageSize: 10 });
    expect(container.querySelector('.results-grid.embedded')).toBeTruthy();
  });

  it('non-embedded mode does not add the embedded class', () => {
    const { container } = mount();
    expect(container.querySelector('.results-grid.embedded')).toBeNull();
  });
});
