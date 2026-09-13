// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/svelte';
import StatusBar from './StatusBar.svelte';
import IndexingStatusWidget from './IndexingStatusWidget.svelte';
import IndexingDetailsPopover from './IndexingDetailsPopover.svelte';
import { indexing, __resetForTests } from '../../stores/indexing.svelte';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === 'get_schema_indexing_status') {
      return Promise.resolve({
        hasGraph: true,
        tier: 'fully_enriched',
        tableCount: 15,
        columnCount: 64,
        viewCount: 3,
        builtAtUnix: 1700000000,
      });
    }
    return Promise.resolve();
  }),
}));

beforeEach(() => {
  __resetForTests();
});

afterEach(cleanup);

describe('StatusBar Component', () => {
  it('renders disconnected state when connected is false', () => {
    render(StatusBar, { connected: false });
    expect(screen.getByText('Not connected')).toBeTruthy();
  });

  it('renders connected connection name and database', () => {
    render(StatusBar, {
      connected: true,
      connectionName: 'Production Postgres',
      databaseName: 'analytics_db',
      driver: 'postgres',
      readOnly: true,
    });
    expect(screen.getByText('Production Postgres')).toBeTruthy();
    expect(screen.getByText('analytics_db')).toBeTruthy();
    expect(screen.getByText('postgres')).toBeTruthy();
    expect(screen.getByText('RO')).toBeTruthy();
  });
});

describe('IndexingStatusWidget Component', () => {
  it('renders idle indexed badge when not active', () => {
    indexing.isComplete = true;
    render(IndexingStatusWidget, {
      connectionName: 'Prod',
      databaseName: 'main',
    });
    expect(screen.getByText('Indexed')).toBeTruthy();
  });

  it('renders active progress and stage when indexing', () => {
    indexing.isComplete = false;
    indexing.stage = 'sampling';
    indexing.percent = 45;
    indexing.text = 'Sampling 9/20 tables';

    render(IndexingStatusWidget, {
      connectionName: 'Prod',
      databaseName: 'main',
    });
    expect(screen.getByText('Indexing 45%')).toBeTruthy();
  });

  it('toggles details popover on widget click', async () => {
    indexing.isComplete = true;
    render(IndexingStatusWidget, {
      connectionName: 'Prod',
      databaseName: 'main',
    });

    const button = screen.getByRole('button', { name: /schema indexing status/i });
    expect(indexing.detailsOpen).toBe(false);

    await fireEvent.click(button);
    expect(indexing.detailsOpen).toBe(true);
  });
});

describe('IndexingDetailsPopover Component', () => {
  it('renders delta counts and action buttons', async () => {
    indexing.delta = {
      added: 3,
      modified: 2,
      deleted: 1,
      unchanged: 10,
    };
    indexing.stats = {
      cacheHits: 40,
      embeddingsComputed: 5,
      elapsedMs: 120,
    };

    render(IndexingDetailsPopover, {
      connectionName: 'Demo DB',
      databaseName: 'demo_data',
    });

    expect(screen.getByText('Schema Indexing')).toBeTruthy();
    expect(screen.getByText('+3')).toBeTruthy();
    expect(screen.getByText('~2')).toBeTruthy();
    expect(screen.getByText('-1')).toBeTruthy();
    expect(screen.getByText('=10')).toBeTruthy();
    expect(screen.getByText('Sync Delta')).toBeTruthy();
    expect(screen.getByText('Rebuild Index')).toBeTruthy();
  });
});
