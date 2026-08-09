import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, fireEvent, cleanup } from '@testing-library/svelte';

// ChatLanding pulls in the history and schema-summary stores, both of which
// reach for Tauri IPC on construction/use. Stub the boundary so these stay
// pure render tests.
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => []),
  Channel: class {},
}));

import ChatLanding from './ChatLanding.svelte';
import { history } from '../../stores/history.svelte.ts';
import { schemaSummary } from '../../stores/schema-summary.svelte.ts';
import type { HistoryEntry } from '../../stores/history.svelte.ts';

function historyEntry(over: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: 'h1',
    connectionId: 'c1',
    connectionName: 'prod',
    database: 'shop',
    sql: 'SELECT * FROM invoices',
    durationMs: 12,
    rowCount: 42,
    status: 'success',
    error: null,
    executedAt: new Date().toISOString(),
    favorite: false,
    dateGroup: 'Today',
    ...over,
  };
}

function setup(props: Record<string, unknown> = {}) {
  const onSend = vi.fn();
  const result = render(ChatLanding, { onSend, ...props });
  return { ...result, onSend };
}

/**
 * Collapsed text content. The SQL excerpt renders as separate spans (the verb
 * is weighted differently), so the markup's indentation lands between words.
 */
function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

beforeEach(() => {
  history.entries = [];
  history.loading = false;
  history.error = null;
  schemaSummary.reset();
});

afterEach(cleanup);

describe('ChatLanding — disconnected', () => {
  it('disables the composer instead of inviting requests that must fail', () => {
    const { getByPlaceholderText } = setup({ connected: false });
    const input = getByPlaceholderText(
      'Connect a database to start asking…',
    ) as HTMLTextAreaElement;
    expect(input.disabled).toBe(true);
  });

  it('says it is not connected', () => {
    const { getByText } = setup({ connected: false });
    expect(getByText('No database connected')).toBeTruthy();
  });

  it('lists capabilities rather than schema suggestions', () => {
    const { getByText, queryByText } = setup({ connected: false });
    expect(getByText('What it can do')).toBeTruthy();
    expect(queryByText('Try asking')).toBeNull();
  });

  it('hides recent queries, which belong to a connection', () => {
    history.entries = [historyEntry()];
    const { queryByText } = setup({ connected: false });
    expect(queryByText('Recent queries')).toBeNull();
  });

  it('offers no destructive suggestion', () => {
    const { container } = setup({ connected: false });
    expect(container.textContent).not.toMatch(/\bdelete\b/i);
  });
});

describe('ChatLanding — connected', () => {
  it('names the database in the hero', () => {
    const { container } = setup({ connected: true, database: 'shop_db' });
    expect(container.querySelector('.hero')?.textContent).toContain('shop_db');
  });

  it('shows connection, database and model in the context strip', () => {
    const { container } = setup({
      connected: true,
      database: 'shop_db',
      connectionName: 'prod',
    });
    const strip = container.querySelector('.context')?.textContent ?? '';
    expect(strip).toContain('prod');
    expect(strip).toContain('shop_db');
    expect(strip).toContain('gpt-4o');
  });

  it('does not repeat a name shared by the connection and the database', () => {
    const { container } = setup({
      connected: true,
      database: 'shop',
      connectionName: 'shop',
    });
    const parts = Array.from(container.querySelectorAll('.context .part'));
    expect(parts.filter((p) => p.textContent === 'shop')).toHaveLength(1);
  });

  it('enables the composer', () => {
    const { getByPlaceholderText } = setup({
      connected: true,
      database: 'shop_db',
    });
    const input = getByPlaceholderText(
      'Ask anything about shop_db…',
    ) as HTMLTextAreaElement;
    expect(input.disabled).toBe(false);
  });

  it('opens AI settings from the context strip', async () => {
    const onOpenSettings = vi.fn();
    const { getByLabelText } = setup({ connected: true, onOpenSettings });
    await fireEvent.click(getByLabelText('AI settings'));
    expect(onOpenSettings).toHaveBeenCalled();
  });

  it('sends the full prompt on chip click, not the truncated label', async () => {
    schemaSummary.tables = [
      { name: 'a_very_long_table_name_that_will_be_truncated', rowCount: 900 },
    ];
    schemaSummary.loaded = true;

    const { getByText, onSend } = setup({ connected: true });
    const chip = getByText(/Summarize the/);
    await fireEvent.click(chip.closest('button')!);

    const sent = onSend.mock.calls[0][0] as string;
    expect(sent).toContain('a_very_long_table_name_that_will_be_truncated');
    expect(sent).not.toContain('…');
  });

  it('grounds suggestions in the loaded schema', () => {
    schemaSummary.tables = [
      { name: 'invoices', rowCount: 90_000 },
      { name: 'customers', rowCount: 400 },
    ];
    schemaSummary.loaded = true;

    const { container } = setup({ connected: true });
    expect(container.textContent).toContain('invoices');
  });
});

