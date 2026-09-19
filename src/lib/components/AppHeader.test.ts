import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';
import AppHeader from './AppHeader.svelte';

afterEach(cleanup);

describe('AppHeader ARIA labels', () => {
  it('provides accessible ARIA labels for header action buttons when connected', () => {
    const { getByRole, getAllByRole } = render(AppHeader, {
      connected: true,
      sidebarCollapsed: false,
      hasTabs: true,
      tabs: [{ id: 't1', name: 'Query 1', kind: 'query' }],
    });

    expect(getByRole('button', { name: 'Hide sidebar' })).toBeTruthy();
    expect(getByRole('button', { name: 'New Query' })).toBeTruthy();
    expect(getByRole('button', { name: 'AI Settings' })).toBeTruthy();
    expect(getByRole('button', { name: 'Worker logs' })).toBeTruthy();
    expect(
      getByRole('button', { name: 'Search command palette' }),
    ).toBeTruthy();
    expect(getByRole('button', { name: 'Toggle theme' })).toBeTruthy();
    expect(getByRole('button', { name: 'Toggle AI Chat' })).toBeTruthy();

    const closeTabBtns = getAllByRole('button', { name: 'Close tab' });
    expect(closeTabBtns.length).toBeGreaterThan(0);
  });

  it('updates sidebar toggle ARIA label when sidebar is collapsed', () => {
    const { getByRole } = render(AppHeader, {
      connected: true,
      sidebarCollapsed: true,
    });

    expect(getByRole('button', { name: 'Show sidebar' })).toBeTruthy();
  });
});
