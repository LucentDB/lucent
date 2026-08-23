// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import GridPagination from './GridPagination.svelte';

// vitest globals are off in this repo, so testing-library's auto-cleanup
// never registers — clean up explicitly between renders.
afterEach(cleanup);

const base = {
  page: 0,
  firstRowNumber: 1,
  lastRowNumber: 200,
  fetchedCount: 450,
  totalCount: null,
  canGoNext: true,
  isFetchingMore: false,
  onNext: () => {},
  onPrev: () => {},
};

describe('GridPagination', () => {
  it('shows the current row range', () => {
    const { getByText } = render(GridPagination, { props: base });
    expect(getByText(/1.*200/)).toBeTruthy();
  });

  it('disables Previous on the first page', () => {
    const { getByRole } = render(GridPagination, { props: base });
    expect((getByRole('button', { name: /previous/i }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('enables Previous past the first page', () => {
    const { getByRole } = render(GridPagination, { props: { ...base, page: 1 } });
    expect((getByRole('button', { name: /previous/i }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('disables Next when there is nothing to advance to', () => {
    const { getByRole } = render(GridPagination, { props: { ...base, canGoNext: false } });
    expect((getByRole('button', { name: /next/i }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('disables Next while a fetch is in flight', () => {
    const { getByRole } = render(GridPagination, { props: { ...base, isFetchingMore: true } });
    expect((getByRole('button', { name: /next/i }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('calls onNext and onPrev', async () => {
    const onNext = vi.fn();
    const onPrev = vi.fn();
    const { getByRole } = render(GridPagination, {
      props: { ...base, page: 1, onNext, onPrev },
    });
    await fireEvent.click(getByRole('button', { name: /next/i }));
    await fireEvent.click(getByRole('button', { name: /previous/i }));
    expect(onNext).toHaveBeenCalledTimes(1);
    expect(onPrev).toHaveBeenCalledTimes(1);
  });

  it('shows the total when one is known', () => {
    const { getByText } = render(GridPagination, {
      props: { ...base, totalCount: 9000 },
    });
    expect(getByText(/9,000/)).toBeTruthy();
  });
});
