import { describe, it, expect } from 'vitest';
import { toTsv, toCsv } from './serialize.js';

describe('toTsv', () => {
  it('joins cells with tabs and rows with newlines', () => {
    expect(
      toTsv([
        [1, 'a'],
        [2, 'b'],
      ]),
    ).toBe('1\ta\n2\tb');
  });

  it('renders numbers verbatim, matching what is on screen', () => {
    expect(toTsv([[4200000000000]])).toBe('4200000000000');
  });

  it('renders null as empty, matching the rendered cell', () => {
    expect(toTsv([[null, 'x']])).toBe('\tx');
  });

  it('returns an empty string for no rows', () => {
    expect(toTsv([])).toBe('');
  });
});

describe('toCsv', () => {
  it('leaves plain values unquoted', () => {
    expect(toCsv([[1, 'abc']])).toBe('1,abc');
  });

  it('quotes a value containing a comma', () => {
    expect(toCsv([['a,b']])).toBe('"a,b"');
  });

  it('quotes and doubles an interior quote', () => {
    expect(toCsv([['say "hi"']])).toBe('"say ""hi"""');
  });

  it('quotes a value containing a newline', () => {
    expect(toCsv([['line1\nline2']])).toBe('"line1\nline2"');
  });

  it('quotes a value containing a carriage return', () => {
    expect(toCsv([['a\rb']])).toBe('"a\rb"');
  });

  it('renders null as an empty unquoted field', () => {
    expect(toCsv([[null, 1]])).toBe(',1');
  });

  it('does not quote a value that merely contains a tab', () => {
    // Tabs are only special in TSV. Over-quoting CSV is its own bug.
    expect(toCsv([['a\tb']])).toBe('a\tb');
  });

  it('neutralizes spreadsheet formula triggers (CWE-1236)', () => {
    expect(toCsv([['=1+2']])).toBe("'=1+2");
    expect(toCsv([['+123']])).toBe("'+123");
    expect(toCsv([['-5']])).toBe("'-5");
    expect(toCsv([['@SUM(A1)']])).toBe("'@SUM(A1)");
    expect(toCsv([['\tcmd']])).toBe("'\tcmd");
  });

  it('preserves negative and positive numeric values without single-quote neutralization', () => {
    expect(toCsv([[-5]])).toBe('-5');
    expect(toCsv([[+123]])).toBe('123');
  });

  it('neutralizes formula triggers before RFC 4180 quoting', () => {
    expect(toCsv([['=1,2']])).toBe("\"'=1,2\"");
  });
});
