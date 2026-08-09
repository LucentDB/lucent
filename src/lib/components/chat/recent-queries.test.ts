import { describe, it, expect } from 'vitest';
import {
  excerptSql,
  relativeTime,
  formatRowCount,
  formatDuration,
  splitExcerpt,
  dedupeBySql,
  describeEntry,
  explainPrompt,
} from './recent-queries.ts';
import type { HistoryEntry } from '../../stores/history.svelte.ts';

const NOW = new Date('2026-07-27T12:00:00Z').getTime();

function entry(over: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: 'h1',
    connectionId: 'c1',
    connectionName: 'prod',
    database: 'shop',
    sql: 'SELECT 1',
    durationMs: 5,
    rowCount: 1,
    status: 'success',
    error: null,
    executedAt: '2026-07-27T11:59:30Z',
    favorite: false,
    dateGroup: 'Today',
    ...over,
  };
}

describe('excerptSql', () => {
  it('collapses newlines and indentation into one line', () => {
    expect(excerptSql('SELECT a,\n       b\nFROM t')).toBe(
      'SELECT a, b FROM t',
    );
  });

  it('ellipsises long statements', () => {
    const out = excerptSql('SELECT ' + 'column_name, '.repeat(20) + 'FROM t');
    expect(out.length).toBeLessThanOrEqual(44);
    expect(out.endsWith('…')).toBe(true);
  });

  it('leaves short statements untouched', () => {
    expect(excerptSql('  SELECT 1  ')).toBe('SELECT 1');
  });
});

describe('relativeTime', () => {
  it.each([
    ['2026-07-27T11:59:30Z', 'just now'],
    ['2026-07-27T11:55:00Z', '5m ago'],
    ['2026-07-27T09:00:00Z', '3h ago'],
    ['2026-07-26T12:00:00Z', 'yesterday'],
    ['2026-07-24T12:00:00Z', '3d ago'],
    ['2026-07-20T12:00:00Z', '1w ago'],
    ['2026-07-06T12:00:00Z', '3w ago'],
  ])('renders %s as %s', (iso, expected) => {
    expect(relativeTime(iso, NOW)).toBe(expected);
  });

  it('returns an empty string for an unparseable timestamp', () => {
    expect(relativeTime('not a date', NOW)).toBe('');
  });

  it('clamps future timestamps instead of showing negative time', () => {
    expect(relativeTime('2026-07-27T12:05:00Z', NOW)).toBe('just now');
  });
});

describe('formatRowCount', () => {
  it.each([
    [0, '0'],
    [999, '999'],
    [1000, '1k'],
    [1234, '1.2k'],
    [12_345, '12k'],
    [1_200_000, '1.2M'],
  ])('formats %i as %s', (n, expected) => {
    expect(formatRowCount(n)).toBe(expected);
  });

  // This is what produced "NaNM rows" on screen when the backend sent
  // snake_case and `entry.rowCount` read as undefined.
  it.each([undefined, null, NaN, Infinity, '42'])(
    'returns null for the non-numeric value %s',
    (bad) => {
      expect(formatRowCount(bad)).toBeNull();
    },
  );
});

describe('formatDuration', () => {
  it.each([
    [0, '0ms'],
    [12, '12ms'],
    [999, '999ms'],
    [1500, '1.5s'],
    [12_000, '12s'],
  ])('formats %i as %s', (ms, expected) => {
    expect(formatDuration(ms)).toBe(expected);
  });

  it('returns null for a missing or negative duration', () => {
    expect(formatDuration(undefined)).toBeNull();
    expect(formatDuration(-5)).toBeNull();
    expect(formatDuration(NaN)).toBeNull();
  });
});

describe('splitExcerpt', () => {
  it('lifts the leading keyword and upper-cases it', () => {
    expect(splitExcerpt('select * from t')).toEqual({
      verb: 'SELECT',
      rest: '* from t',
    });
  });

  it('handles CTEs', () => {
    expect(splitExcerpt('WITH x AS (…) SELECT 1').verb).toBe('WITH');
  });

  it('returns a null verb for an unrecognised statement', () => {
    expect(splitExcerpt('\\d bookings')).toEqual({
      verb: null,
      rest: '\\d bookings',
    });
  });

  it('does not match a keyword that is only a prefix of a word', () => {
    expect(splitExcerpt('selection_report()').verb).toBeNull();
  });
});

describe('dedupeBySql', () => {
  it('collapses repeated runs of the same statement', () => {
    const out = dedupeBySql([
      entry({ id: 'a', sql: 'SELECT * FROM t' }),
      entry({ id: 'b', sql: 'SELECT  *  FROM   t' }),
      entry({ id: 'c', sql: 'select * from t' }),
      entry({ id: 'd', sql: 'SELECT * FROM other' }),
    ]);
    expect(out.map((e) => e.id)).toEqual(['a', 'd']);
  });

  it('keeps the newest occurrence, since history arrives newest-first', () => {
    const out = dedupeBySql([
      entry({ id: 'newest', sql: 'SELECT 1' }),
      entry({ id: 'older', sql: 'SELECT 1' }),
    ]);
    expect(out.map((e) => e.id)).toEqual(['newest']);
  });

  it('passes an empty list through', () => {
    expect(dedupeBySql([])).toEqual([]);
  });
});

describe('describeEntry', () => {
  it('reports rows, duration and age for a successful query', () => {
    expect(describeEntry(entry({ rowCount: 1234, durationMs: 8 }), NOW)).toBe(
      '1.2k rows · 8ms · just now',
    );
  });

  it('singularises a one-row result', () => {
    expect(describeEntry(entry({ rowCount: 1, durationMs: 5 }), NOW)).toBe(
      '1 row · 5ms · just now',
    );
  });

  it('surfaces failure rather than hiding it', () => {
    const out = describeEntry(
      entry({ status: 'error', rowCount: null, error: 'boom' }),
      NOW,
    );
    expect(out).toContain('failed');
    expect(out).not.toContain('row');
  });

  it('says "ran" when the row count is unknown', () => {
    expect(describeEntry(entry({ rowCount: null }), NOW)).toContain('ran');
  });

  it('drops the age when the timestamp is unusable', () => {
    expect(describeEntry(entry({ executedAt: 'garbage' }), NOW)).toBe(
      '1 row · 5ms',
    );
  });

  // The exact shape the snake_case wire bug produced on screen.
  it('never renders NaN when the backend omits the numeric fields', () => {
    const raw = {
      ...entry(),
      rowCount: undefined,
      durationMs: undefined,
      executedAt: undefined,
    } as unknown as HistoryEntry;
    const out = describeEntry(raw, NOW);
    expect(out).toBe('ran');
    expect(out).not.toContain('NaN');
  });
});

describe('explainPrompt', () => {
  it('asks for an explanation of a successful query', () => {
    const out = explainPrompt(entry({ sql: 'SELECT * FROM orders' }));
    expect(out).toContain('Explain what this query does');
    expect(out).toContain('```sql\nSELECT * FROM orders\n```');
  });

  it('asks for a fix — and includes the error — when the query failed', () => {
    const out = explainPrompt(
      entry({
        status: 'error',
        error: 'relation "ordrs" does not exist',
        sql: 'SELECT * FROM ordrs',
      }),
    );
    expect(out).toContain('failed');
    expect(out).toContain('corrected version');
    expect(out).toContain('relation "ordrs" does not exist');
  });

  it('does not append an error section when there is no error text', () => {
    expect(explainPrompt(entry())).not.toContain('The error was');
  });
});
