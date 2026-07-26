<script lang="ts">
  import { slide } from 'svelte/transition';
  import { renderMarkdown } from './markdown.ts';
  import ToolCallCard from './ToolCallCard.svelte';
  import ThinkingCard from './ThinkingCard.svelte';
  import type { WorkSession as T } from '../../stores/chat.svelte.ts';

  let {
    session,
    onToggle,
  }: {
    session: T;
    onToggle?: (expanded: boolean) => void;
  } = $props();

  let kind = $derived(
    session.segments.some((s) => s.type === 'tool_call') ? 'Worked' : 'Thought',
  );
  let showExpanded = $derived(session.expanded ?? session.active);

  function formatDuration(ms: number | undefined): string {
    const seconds = Math.max(1, Math.round((ms ?? 0) / 1000));
    return `${seconds}s`;
  }

  let headerText = $derived(
    session.active
      ? kind === 'Worked'
        ? 'Working…'
        : 'Thinking…'
      : `${kind} for ${formatDuration(session.durationMs)}`,
  );

  function handleClick() {
    onToggle?.(!showExpanded);
  }

  let bodyEl: HTMLDivElement | undefined = $state();

  let contentVersion = $derived.by(() => {
    // Track segment count AND last segment text length so auto-scroll
    // fires for both new segments and in-streaming content appends.
    const n = session.segments.length;
    if (n === 0) return 0;
    const last = session.segments[n - 1] as { content?: string };
    return n + (last.content?.length ?? 0);
  });

  $effect(() => {
    if (session.active && bodyEl) {
      void contentVersion;
      bodyEl.scrollTop = bodyEl.scrollHeight;
    }
  });
</script>

<div class="work-session">
  <button class="session-header" onclick={handleClick} type="button">
    <span class="session-label">{headerText}</span>
    <svg
      class="chevron"
      class:expanded={showExpanded}
      width="12"
      height="12"
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

  {#if showExpanded}
    <div
      class="session-body"
      bind:this={bodyEl}
      transition:slide={{ duration: 200 }}
    >
      {#each session.segments as segment}
        {#if segment.type === 'thinking'}
          <div class="segment segment-tool">
            <ThinkingCard
              content={segment.content}
              durationMs={segment.durationMs}
            />
          </div>
        {:else if segment.type === 'note'}
          <div class="segment segment-note">
            {@html renderMarkdown(segment.content)}
          </div>
        {:else if segment.type === 'tool_call'}
          <div class="segment segment-tool">
            <ToolCallCard tool={segment.call} />
          </div>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .work-session {
    display: flex;
    flex-direction: column;
    margin: 4px 0;
  }

  .session-header {
    display: flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: none;
    padding: 2px 0;
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .session-label {
    font-weight: 400;
  }

  .chevron {
    color: var(--text-muted);
    transition: transform 0.15s ease;
  }

  .chevron.expanded {
    transform: rotate(90deg);
  }

  .session-body {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 4px;
    padding-left: 10px;
    border-left: 1px solid var(--border-light, #f3f4f6);
    max-height: 320px;
    overflow-y: auto;
  }

  .segment-tool {
    margin: 0;
  }
</style>
