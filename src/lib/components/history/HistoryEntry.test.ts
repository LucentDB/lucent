import { render, screen, cleanup } from '@testing-library/svelte';
import { describe, it, expect, beforeEach } from 'vitest';
import HistoryEntry from './HistoryEntry.svelte';
import type { HistoryEntry as HistoryEntryType } from '../../stores/history.svelte';

describe('HistoryEntry accessibility', () => {
  const sampleEntry: HistoryEntryType = {
    id: 'entry-1',
    sql: 'SELECT * FROM users;',
    durationMs: 42,
    rowCount: 10,
    status: 'success',
    executedAt: new Date().toISOString(),
    favorite: false,
    connectionName: 'main-db',
  };

  beforeEach(() => {
    cleanup();
  });

  it('renders a semantic button for rerunning the query with proper aria-label', () => {
    render(HistoryEntry, { entry: sampleEntry });

    const mainBtn = screen.getByRole('button', {
      name: /rerun query: select \* from users;/i,
    });
    expect(mainBtn).toBeDefined();
  });

  it('renders action buttons with explicit aria-labels', () => {
    render(HistoryEntry, { entry: sampleEntry });

    expect(
      screen.getByRole('button', { name: 'Favorite query' }),
    ).toBeDefined();
    expect(screen.getByRole('button', { name: 'Copy SQL' })).toBeDefined();
    expect(
      screen.getByRole('button', { name: 'Delete query from history' }),
    ).toBeDefined();
  });

  it('updates favorite button aria-label when entry is favorited', () => {
    render(HistoryEntry, { entry: { ...sampleEntry, favorite: true } });

    expect(
      screen.getByRole('button', { name: 'Unfavorite query' }),
    ).toBeDefined();
  });
});
