<script lang="ts">
  // Attribution popover surfaced from an assistant message's memory pill
  // (F-C2 / R24). It shows which learned rules were applied and offers a
  // lightweight feedback affordance. Feedback is LOCAL state only — there is
  // no backend feedback command in this plan, so we never call one.
  let {
    ruleIds,
    onClose,
    onOpenDrawer,
  }: {
    ruleIds: string[];
    onClose?: () => void;
    onOpenDrawer?: () => void;
  } = $props();

  type Feedback = 'up' | 'down' | null;
  let feedback = $state<Feedback>(null);
  let reported = $state(false);

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onClose?.();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="attribution-popover" role="dialog" aria-label="Applied rules">
  <div class="attr-hdr">
    <span class="attr-icon">🧠</span>
    <span class="attr-title">Applied rules</span>
    <button
      class="attr-close"
      aria-label="Close attribution popover"
      title="Close"
      onclick={onClose}>×</button
    >
  </div>

  {#if ruleIds.length > 0}
    <ul class="attr-rules">
      {#each ruleIds as id (id)}
        <li class="attr-rule">{id}</li>
      {/each}
    </ul>
  {:else}
    <p class="attr-empty">No rule details available</p>
  {/if}

  <div class="attr-feedback">
    <button
      class="attr-fb"
      class:selected={feedback === 'up'}
      aria-pressed={feedback === 'up'}
      title="These rules helped"
      onclick={() => (feedback = feedback === 'up' ? null : 'up')}
    >
      👍 Helpful
    </button>
    <button
      class="attr-fb"
      class:selected={feedback === 'down'}
      aria-pressed={feedback === 'down'}
      title="These rules were not helpful"
      onclick={() => (feedback = feedback === 'down' ? null : 'down')}
    >
      👎 Not helpful
    </button>
    <button
      class="attr-fb"
      class:selected={reported}
      aria-pressed={reported}
      title="Report these rules"
      onclick={() => (reported = !reported)}
    >
      Report
    </button>
  </div>

  <div class="attr-actions">
    <button class="attr-open" onclick={onOpenDrawer}>Open Memory Drawer</button>
  </div>
</div>

<style>
  .attribution-popover {
    margin-top: 6px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    max-width: 320px;
    font-size: var(--text-xs);
  }

  .attr-hdr {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 8px;
  }
  .attr-icon {
    font-size: 14px;
  }
  .attr-title {
    flex: 1;
    font-weight: var(--weight-semibold);
    font-size: var(--text-sm);
    color: var(--text);
  }
  .attr-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
    flex-shrink: 0;
  }
  .attr-close:hover {
    background: var(--bg-hover);
    color: var(--text);
  }

  .attr-rules {
    list-style: none;
    margin: 0 0 8px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .attr-rule {
    font-family: var(--font-mono);
    color: var(--text-secondary);
    background: var(--bg-subtle);
    padding: 3px 8px;
    border-radius: var(--radius-sm);
  }
  .attr-empty {
    margin: 0 0 8px;
    color: var(--text-muted);
  }

  .attr-feedback {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 8px;
  }
  .attr-fb {
    background: transparent;
    border: 1px solid var(--border);
    border-radius: var(--radius-full);
    color: var(--text-secondary);
    font-size: var(--text-xs);
    padding: 3px 8px;
    cursor: pointer;
    transition: all var(--transition-fast);
  }
  .attr-fb:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .attr-fb.selected {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }

  .attr-actions {
    display: flex;
    justify-content: flex-end;
  }
  .attr-open {
    background: transparent;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    font-size: var(--text-xs);
    padding: 5px 12px;
    cursor: pointer;
    transition: all var(--transition-fast);
  }
  .attr-open:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
</style>
