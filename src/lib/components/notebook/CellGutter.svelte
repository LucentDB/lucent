<script lang="ts">
  /**
   * The cell gutter, following the Jupyter/Colab/Databricks convention:
   * the execution counter and the run button are the SAME control, anchored to
   * the first line of code. At rest it reads `[4]` (or `[ ]` when never run);
   * on hover it becomes a play triangle. Nothing here is anchored to the cell's
   * vertical centre, because a cell's height is driven by its output and any
   * centred control drifts arbitrarily far from the code it acts on.
   */
  let {
    executionOrder,
    status,
    runnable = true,
    collapsed = false,
    onRun,
    onToggleCollapse,
    onGripDown,
    onMoveUp,
    onMoveDown,
  }: {
    executionOrder: number | null;
    status: string;
    runnable?: boolean;
    collapsed?: boolean;
    onRun?: () => void;
    onToggleCollapse?: () => void;
    onGripDown?: (e: PointerEvent) => void;
    onMoveUp?: () => void;
    onMoveDown?: () => void;
  } = $props();

  let running = $derived(status === 'running');
  let counter = $derived(
    executionOrder != null ? `In [${executionOrder}]` : 'In [ ]',
  );

  // Keyboard reorder, so the drag grip is not the only way to move a cell.
  function gripKeydown(e: KeyboardEvent) {
    if (e.altKey && e.key === 'ArrowUp') {
      e.preventDefault();
      onMoveUp?.();
    } else if (e.altKey && e.key === 'ArrowDown') {
      e.preventDefault();
      onMoveDown?.();
    }
  }
</script>

