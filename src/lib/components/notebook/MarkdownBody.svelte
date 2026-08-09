<script lang="ts">
  import { renderMarkdown } from '../chat/markdown.ts';

  let {
    source,
    onToggleTask,
  }: {
    source: string;
    onToggleTask?: (index: number, checked: boolean) => void;
  } = $props();

  let html = $derived(renderMarkdown(source));

  // marked emits task inputs in source order, so the index of the clicked box is
  // the index of its marker in the source text.
  function handleClick(e: MouseEvent) {
    const target = e.target as HTMLElement | null;
    if (!target || target.tagName !== 'INPUT') return;
    const input = target as HTMLInputElement;
    if (input.type !== 'checkbox') return;

    const boxes = [
      ...(e.currentTarget as HTMLElement).querySelectorAll<HTMLInputElement>(
        'input[type="checkbox"]',
      ),
    ];
    const index = boxes.indexOf(input);
    if (index < 0) return;

    if (!onToggleTask) {
      e.preventDefault(); // read-only: don't let the DOM diverge from source
      return;
    }
    onToggleTask(index, input.checked);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
<div class="markdown-body" onclick={handleClick}>
  {@html html}
</div>

<style>
  .markdown-body {
    font-size: var(--text-sm);
    line-height: 1.6;
    color: var(--text);
  }
  .markdown-body :global(h1),
  .markdown-body :global(h2),
  .markdown-body :global(h3),
  .markdown-body :global(h4),
  .markdown-body :global(h5),
  .markdown-body :global(h6) {
    margin: 0.6em 0 0.3em;
    line-height: 1.3;
    font-weight: var(--weight-semibold);
  }
  .markdown-body :global(h1) {
    font-size: 1.5em;
  }
  .markdown-body :global(h2) {
    font-size: 1.3em;
  }
  .markdown-body :global(h3) {
    font-size: 1.15em;
  }
  .markdown-body :global(p) {
    margin: 0.4em 0;
  }
  .markdown-body :global(:first-child) {
    margin-top: 0;
  }
  .markdown-body :global(:last-child) {
    margin-bottom: 0;
  }

  .markdown-body :global(ul),
  .markdown-body :global(ol) {
    margin: 0.4em 0;
    padding-left: 1.4em;
  }
  .markdown-body :global(li) {
    margin: 0.15em 0;
  }
  .markdown-body :global(li > ul),
  .markdown-body :global(li > ol) {
    margin: 0.15em 0;
  }

  /* Task lists: without list-style:none the bullet sits beside the checkbox,
     which is the "bullet plus blob" the old stylesheet produced. */
  .markdown-body :global(li:has(> input[type='checkbox'])) {
    list-style: none;
    margin-left: -1.2em;
    display: flex;
    align-items: baseline;
    gap: 0.5em;
  }
  .markdown-body :global(input[type='checkbox']) {
    width: 13px;
    height: 13px;
    margin: 0;
    flex-shrink: 0;
    accent-color: var(--accent);
    cursor: pointer;
    position: relative;
    top: 1px;
  }

  .markdown-body :global(table) {
    display: block;
    width: max-content;
    max-width: 100%;
    overflow-x: auto;
    border-collapse: collapse;
    margin: 0.6em 0;
    font-size: 0.95em;
  }
  .markdown-body :global(th),
  .markdown-body :global(td) {
    padding: 4px 10px;
    border: 1px solid var(--border);
    text-align: left;
  }
  .markdown-body :global(th) {
    background: var(--bg-subtle);
    font-weight: var(--weight-semibold);
  }
  .markdown-body :global(tbody tr:nth-child(even)) {
    background: var(--bg-subtle);
  }

  .markdown-body :global(blockquote) {
    margin: 0.6em 0;
    padding: 0.2em 0 0.2em 0.9em;
    border-left: 3px solid var(--border);
    color: var(--text-secondary);
  }
  .markdown-body :global(hr) {
    margin: 1em 0;
    border: none;
    border-top: 1px solid var(--border);
  }
  .markdown-body :global(del) {
    color: var(--text-muted);
  }
  .markdown-body :global(img) {
    max-width: 100%;
    height: auto;
    border-radius: var(--radius-sm);
  }
  .markdown-body :global(a) {
    color: var(--accent);
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .markdown-body :global(code) {
    background: var(--bg-subtle);
    padding: 1px 4px;
    border-radius: 3px;
    font-family: var(--font-mono);
    font-size: 0.9em;
  }
  .markdown-body :global(pre) {
    margin: 0.6em 0;
    padding: 8px 10px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow-x: auto;
  }
  .markdown-body :global(pre code) {
    background: none;
    padding: 0;
  }
</style>
