// @vitest-environment jsdom
import { test, expect } from 'vitest';
import { renderMarkdown } from './markdown';

test('strips script and event-handler payloads', () => {
  const out = renderMarkdown(
    '<img src=x onerror=alert(1)>\n\n<script>alert(2)</script>',
  );
  expect(out.toLowerCase()).not.toContain('onerror');
  expect(out.toLowerCase()).not.toContain('<script');
});

test('strips javascript: URLs in links', () => {
  const out = renderMarkdown('[click](javascript:alert(1))');
  expect(out.toLowerCase()).not.toContain('javascript:');
});

test('keeps benign markdown formatting', () => {
  const out = renderMarkdown('**bold** and `code`');
  expect(out).toContain('<strong>');
  expect(out).toContain('<code>');
});
