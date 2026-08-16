import { marked, type Tokens } from 'marked';
import DOMPurify from 'dompurify';
import { highlightSqlHtml } from '../../utils/sql-highlight.ts';

// LLM output is rendered with {@html}, and it is NOT trusted: it can echo
// attacker-influenced data (e.g. a table cell containing markup). In a
// privileged Tauri webview an injected <script>/onerror could reach invoke(),
// so every rendered string is sanitized before it becomes HTML.
marked.setOptions({
  breaks: true,
  gfm: true,
});

const SQL_LANGUAGES = new Set([
  'sql',
  'postgres',
  'postgresql',
  'pgsql',
  'duckdb',
]);
const markdownRenderer = new marked.Renderer();
const defaultCodeRenderer = markdownRenderer.code.bind(markdownRenderer);

markdownRenderer.code = (token: Tokens.Code) => {
  const { text, lang } = token;
  const normalizedLanguage = (lang ?? '').trim().toLowerCase();
  if (!SQL_LANGUAGES.has(normalizedLanguage)) {
    return defaultCodeRenderer(token);
  }

  return `<pre><code class="language-${normalizedLanguage}">${highlightSqlHtml(text)}</code></pre>\n`;
};

export function renderMarkdown(text: string): string {
  try {
    const result = marked.parse(String(text ?? ''), {
      async: false,
      renderer: markdownRenderer,
    });
    const html = typeof result === 'string' ? result : String(result);
    return DOMPurify.sanitize(html);
  } catch {
    return String(text ?? '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
  }
}

/**
 * Matches a GFM task marker only at the start of a list item, so a literal "[x]"
 * in prose is never mistaken for a checkbox. Kept in source order, which is the
 * order `marked` emits the corresponding inputs.
 */
const TASK_MARKER_RE = /^([ \t]*(?:[-*+]|\d+[.)])[ \t]+)\[([ xX])\]/gm;

/** Flips the Nth task marker in `source`. Returns source unchanged if N is out of range. */
export function toggleTaskAtIndex(
  source: string,
  index: number,
  checked: boolean,
): string {
  let seen = 0;
  return source.replace(TASK_MARKER_RE, (match, prefix) => {
    const isTarget = seen === index;
    seen += 1;
    if (!isTarget) return match;
    return `${prefix}[${checked ? 'x' : ' '}]`;
  });
}
