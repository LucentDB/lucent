import { describe, it, expect } from 'vitest';
import { buildSuggestions, CAPABILITIES } from './suggestions.ts';
import type { SchemaSummary } from './suggestions.ts';

function summary(tables: SchemaSummary['tables']): SchemaSummary {
  return { schema: 'public', tables };
}

describe('buildSuggestions', () => {
  it('grounds the first suggestion in the largest table', () => {
    const out = buildSuggestions(
      summary([
        { name: 'small_log', rowCount: 12 },
        { name: 'invoices', rowCount: 90_000 },
        { name: 'customers', rowCount: 4_000 },
      ]),
    );
    expect(out[0].prompt).toContain('invoices');
    expect(out[0].prompt).not.toContain('small_log');
  });

  it('offers a relationship prompt naming the two largest tables', () => {
    const out = buildSuggestions(
      summary([
        { name: 'customers', rowCount: 4_000 },
        { name: 'invoices', rowCount: 90_000 },
        { name: 'small_log', rowCount: 12 },
      ]),
    );
    const relate = out.find((s) => s.prompt.includes('relate'));
    expect(relate).toBeTruthy();
    expect(relate!.prompt).toContain('invoices');
    expect(relate!.prompt).toContain('customers');
    expect(relate!.prompt).not.toContain('small_log');
  });

  it('skips the relationship prompt when there is only one table', () => {
    const out = buildSuggestions(summary([{ name: 'events', rowCount: 5 }]));
    expect(out.some((s) => s.prompt.includes('relate'))).toBe(false);
    expect(out[0].prompt).toContain('events');
  });

  it('sorts tables with unknown row counts last without dropping them', () => {
    const out = buildSuggestions(
      summary([
        { name: 'never_analyzed', rowCount: null },
        { name: 'orders', rowCount: 7 },
      ]),
    );
    // `orders` (7 rows) must outrank the un-analyzed table...
    expect(out[0].prompt).toContain('orders');
    // ...but the un-analyzed one is still a real table worth relating to.
    expect(out[1].prompt).toContain('never_analyzed');
  });

  it('falls back to generic prompts when the schema is unknown', () => {
    for (const input of [null, summary([])]) {
      const out = buildSuggestions(input);
      expect(out.length).toBeGreaterThan(0);
      expect(out.every((s) => s.prompt.length > 0)).toBe(true);
    }
    expect(buildSuggestions(null)).toEqual(buildSuggestions(summary([])));
  });

  it('never returns an empty list', () => {
    expect(buildSuggestions(null).length).toBeGreaterThan(0);
  });

  it('caps the list at four chips', () => {
    const many = Array.from({ length: 40 }, (_, i) => ({
      name: `table_${i}`,
      rowCount: i,
    }));
    expect(buildSuggestions(summary(many))).toHaveLength(4);
  });

  it('truncates long labels but keeps the full table name in the prompt', () => {
    const long = 'an_extremely_long_and_unwieldy_table_name_from_legacy_etl';
    const out = buildSuggestions(summary([{ name: long, rowCount: 1 }]));
    expect(out[0].label.length).toBeLessThanOrEqual(42);
    expect(out[0].label).toContain('…');
    expect(out[0].prompt).toContain(long);
  });

  it('never suggests a destructive statement', () => {
    const out = buildSuggestions(
      summary([
        { name: 'orders', rowCount: 100 },
        { name: 'users', rowCount: 50 },
      ]),
    );
    const destructive = /\b(delete|drop|truncate|remove|purge|alter)\b/i;
    for (const s of out) {
      expect(s.prompt).not.toMatch(destructive);
      expect(s.label).not.toMatch(destructive);
    }
  });

  it('gives every suggestion an icon', () => {
    const out = buildSuggestions(summary([{ name: 'a', rowCount: 1 }]));
    expect(out.every((s) => s.icon.length > 0)).toBe(true);
  });
});

describe('CAPABILITIES', () => {
  it('names no specific tables, since there is no schema to name', () => {
    for (const c of CAPABILITIES) {
      expect(c.text).not.toMatch(/\b(orders|users|customers|invoices)\b/i);
      expect(c.icon.length).toBeGreaterThan(0);
    }
  });
});
