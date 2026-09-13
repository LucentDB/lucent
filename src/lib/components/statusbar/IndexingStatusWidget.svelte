<script lang="ts">
  import { indexing, stageLabel } from '../../stores/indexing.svelte';
  import IndexingDetailsPopover from './IndexingDetailsPopover.svelte';

  interface Props {
    connectionName?: string;
    databaseName?: string;
  }

  let { connectionName = 'Active Connection', databaseName = '' }: Props = $props();

  let widgetRef = $state<HTMLElement | null>(null);

  function handleClick(e: MouseEvent) {
    e.stopPropagation();
    indexing.toggleDetails();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && indexing.detailsOpen) {
      indexing.toggleDetails(false);
    }
  }

  $effect(() => {
    function onDocClick(e: MouseEvent) {
      if (indexing.detailsOpen && widgetRef && !widgetRef.contains(e.target as Node)) {
        indexing.toggleDetails(false);
      }
    }
    window.addEventListener('click', onDocClick);
    window.addEventListener('keydown', handleKeydown);
    return () => {
      window.removeEventListener('click', onDocClick);
      window.removeEventListener('keydown', handleKeydown);
    };
  });
</script>

<div class="status-widget-container" bind:this={widgetRef}>
  <button
    class="status-widget-btn"
    class:active={!indexing.isComplete}
    class:popover-open={indexing.detailsOpen}
    onclick={handleClick}
    aria-label="Schema indexing status"
    title={indexing.isComplete ? 'Schema indexing up to date (Click for details)' : indexing.text}
  >
    {#if !indexing.isComplete}
      <span class="spinner-icon" aria-hidden="true"></span>
      <span class="widget-label">
        {#if indexing.stage === 'delta'}
          Delta Scan…
        {:else if indexing.percent > 0}
          Indexing {indexing.percent}%
        {:else}
          {stageLabel(indexing.stage)}
        {/if}
      </span>
      <div class="micro-progress">
        <div class="micro-progress-fill" style="width: {indexing.percent}%"></div>
      </div>
    {:else}
      <svg class="idle-icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <ellipse cx="12" cy="5" rx="9" ry="3"></ellipse>
        <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"></path>
        <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"></path>
      </svg>
      <span class="widget-label quiet">Indexed</span>
    {/if}
  </button>

  {#if indexing.detailsOpen}
    <div class="popover-anchor">
      <IndexingDetailsPopover
        {connectionName}
        {databaseName}
        onClose={() => indexing.toggleDetails(false)}
      />
    </div>
  {/if}
</div>

<style>
  .status-widget-container {
    position: relative;
    display: inline-flex;
    align-items: center;
    height: 100%;
  }

  .status-widget-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 100%;
    padding: 0 8px;
    background: transparent;
    border: none;
    color: var(--text-secondary);
    font-family: var(--font-sans);
    font-size: var(--text-xs);
    cursor: pointer;
    position: relative;
    user-select: none;
    transition: background var(--transition-fast), color var(--transition-fast);
  }

  .status-widget-btn:hover,
  .status-widget-btn.popover-open {
    background: var(--bg-hover);
    color: var(--text);
  }

  .status-widget-btn.active {
    color: var(--accent);
  }

  .spinner-icon {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid var(--accent);
    border-top-color: transparent;
    animation: widget-spin 0.8s linear infinite;
  }

  @keyframes widget-spin {
    to { transform: rotate(360deg); }
  }

  .idle-icon {
    color: var(--text-muted);
    opacity: 0.85;
  }

  .widget-label {
    font-size: 11px;
    white-space: nowrap;
  }

  .widget-label.quiet {
    color: var(--text-muted);
  }

  .micro-progress {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--accent-soft);
    overflow: hidden;
  }

  .micro-progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s ease-out;
  }

  .popover-anchor {
    position: absolute;
    bottom: calc(100% + 6px);
    right: 0;
    z-index: 1100;
  }
</style>