describe('ChatLanding — recent queries', () => {
  it('lists recent queries with their outcome', () => {
    history.entries = [historyEntry({ sql: 'SELECT * FROM invoices' })];
    const { getByText, container } = setup({
      connected: true,
      database: 'shop',
    });
    expect(getByText('Recent queries')).toBeTruthy();
    expect(text(container)).toContain('SELECT * FROM invoices');
    expect(text(container)).toContain('42 rows');
  });

  it('renders no NaN when the backend omits the numeric fields', () => {
    history.entries = [
      {
        ...historyEntry(),
        rowCount: undefined,
        durationMs: undefined,
        executedAt: undefined,
      } as unknown as HistoryEntry,
    ];
    const { container } = setup({ connected: true });
    expect(text(container)).not.toContain('NaN');
  });

  it('asks the copilot to explain the query rather than re-running it', async () => {
    history.entries = [historyEntry({ sql: 'SELECT 1' })];
    const { container, onSend } = setup({ connected: true });

    const button = Array.from(container.querySelectorAll('button')).find((b) =>
      text(b).includes('SELECT 1'),
    );
    await fireEvent.click(button!);

    const sent = onSend.mock.calls[0][0] as string;
    expect(sent).toContain('Explain what this query does');
    expect(sent).toContain('```sql');
  });

  it('asks for a fix when the recent query failed', async () => {
    history.entries = [
      historyEntry({
        sql: 'SELECT * FROM ordrs',
        status: 'error',
        rowCount: null,
        error: 'relation "ordrs" does not exist',
      }),
    ];
    const { container, onSend } = setup({ connected: true });

    const button = Array.from(container.querySelectorAll('button')).find((b) =>
      text(b).includes('ordrs'),
    );
    await fireEvent.click(button!);

    expect(onSend.mock.calls[0][0]).toContain('corrected version');
  });

  it('caps the list at three entries', () => {
    history.entries = Array.from({ length: 10 }, (_, i) =>
      historyEntry({ id: `h${i}`, sql: `SELECT ${i}` }),
    );
    const { container } = setup({ connected: true });
    expect(container.querySelectorAll('.recents .item')).toHaveLength(3);
  });

  // The screenshot showed the same browse query three times over.
  it('collapses repeated runs of the same statement', () => {
    history.entries = Array.from({ length: 5 }, (_, i) =>
      historyEntry({
        id: `h${i}`,
        sql: 'SELECT * FROM bookings.airports_data',
      }),
    );
    const { container } = setup({ connected: true });
    expect(container.querySelectorAll('.recents .item')).toHaveLength(1);
  });

  it('hides the section entirely when history failed to load', () => {
    history.entries = [historyEntry()];
    history.error = 'boom';
    const { queryByText } = setup({ connected: true });
    expect(queryByText('Recent queries')).toBeNull();
  });
});
