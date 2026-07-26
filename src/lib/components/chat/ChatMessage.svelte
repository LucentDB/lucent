<script lang="ts">
  import type { ChatMessage as T } from '../../stores/chat.svelte.ts';
  import { renderMarkdown } from './markdown.ts';
  import DmlApprovalCard from './DmlApprovalCard.svelte';
  import WorkSession from './WorkSession.svelte';
  import { setSessionExpanded } from '../../stores/chat.svelte.ts';

  let {
    message,
    onRunDml,
    onCancelDml,
    grouped = false,
    conversationId,
  }: {
    message: T;
    onRunDml?: () => void;
    onCancelDml?: () => void;
    grouped?: boolean; // consecutive message from the same role — tighter spacing
    conversationId?: string; // NEW — for updating thinking state
  } = $props();

  let rendered = $derived(renderMarkdown(message.content));

  function handleSessionToggle(expanded: boolean) {
    if (conversationId) {
      setSessionExpanded(conversationId, message.id, expanded);
    }
  }
</script>

<div class="message {message.role}" class:grouped>
  <div class="bubble">
    {#if message.session}
      <WorkSession session={message.session} onToggle={handleSessionToggle} />
    {/if}

    {#if message.content}
      <div class="text">{@html rendered}</div>
    {/if}

    {#if message.dmlApproval}
      <DmlApprovalCard
        dml={message.dmlApproval}
        onRun={onRunDml}
        onCancel={onCancelDml}
      />
    {/if}

    {#if message.usage}
      <div class="usage">
        ~{message.usage.promptTokens + message.usage.completionTokens} tokens
        {#if message.usage.estimatedCostUsd !== null}
          · ${message.usage.estimatedCostUsd.toFixed(4)}
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .message {
    display: flex;
    padding: 3px 0;
    animation: msg-in 0.2s ease-out;
  }

  .message.grouped {
    padding-top: 0;
    margin-top: -1px;
  }

  @keyframes msg-in {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .message.user {
    justify-content: flex-end;
  }
  .message.assistant {
    justify-content: flex-start;
  }

  .bubble {
    min-width: 0;
  }

  .message.assistant .bubble {
    flex: 1;
  }

  .message.user .bubble {
    max-width: 82%;
    background: var(--bg-subtle);
    border-radius: var(--radius-lg);
    padding: 8px 14px;
  }

  /* ── Markdown rendered text (default marked output) ── */
  .text {
    line-height: 1.65;
    font-size: var(--text-md);
    color: var(--text);
  }

  .text :global(p) {
    margin: 0 0 10px;
  }
  .text :global(p:last-child) {
    margin-bottom: 0;
  }

  .text :global(h1),
  .text :global(h2),
  .text :global(h3),
  .text :global(h4) {
    margin: 16px 0 8px;
    font-weight: var(--weight-semibold);
    line-height: 1.3;
  }
  .text :global(h1) {
    font-size: var(--text-xl);
  }
  .text :global(h2) {
    font-size: var(--text-lg);
  }
  .text :global(h3) {
    font-size: var(--text-md);
  }
  .text :global(h4) {
    font-size: var(--text-base);
  }

  .text :global(ul),
  .text :global(ol) {
    margin: 4px 0 10px;
    padding-left: 22px;
  }
  .text :global(li) {
    margin: 3px 0;
  }
  .text :global(li > ul),
  .text :global(li > ol) {
    margin: 2px 0;
  }

  .text :global(code) {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    background: var(--bg-subtle);
    padding: 1px 6px;
    border-radius: 3px;
    color: var(--accent);
  }

  .text :global(pre) {
    margin: 10px 0;
    padding: 12px 14px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow-x: auto;
  }

  .text :global(pre code) {
    background: none;
    padding: 0;
    color: inherit;
    font-size: var(--text-sm);
    line-height: 1.6;
  }

  .text :global(strong) {
    font-weight: var(--weight-semibold);
  }
  .text :global(em) {
    font-style: italic;
  }

  .text :global(blockquote) {
    margin: 8px 0;
    padding: 6px 12px;
    border-left: 3px solid var(--accent);
    background: var(--bg-subtle);
    border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
    color: var(--text-secondary);
  }
  .text :global(blockquote p) {
    margin: 0;
  }

  .text :global(hr) {
    margin: 16px 0;
    border: none;
    border-top: 1px solid var(--border);
  }

  .text :global(table) {
    margin: 10px 0;
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
  }

  .text :global(th) {
    padding: 6px 10px;
    text-align: left;
    font-weight: var(--weight-semibold);
    color: var(--text-secondary);
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }

  .text :global(td) {
    padding: 5px 10px;
    border-bottom: 1px solid var(--border-light);
  }

  .text :global(tr:last-child td) {
    border-bottom: none;
  }
  .text :global(tbody tr:hover) {
    background: var(--bg-hover);
  }

  .text :global(a) {
    color: var(--accent);
    text-decoration: none;
  }
  .text :global(a:hover) {
    text-decoration: underline;
  }

  .text :global(img) {
    max-width: 100%;
    border-radius: var(--radius-md);
    margin: 8px 0;
  }

  .usage {
    font-size: var(--text-xs);
    color: var(--text-muted);
    margin-top: 6px;
    padding-top: 6px;
    border-top: 1px solid var(--border-light);
  }
</style>
