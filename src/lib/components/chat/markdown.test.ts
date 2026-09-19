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

test('strips unsafe URI schemes (javascript:, data:, vbscript:) in links', () => {
  const jsOut = renderMarkdown('[click](javascript:alert(1))');
  expect(jsOut.toLowerCase()).not.toContain('javascript:');

  const dataOut = renderMarkdown(
    '[click](data:text/html,<script>alert(1)</script>)',
  );
  expect(dataOut.toLowerCase()).not.toContain('data:');

  const vbOut = renderMarkdown('[click](vbscript:msgbox(1))');
  expect(vbOut.toLowerCase()).not.toContain('vbscript:');
});

test('adds target="_blank" and rel="noopener noreferrer" to links', () => {
  const out = renderMarkdown('[example](https://example.com)');
  expect(out).toContain('target="_blank"');
  expect(out).toContain('rel="noopener noreferrer"');
});

test('keeps benign markdown formatting', () => {
  const out = renderMarkdown('**bold** and `code`');
  expect(out).toContain('<strong>');
  expect(out).toContain('<code>');
});

test('syntax highlights SQL fenced code blocks', () => {
  const out = renderMarkdown(
    "```sql\nSELECT name FROM users WHERE id = 1 AND name = 'Ada';\n```",
  );

  expect(out).toContain('<code class="language-sql">');
  expect(out).toContain('<span class="tok-keyword">SELECT</span>');
  expect(out).toContain('<span class="tok-string">\'Ada\'</span>');
  expect(out).toContain('<span class="tok-number">1</span>');
});

test('escapes SQL fenced code before adding token markup', () => {
  const out = renderMarkdown(
    "```sql\nSELECT '<img src=x onerror=alert(1)>' AS payload;\n```",
  );

  expect(out).not.toContain('<img src=x');
  expect(out).toContain('&lt;img src=x onerror=alert(1)&gt;');
});
