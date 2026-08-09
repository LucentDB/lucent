import { describe, it, expect } from 'vitest';
import { formatCell } from './ResultsGrid.svelte';

describe('formatCell', () => {
  it('renders integers without locale separators', () => {
    expect(formatCell(4200000000000)).toBe('4200000000000');
    expect(formatCell(42)).toBe('42');
    expect(formatCell(-1)).toBe('-1');
  });

  it('renders floats without locale separators or forced precision', () => {
    expect(formatCell(1.5)).toBe('1.5');
    expect(formatCell(0.1)).toBe('0.1');
  });

  it('renders booleans and strings unchanged', () => {
    expect(formatCell(true)).toBe('true');
    expect(formatCell('hello')).toBe('hello');
  });

  it('renders null and undefined as empty', () => {
    expect(formatCell(null)).toBe('');
    expect(formatCell(undefined)).toBe('');
  });
});
