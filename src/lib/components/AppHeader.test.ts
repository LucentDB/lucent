// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/svelte';

import AppHeader from './AppHeader.svelte';

afterEach(cleanup);

const tab = { id: 't1', name: 'Query 1', kind: 'query' };

describe('AppHeader accessibility', () => {
  it('gives every icon-only action button an accessible name', () => {
    render(AppHeader, {
      connected: true,
      sidebarCollapsed: false,
      hasTabs: true,
      tabs: [tab],
    });

    for (const name of [
      'Hide sidebar',
      'New Query',
      'AI Settings',
      'Worker logs',
      'Search command palette',
      'Toggle theme',
      'Toggle AI Chat',
    ]) {
      expect(screen.getByRole('button', { name })).toBeTruthy();
    }

    expect(screen.getAllByRole('button', { name: 'Close tab' }).length).toBe(1);
  });

  it('track the sidebar and chat state in their labels', () => {
    render(AppHeader, {
      connected: true,
      sidebarCollapsed: true,
      hasTabs: false,
      tabs: [],
    });

    expect(screen.getByRole('button', { name: 'Show sidebar' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'AI Chat' })).toBeTruthy();
  });
});
