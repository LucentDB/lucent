import { describe, it, expect, afterEach } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';
import ResultsGrid from './ResultsGrid.svelte';

afterEach(cleanup);

const COLUMNS = [{ name: 'id', type_name: 'int4' }];

function mount(props = {}) {
  return render(ResultsGrid, {
    props: {
      columns: COLUMNS,
      rows: [],
      fetchedCount: 0,
      isEnd: true,
      ...props,
    },
  });
}

describe('ResultsGrid DML summary', () => {
  it('shows the summary in the empty state instead of the generic texts', () => {
    const { getAllByText, queryByText } = mount({
      summary: '14 rows affected',
    });
    expect(getAllByText('14 rows affected').length).toBeGreaterThan(0);
    expect(queryByText('No rows found')).toBeNull();
    expect(queryByText('The query returned no results')).toBeNull();
  });

  it('shows the summary in the toolbar instead of "No results"', () => {
    const { container, queryByText } = mount({ summary: '0 rows affected' });
    expect(container.querySelector('.no-results')?.textContent).toBe(
      '0 rows affected',
    );
    expect(queryByText('No results')).toBeNull();
  });

  it('keeps the generic empty state without a summary', () => {
    const { getByText } = mount();
    expect(getByText('No rows found')).toBeTruthy();
  });
});