<div class="cell-gutter" class:collapsed>
  {#if runnable}
    <button
      class="gutter-run"
      class:running
      onclick={running ? undefined : onRun}
      disabled={running}
      title={running ? 'Running…' : 'Run cell (⌘⏎)'}
      aria-label={running ? 'Cell is running' : 'Run cell'}
    >
      <span class="run-stack">
        <span class="run-counter" aria-live="polite">
          {#if running}
            <span class="spinner" aria-hidden="true"></span>
          {:else}
            <span
              class="order-number"
              class:order-empty={executionOrder == null}>{counter}</span
            >
          {/if}
        </span>
        {#if !running}
          <svg
            class="run-glyph"
            width="9"
            height="9"
            viewBox="0 0 16 16"
            fill="currentColor"
            aria-hidden="true"
          >
            <polygon points="3 1 13 8 3 15" />
          </svg>
        {/if}
      </span>
    </button>
  {:else}
    <!-- Markdown cells have no execution: hold the column width, show nothing. -->
    <div class="gutter-spacer" aria-hidden="true"></div>
  {/if}

  <!-- Out of flow while expanded: a stacked column of secondary controls is
       taller than a one-line editor, and would then dictate the cell's height,
       leaving dead space under short queries. -->
  <div class="gutter-secondary">
    <button
      class="gutter-collapse"
      onclick={onToggleCollapse}
      aria-label={collapsed ? 'Expand cell' : 'Collapse cell'}
      aria-expanded={!collapsed}
      title={collapsed ? 'Expand cell' : 'Collapse cell'}
    >
      <svg
        class="chevron"
        class:collapsed
        width="9"
        height="9"
        viewBox="0 0 16 16"
        fill="none"
        aria-hidden="true"
      >
        <path
          d="M6 4l4 4-4 4"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </button>

    {#if !collapsed}
      <div
        class="gutter-grip"
        onpointerdown={(e) => {
          e.preventDefault();
          onGripDown?.(e);
        }}
        onkeydown={gripKeydown}
        role="button"
        tabindex="0"
        aria-label="Reorder cell — drag, or Alt with arrow keys"
        title="Drag to reorder (Alt+↑/↓)"
      >
        <svg width="8" height="12" viewBox="0 0 8 12" aria-hidden="true">
          <circle cx="2" cy="2" r="1.1" fill="currentColor" />
          <circle cx="6" cy="2" r="1.1" fill="currentColor" />
          <circle cx="2" cy="6" r="1.1" fill="currentColor" />
          <circle cx="6" cy="6" r="1.1" fill="currentColor" />
          <circle cx="2" cy="10" r="1.1" fill="currentColor" />
          <circle cx="6" cy="10" r="1.1" fill="currentColor" />
        </svg>
      </div>
    {/if}
  </div>
</div>

<style>
  /* Top-aligned, never centred: see the component comment. padding-top lines the
     run control up with the first line of CodeMirror text (3px cell-content pad
     + 10px cm-content pad + half a 19.6px line ≈ 22px centre). */
  .cell-gutter {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    width: 52px;
    flex-shrink: 0;
    /* In-flow height is the run control alone (10 + 24 + 8 = 42px), which is
       just over a one-line editor, so the gutter never pads the cell out. */
    padding: 10px 4px 8px;
    font-family: var(--font-mono);
    color: var(--text-muted);
    user-select: none;
  }
  /* Collapsed: keep the same column layout as expanded cells, just reduce
     vertical padding so the cell height shrinks to fit the summary. */
  .cell-gutter.collapsed {
    padding: 4px 4px;
  }
  .gutter-secondary {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 1px;
  }
  /* Collapsed: hide secondary controls (chevron+grip) to keep the cell compact. */
  .cell-gutter.collapsed .gutter-secondary {
    display: none;
  }

  /* ─── Run / execution counter (one control) ────────────────────── */
  .gutter-run {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 44px;
    height: 24px;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    font-family: var(--font-mono);
    transition:
      background 0.15s,
      color 0.15s;
  }
  .gutter-run:hover:not(.running) {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .gutter-run:active:not(.running) {
    background: var(--accent-muted);
  }
  .gutter-run.running {
    cursor: default;
    color: var(--accent);
  }
  .gutter-run:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  /* Counter and play glyph occupy the same grid cell so the swap costs no
     layout: only opacity animates. */
  .run-stack {
    display: grid;
    place-items: center;
  }
  .run-stack > * {
    grid-area: 1 / 1;
  }
  .run-counter {
    display: flex;
    align-items: center;
    justify-content: center;
    transition: opacity 0.15s;
  }
  .run-glyph {
    opacity: 0;
    transition: opacity 0.15s;
  }
  .gutter-run:hover:not(.running) .run-counter {
    opacity: 0;
  }
  .gutter-run:hover:not(.running) .run-glyph {
    opacity: 1;
  }

  .order-number {
    font-size: 10px;
    font-weight: var(--weight-medium);
    font-variant-numeric: tabular-nums;
    color: var(--text-secondary);
    white-space: nowrap;
    line-height: 1;
  }
  .order-number.order-empty {
    color: var(--text-muted);
    font-weight: var(--weight-normal, 400);
  }

  .gutter-spacer {
    width: 30px;
    height: 24px;
    flex-shrink: 0;
  }

  /* ─── Collapse ─────────────────────────────────────────────────── */
  .gutter-collapse {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-muted);
    cursor: pointer;
    opacity: 0;
    transition:
      opacity 0.15s,
      background 0.15s,
      color 0.15s;
  }
  /* A collapsed cell must always expose the way back out. */
  .cell-gutter.collapsed .gutter-collapse,
  :global(.cell:hover) .gutter-collapse,
  :global(.cell.selected) .gutter-collapse,
  .gutter-collapse:focus-visible {
    opacity: 1;
  }
  .gutter-collapse:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .chevron {
    transform: rotate(90deg);
    transition: transform 0.15s ease-out;
  }
  .chevron.collapsed {
    transform: rotate(0deg);
  }

  /* ─── Drag grip ────────────────────────────────────────────────── */
  .gutter-grip {
    color: var(--text-muted);
    cursor: grab;
    opacity: 0;
    padding: 2px;
    border-radius: var(--radius-sm);
    transition:
      opacity 0.15s,
      background 0.15s;
  }
  .gutter-grip:active {
    cursor: grabbing;
  }
  :global(.cell:hover) .gutter-grip {
    opacity: 0.45;
  }
  :global(.cell:hover) .gutter-grip:hover,
  .gutter-grip:focus-visible {
    opacity: 1;
    background: var(--bg-hover);
  }

  .gutter-collapse:focus-visible,
  .gutter-grip:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .spinner {
    display: inline-block;
    width: 11px;
    height: 11px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
