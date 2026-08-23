import { describe, it, expect } from 'vitest';
import { formatCell, cellClass } from './format.js';

describe('formatCell', () => {
  it('renders integers verbatim, with no locale separators', () => {
    // This is a database client: 4,200,000,000,000 cannot be pasted into a query.
    expect(formatCell(4200000000000)).toBe('4200000000000');
    expect(formatCell(42)).toBe('42');
    expect(formatCell(-1)).toBe('-1');
  });

  it('renders floats at their shortest round-trippable form', () => {
    expect(formatCell(1.5)).toBe('1.5');
  });

  it('renders null and undefined as empty, not as the word null', () => {
    expect(formatCell(null)).toBe('');
    expect(formatCell(undefined)).toBe('');
  });

  it('renders booleans as their literal text', () => {
    expect(formatCell(true)).toBe('true');
    expect(formatCell(false)).toBe('false');
  });

  it('stringifies anything else', () => {
    expect(formatCell('abc')).toBe('abc');
  });
});

describe('cellClass', () => {
  it('marks nullish cells so CSS can grey them', () => {
    expect(cellClass(null)).toBe('cell-null');
    expect(cellClass(undefined)).toBe('cell-null');
  });

  it('marks booleans and numbers for their own styling', () => {
    expect(cellClass(true)).toBe('cell-bool');
    expect(cellClass(0)).toBe('cell-number');
  });

  it('leaves strings unclassed', () => {
    expect(cellClass('abc')).toBe('');
  });

  it('classes zero as a number, not as nullish', () => {
    // A falsy-check regression here would render 0 as a null cell.
    expect(cellClass(0)).toBe('cell-number');
  });
});
