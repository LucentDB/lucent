import { describe, it, expect } from 'vitest';
import { resultSummary } from './resultSummary.js';

describe('resultSummary', () => {
  it('reports rows affected for DML results', () => {
    expect(resultSummary({ rows_affected: 14, row_count: 0 })).toBe(
      '14 rows affected',
    );
  });

  it('reports 0 rows affected for DML that matched nothing', () => {
    expect(resultSummary({ rows_affected: 0, row_count: 0 })).toBe(
      '0 rows affected',
    );
  });

  it('falls back to the row count when rows_affected is null', () => {
    expect(resultSummary({ rows_affected: null, row_count: 5 })).toBe('5 rows');
  });

  it('falls back to the row count when rows_affected is missing', () => {
    expect(resultSummary({ row_count: 42 })).toBe('42 rows');
  });

  it('reports 0 rows for an empty SELECT result', () => {
    expect(resultSummary({ rows_affected: null, row_count: 0 })).toBe('0 rows');
  });

  it('treats a missing result as empty', () => {
    expect(resultSummary(undefined)).toBe('0 rows');
  });
});
