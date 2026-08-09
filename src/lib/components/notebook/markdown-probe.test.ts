import { describe, it, expect } from 'vitest';
import { looksLikeMarkdown } from './markdown-probe.ts';

describe('looksLikeMarkdown', () => {
  it('treats plain prose as plain text', () => {
    expect(looksLikeMarkdown('Which airplanes fly the most')).toBe(false);
    expect(looksLikeMarkdown('show me revenue by region for Q4')).toBe(false);
    expect(looksLikeMarkdown('')).toBe(false);
    expect(looksLikeMarkdown('what is 2 * 3 * 4')).toBe(false);
  });

  it('detects headings', () => {
    expect(looksLikeMarkdown('# Title')).toBe(true);
    expect(looksLikeMarkdown('intro\n### Section')).toBe(true);
  });

  it('does not treat a hash without a space as a heading', () => {
    expect(looksLikeMarkdown('#hashtag only')).toBe(false);
  });

  it('detects lists', () => {
    expect(looksLikeMarkdown('- one\n- two')).toBe(true);
    expect(looksLikeMarkdown('1. first')).toBe(true);
  });

  it('detects fenced code, blockquotes, and tables', () => {
    expect(looksLikeMarkdown('```sql\nSELECT 1\n```')).toBe(true);
    expect(looksLikeMarkdown('> quoted')).toBe(true);
    expect(looksLikeMarkdown('| a | b |\n|---|---|')).toBe(true);
  });

  it('detects inline emphasis, code, and links', () => {
    expect(looksLikeMarkdown('this is **bold**')).toBe(true);
    expect(looksLikeMarkdown('use `SELECT` here')).toBe(true);
    expect(looksLikeMarkdown('see [docs](http://x)')).toBe(true);
  });

  it('detects task lists', () => {
    expect(looksLikeMarkdown('- [ ] todo')).toBe(true);
  });
});
