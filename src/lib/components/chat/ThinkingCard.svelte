<script lang="ts">
  import { slide } from 'svelte/transition';
  import { renderMarkdown } from './markdown.ts';

  let {
    content,
    durationMs,
  }: {
    content: string;
    durationMs?: number;
  } = $props();

  let open = $state(false);
  let rendered = $derived(renderMarkdown(content));
  let bodyEl: HTMLDivElement | undefined = $state();

  // Auto-expand while streaming, close when finalized.
  $effect(() => {
    open = durationMs === undefined;
  });

  // Auto-scroll the body as thinking content streams in.
  $effect(() => {
    if (durationMs === undefined && bodyEl) {
      void content; // establish reactive dependency on streaming content
      bodyEl.scrollTop = bodyEl.scrollHeight;
    }
  });

  function formatDuration(ms: number | undefined): string {
    if (ms === undefined) return 'Thinking…';
    const seconds = Math.max(1, Math.round(ms / 1000));
    return `Thought for ${seconds}s`;
  }
</script>

<div class="tc">
  <button class="tc-hdr" onclick={() => (open = !open)} type="button">
    <span class="tc-label">{formatDuration(durationMs)}</span>
    <svg
      class="tc-chevron"
      class:open
      width="10"
      height="10"
      viewBox="0 0 16 16"
      fill="none"
    >
      <path
        d="M6 4l4 4-4 4"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  </button>

  {#if open}
    <div
      class="tc-body"
      bind:this={bodyEl}
      transition:slide={{ duration: 150 }}
    >
      {@html rendered}
    </div>
  {/if}
</div>

<style>
  .tc {
    margin: 2px 0;
  }

  .tc-hdr {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 2px 0;
    background: none;
    border: none;
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-muted);
    text-align: left;
  }

  .tc-hdr:hover {
    color: var(--text-secondary);
  }

  .tc-label {
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
  }

  .tc-chevron {
    flex-shrink: 0;
    color: var(--text-muted);
    transition: transform 0.15s ease;
  }

  .tc-chevron.open {
    transform: rotate(90deg);
  }

  .tc-body {
    margin-top: 4px;
    margin-left: 18px;
    padding: 6px 8px;
    background: var(--bg-subtle);
    border-radius: var(--radius-sm);
    font-size: 0.9em;
    color: var(--text-secondary);
    line-height: 1.6;
    max-height: 240px;
    overflow-y: auto;
  }

  .tc-body :global(p) {
    margin: 0 0 6px;
  }
  .tc-body :global(p:last-child) {
    margin-bottom: 0;
  }
  .tc-body :global(code) {
    font-family: var(--font-mono);
    font-size: 0.9em;
    background: var(--bg-surface);
    padding: 1px 4px;
    border-radius: 3px;
  }
  .tc-body :global(pre) {
    margin: 6px 0;
    padding: 6px 8px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow-x: auto;
  }
</style>
