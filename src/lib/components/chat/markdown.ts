import { marked } from 'marked';
import DOMPurify from 'dompurify';

// LLM output is rendered with {@html}, and it is NOT trusted: it can echo
// attacker-influenced data (e.g. a table cell containing markup). In a
// privileged Tauri webview an injected <script>/onerror could reach invoke(),
// so every rendered string is sanitized before it becomes HTML.
marked.setOptions({
  breaks: true,
  gfm: true,
});

export function renderMarkdown(text: string): string {
  try {
    const result = marked.parse(String(text ?? ''), { async: false });
    const html = typeof result === 'string' ? result : String(result);
    return DOMPurify.sanitize(html);
  } catch {
    return String(text ?? '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
  }
}
