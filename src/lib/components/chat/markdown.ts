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

// Force all links rendered from markdown to open in a new tab/window securely,
// preventing reverse tabnabbing (window.opener hijacking) in webview environments.
DOMPurify.addHook('afterSanitizeAttributes', (node) => {
  // Only links that survived sanitization with an href are worth opening in a
  // new tab: DOMPurify drops `javascript:`/`data:` hrefs, and an anchor without
  // an href is inert, so decorating it is noise.
  if (node.tagName === 'A' && node.hasAttribute('href')) {
    node.setAttribute('target', '_blank');
    node.setAttribute('rel', 'noopener noreferrer');
  }
});

// Forbid HTML forms and non-checkbox input controls to prevent UI spoofing / phishing
// in rendered markdown within the webview environment.
DOMPurify.addHook('uponSanitizeElement', (node, data) => {
  if (data.tagName === 'input' && node instanceof Element) {
    const type = node.getAttribute('type');
    if (type !== 'checkbox') {
      node.parentNode?.removeChild(node);
    }
  }
});

const FORBIDDEN_MARKDOWN_TAGS = [
  'form',
  'script',
  'iframe',
  'object',
  'embed',
  'style',
  'dialog',
  'select',
  'textarea',
  'button',
];

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
    return DOMPurify.sanitize(html, {
      FORBID_TAGS: FORBIDDEN_MARKDOWN_TAGS,
      FORBID_ATTR: ['autofocus'],
      // Only safe link protocols, so `data:`/`javascript:` URLs are dropped.
      ALLOWED_URI_REGEXP:
        /^(?:(?:(?:f|ht)tps?|mailto|tel):|[^a-z]|[a-z+.-]+(?:[^a-z+.-:]|$))/i,
    });
  } catch {
    return String(text ?? '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
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
