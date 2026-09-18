import { test, expect } from 'vitest';
import { highlightSqlHtml, tokenizeSql } from './sql-highlight.ts';

test('tokenizeSql returns tokens for empty and non-empty code', () => {
  expect(tokenizeSql('')).toEqual([]);
  const tokens = tokenizeSql('SELECT 1;');
  expect(tokens.length).toBeGreaterThan(0);
});

test('highlightSqlHtml escapes double and single quotes along with HTML special characters', () => {
  const sqlWithQuotesAndMarkup = `SELECT '<img src="x" onerror="alert(1)">' AS "col'name";`;
  const html = highlightSqlHtml(sqlWithQuotesAndMarkup);

  expect(html).not.toContain('<img');
  expect(html).not.toContain('onerror="');
  expect(html).toContain('&lt;img');
  expect(html).toContain('&quot;');
  expect(html).toContain('&#39;');
});
