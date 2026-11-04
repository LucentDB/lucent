import { render, screen, cleanup } from '@testing-library/svelte';
import { describe, it, expect, beforeEach } from 'vitest';
import HistoryEntry from './HistoryEntry.svelte';
import type { HistoryEntry as HistoryEntryType } from '../../stores/history.svelte';

describe('HistoryEntry accessibility', () => {
  const sampleEntry: HistoryEntryType = {
    id: 'entry-1',
    connectionId: 'conn-1',
    sql: 'SELECT * FROM users;',
    durationMs: 42,
    rowCount: 10,
    status: 'success',
    error: null,
    executedAt: new Date().toISOString(),
    favorite: false,
    connectionName: 'main-db',
    database: 'main',
    dateGroup: 'Today',
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

  it('correctly truncates multi-line SQL without error and formats label', () => {
    const multiLineEntry: HistoryEntryType = {
      ...sampleEntry,
      sql: 'SELECT id, username\nFROM users\nWHERE active = true;',
    };
    render(HistoryEntry, { entry: multiLineEntry });

    expect(
      screen.getByRole('button', {
        name: /rerun query: select id, username/i,
      }),
    ).toBeDefined();
    expect(screen.getByText('SELECT id, username')).toBeDefined();
  });

  it('truncates long SQL lines exceeding 80 chars with an ellipsis', () => {
    const longSql =
      'SELECT id, username, email, first_name, last_name, created_at, updated_at, status FROM users WHERE is_active = true';
    const longEntry: HistoryEntryType = {
      ...sampleEntry,
      sql: `${longSql}\nLIMIT 10;`,
    };
    render(HistoryEntry, { entry: longEntry });

    const expectedTruncated = longSql.slice(0, 80) + '…';
    expect(screen.getByText(expectedTruncated)).toBeDefined();
  });
});
