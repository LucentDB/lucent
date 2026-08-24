// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';
import GridGutter from './GridGutter.svelte';

// vitest globals are off in this repo, so @testing-library/svelte's automatic
// cleanup never registers — clean up explicitly between cases.
afterEach(cleanup);

describe('GridGutter', () => {
  it('renders the 1-based row number', () => {
    const { getByText } = render(GridGutter, {
      props: { rowNumber: 42, selected: false, onSelect: () => {} },
    });
    expect(getByText('42')).toBeTruthy();
  });

  it('marks itself selected for styling', () => {
    const { container } = render(GridGutter, {
      props: { rowNumber: 1, selected: true, onSelect: () => {} },
    });
    expect(container.querySelector('.selected')).toBeTruthy();
  });

  it('reports a plain click', async () => {
    const onSelect = vi.fn();
    const { getByRole } = render(GridGutter, {
      props: { rowNumber: 1, selected: false, onSelect },
    });
    await fireEvent.click(getByRole('button'));
    expect(onSelect).toHaveBeenCalledWith(
      expect.objectContaining({ shiftKey: false }),
    );
  });

  it('reports a shift-click so the parent can extend', async () => {
    const onSelect = vi.fn();
    const { getByRole } = render(GridGutter, {
      props: { rowNumber: 1, selected: false, onSelect },
    });
    await fireEvent.click(getByRole('button'), { shiftKey: true });
    expect(onSelect).toHaveBeenCalledWith(
      expect.objectContaining({ shiftKey: true }),
    );
  });

  it('is keyboard reachable and labelled', () => {
    const { getByRole } = render(GridGutter, {
      props: { rowNumber: 7, selected: false, onSelect: () => {} },
    });
    // A div would make row selection mouse-only.
    expect(getByRole('button', { name: /select row 7/i })).toBeTruthy();
  });
});
