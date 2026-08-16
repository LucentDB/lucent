<script lang="ts">
  import MarkdownBody from '../MarkdownBody.svelte';
  import { looksLikeMarkdown } from '../markdown-probe.ts';

  let {
    source,
    placeholder = 'Empty cell — click to edit',
    renderMode = 'markdown' as 'markdown' | 'auto',
    showToolbar = false,
    editing = false,
    onSourceChange,
    onRun,
    onRunAndAdvance,
    onEnterEdit,
    onExitEdit,
    onToggleTask,
  }: {
    source: string;
    placeholder?: string;
    renderMode?: 'markdown' | 'auto';
    showToolbar?: boolean;
    editing?: boolean;
    onSourceChange?: (v: string) => void;
    onRun?: () => void;
    onRunAndAdvance?: () => void;
    onEnterEdit?: () => void;
    onExitEdit?: () => void;
    onToggleTask?: (index: number, checked: boolean) => void;
  } = $props();

  let textareaEl: HTMLTextAreaElement | undefined = $state();

  // Entering edit mode is a focus transition, not just a rendering transition.
  // This keeps keyboard entry consistent when command mode activates a cell.
  $effect(() => {
    if (!editing || !textareaEl) return;
    if (document.activeElement !== textareaEl) {
      textareaEl.focus();
      const end = textareaEl.value.length;
      textareaEl.setSelectionRange(end, end);
    }
  });

  // 'auto' renders markdown only when the text actually looks like markdown, so a
  // plain AI prompt displays as the user typed it.
  let asMarkdown = $derived(
    renderMode === 'markdown' || looksLikeMarkdown(source),
  );

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      onExitEdit?.();
      onRun?.();
      return;
    }
    if (e.key === 'Enter' && e.shiftKey) {
      e.preventDefault();
      onExitEdit?.();
      onRunAndAdvance?.();
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      onExitEdit?.();
    }
  }

  function insertFormatting(before: string, after: string) {
    const ta = textareaEl;
    if (!ta) return;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const selected = source.slice(start, end);
    onSourceChange?.(
      source.slice(0, start) + before + selected + after + source.slice(end),
    );
    requestAnimationFrame(() => {
      ta.focus();
      ta.selectionStart = start + before.length;
      ta.selectionEnd = start + before.length + selected.length;
    });
  }
</script>

<div class="text-cell">
  {#if editing}
    {#if showToolbar}
      <div class="toolbar" role="toolbar" aria-label="Formatting">
        <button
          class="toolbar-btn"
          onclick={() => insertFormatting('**', '**')}
          title="Bold"
          aria-label="Bold"><strong>B</strong></button
        >
        <button
          class="toolbar-btn"
          onclick={() => insertFormatting('*', '*')}
          title="Italic"
          aria-label="Italic"><em>I</em></button
        >
        <button
          class="toolbar-btn"
          onclick={() => insertFormatting('### ', '')}
          title="Heading"
          aria-label="Heading"><code>H</code></button
        >
        <button
          class="toolbar-btn"
          onclick={() => insertFormatting('- ', '')}
          title="List"
          aria-label="List">&#8801;</button
        >
        <button
          class="toolbar-btn"
          onclick={() => insertFormatting('- [ ] ', '')}
          title="Task"
          aria-label="Task">&#9744;</button
        >
        <button
          class="toolbar-btn"
          onclick={() => insertFormatting('`', '`')}
          title="Code"
          aria-label="Code">&lt;/&gt;</button
        >
        <button
          class="toolbar-btn"
          onclick={() => insertFormatting('[', '](url)')}
          title="Link"
          aria-label="Link">&#128279;</button
        >
      </div>
    {/if}
    <textarea
      class="editor"
      bind:this={textareaEl}
      value={source}
      oninput={(e) => onSourceChange?.((e.target as HTMLTextAreaElement).value)}
      onkeydown={handleKeydown}
      onblur={() => onExitEdit?.()}
      {placeholder}
      spellcheck="false"></textarea>
  {:else}
    <button class="display" type="button" onclick={() => onEnterEdit?.()}>
      {#if source}
        {#if asMarkdown}
          <MarkdownBody {source} {onToggleTask} />
        {:else}
          <span class="plain">{source}</span>
        {/if}
      {:else}
        <span class="placeholder">{placeholder}</span>
      {/if}
    </button>
  {/if}
</div>

<style>
  .toolbar {
    display: flex;
    gap: 2px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-subtle);
  }
  .toolbar-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    height: 26px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    cursor: pointer;
    transition:
      background 0.12s,
      color 0.12s;
  }
  .toolbar-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .editor {
    display: block;
    width: 100%;
    min-height: 72px;
    padding: 10px 14px;
    border: none;
    background: transparent;
    color: var(--text);
    font-family: var(--font-sans);
    font-size: var(--text-md);
    line-height: 1.6;
    resize: vertical;
    outline: none;
  }
  .editor::placeholder {
    color: var(--text-muted);
    font-style: italic;
  }
  /* A real button, so click and keyboard activation come for free rather than
     being bolted onto a div with role="button". */
  .display {
    display: block;
    width: 100%;
    padding: 10px 14px;
    min-height: 36px;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: text;
    transition: background 0.12s;
  }
  .display:hover {
    background: color-mix(in srgb, var(--bg-subtle) 50%, transparent);
  }
  .display:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
    border-radius: var(--radius-sm);
  }
  /* Must match .editor's size, or the text resizes when edit mode opens. */
  .plain {
    white-space: pre-wrap;
    font-size: var(--text-md);
    line-height: 1.6;
  }
  .placeholder {
    color: var(--text-muted);
    font-style: italic;
    font-size: var(--text-md);
  }
</style>
