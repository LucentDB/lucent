<script lang="ts">
  import ResultsGrid from '../grid/ResultsGrid.svelte';
  import CellGutter from './CellGutter.svelte';
  import CellToolbar from './CellToolbar.svelte';
  import { CELL_PAGE_SIZES } from '../../stores/notebook-view.ts';
  import type { FilterSpec } from '../../ipc/notebook';
  import type {
    NotebookModel,
    CellModel,
    TableOutput,
  } from '../../stores/notebook.svelte.ts';

  let {
    cell,
    model,
    selected = false,
    onSelect,
    onGripDown,
    children,
  }: {
    cell: CellModel;
    model: NotebookModel;
    selected?: boolean;
    onSelect?: () => void;
    onGripDown?: (e: PointerEvent) => void;
    children?: import('svelte').Snippet;
  } = $props();

  function isTable(o: unknown): o is TableOutput {
    return !!o && typeof o === 'object' && 'columns' in o;
  }

  let view = $derived(cell.view ?? model.cellView.stateFor(cell.id));

  // Markdown cells are neither executed nor referenceable by later cells.
  let runnable = $derived(cell.kind !== 'markdown');
  let referencable = $derived(cell.kind !== 'markdown');

  function staleTooltip(at: number | null): string {
    if (!at) return 'Output is stale — re-run this cell';
    const t = new Date(at).toLocaleTimeString([], {
      hour: 'numeric',
      minute: '2-digit',
    });
    return `Output stale since ${t} — re-run this cell`;
  }

  function errorMessage(e: typeof cell.error): string {
    if (!e) return '';
    const em = e as Record<string, unknown>;
    let msg = (em.message || em.hint || em.sql_error || '') as string;
    if (typeof msg === 'string' && msg.startsWith('{')) {
      try {
        const parsed = JSON.parse(msg);
        msg = parsed.message || parsed.error || msg;
      } catch {
        // Not valid JSON, use as-is
      }
    }
    return msg;
  }

  // ─── What counts as output worth rendering ──────────────────────────
  // A zero-column table is not a result, it is the backend's empty envelope for
  // a query that had nothing to execute. Rendering it produced the "No rows
  // found" panel on cells the user had never typed into.
  let table = $derived(isTable(cell.outputs) ? cell.outputs : null);
  let textContent = $derived(
    !isTable(cell.outputs) &&
      !!cell.outputs &&
      typeof (cell.outputs as { content?: unknown }).content === 'string'
      ? (cell.outputs as { content: string }).content
      : null,
  );

  let isTableOutput = $derived(!!table && table.columns.length > 0);
  let isTextOutput = $derived(!!textContent && textContent.length > 0);
  let hasOutput = $derived(
    cell.kind !== 'markdown' &&
      cell.kind !== 'ai' &&
      (isTableOutput || isTextOutput),
  );

  function formatDuration(ms: number | null): string {
    if (ms == null || ms <= 0) return '';
    return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
  }
  let durationLabel = $derived(formatDuration(cell.duration_ms));

  // Choosing a page size is only meaningful when the result outgrows the
  // smallest one. A single-row count(*) has nothing to page.
  let showPageSize = $derived(
    isTableOutput &&
      view.pageable &&
      !(view.isEnd && view.fetchedCount <= CELL_PAGE_SIZES[0]),
  );

  // ─── Collapsed summary ──────────────────────────────────────────────
  // A collapsed cell that renders nothing is dead weight. Showing the first
  // meaningful line plus the result size lets a folded notebook still be read.
  const SUMMARY_MAX = 140;

  let sourcePreview = $derived.by(() => {
    const line = cell.source
      .split('\n')
      .map((l) => l.trim())
      .find((l) => l.length > 0 && !l.startsWith('--'));
    if (!line)
      return cell.kind === 'markdown' ? 'Empty text cell' : 'Empty cell';
    return line.length > SUMMARY_MAX ? `${line.slice(0, SUMMARY_MAX)}…` : line;
  });

  let isEmptySource = $derived(cell.source.trim().length === 0);

  let outputSummary = $derived.by(() => {
    if (cell.status === 'error') return 'error';
    if (cell.status === 'running') return 'running…';
    if (table && table.rows_affected != null) {
      return `${table.rows_affected.toLocaleString()} rows affected`;
    }
    if (isTableOutput && table) {
      const n = table.total_count ?? table.rows.length;
      const approx = table.total_count == null && table.is_truncated ? '+' : '';
      return `${n.toLocaleString()}${approx} ${n === 1 ? 'row' : 'rows'}`;
    }
    if (isTextOutput) return 'text output';
    return null;
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="cell"
  class:selected
  class:running={cell.status === 'running'}
  class:error={cell.status === 'error'}
  class:collapsed={cell.collapsed}
  role="group"
  aria-label="{cell.kind} cell"
  onfocusin={onSelect}
  onpointerdown={onSelect}
>
  <CellGutter
    executionOrder={cell.execution_order}
    status={cell.status}
    {runnable}
    collapsed={cell.collapsed}
    onRun={() => model.runCell(cell.id)}
    onToggleCollapse={() => model.toggleCollapse(cell.id)}
    {onGripDown}
    onMoveUp={() => model.moveCell(cell.id, -1)}
    onMoveDown={() => model.moveCell(cell.id, 1)}
  />

  <div class="cell-body">
    {#if cell.collapsed}
      <button
        class="collapsed-summary"
        onclick={() => model.toggleCollapse(cell.id)}
        title="Expand cell"
        aria-label="Expand cell: {sourcePreview}"
      >
        <code class="summary-source" class:empty={isEmptySource}
          >{sourcePreview}</code
        >
        {#if outputSummary}
          <span class="summary-output">{outputSummary}</span>
        {/if}
      </button>
    {:else}
      <div class="cell-input-card" class:focused={selected}>
        <div class="cell-actions">
          <CellToolbar
            cellId={cell.id}
            cellStatus={cell.status}
            {referencable}
            onCancel={() => model.cancelCell(cell.id)}
            onDelete={() => model.deleteCell(cell.id)}
            onMoveUp={() => model.moveCell(cell.id, -1)}
            onMoveDown={() => model.moveCell(cell.id, 1)}
          />
        </div>

        <div class="cell-content">
          {#if children}
            <!-- CellList's snippet already closes over its own cell, so this
                 takes no argument. -->
            {@render children()}
          {/if}
        </div>
      </div>

      {#if cell.error}
        <div class="cell-error">
          <span class="error-icon" aria-hidden="true">⚠</span>
          <span class="error-message">{errorMessage(cell.error)}</span>
        </div>
      {/if}

      {#if hasOutput}
        <div class="cell-output" class:stale={cell.status === 'stale'}>
          <!-- The output prompt, as in Jupyter: the one unambiguous marker for
               where input ends and results begin. -->
          <div class="output-header">
            <span class="out-prompt">
              Out{#if cell.execution_order != null}&nbsp;[{cell.execution_order}]{/if}
            </span>
            {#if durationLabel}
              <span class="out-duration">{durationLabel}</span>
            {/if}
            <span class="header-spacer"></span>
            {#if cell.status === 'stale'}
              <span class="stale-badge" title={staleTooltip(cell.stale_since)}
                >Stale</span
              >
            {/if}
            {#if showPageSize}
              <label class="page-size-label">
                Rows
                <select
                  class="page-size-select"
                  value={String(view.pageSize)}
                  onchange={(e) =>
                    model.cellView.setPageSize(
                      cell.id,
                      Number((e.currentTarget as HTMLSelectElement).value),
                    )}
                >
                  {#each CELL_PAGE_SIZES as size}
                    <option value={String(size)}>{size}</option>
                  {/each}
                </select>
              </label>
            {/if}
          </div>
          {#if isTableOutput}
            <ResultsGrid
              columns={view.columns}
              rows={view.rows}
              fetchedCount={view.fetchedCount}
              totalCount={view.totalCount}
              isEnd={view.isEnd}
              loading={view.loading}
              embedded={true}
              pageSize={view.pageSize}
              tabId={cell.id}
              initFilters={view.filters}
              initSortCol={view.sortCol}
              initSortDir={view.sortDir}
              onStateChange={(s: {
                filters: FilterSpec[];
                sortCol: string | null;
                sortDir: 'asc' | 'desc';
              }) => model.cellView.applyState(cell.id, s)}
              onNeedMore={() => model.cellView.fetchMore(cell.id)}
              onCountAll={() => model.cellView.countAll(cell.id)}
            />
          {:else if isTextOutput}
            <pre class="text-output">{textContent}</pre>
          {/if}
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  /* ─── Cell container ──────────────────────────────────────────── */
  .cell {
    position: relative;
    display: flex;
    border-bottom: 1px solid var(--border-light, var(--border));
    background: transparent;
    transition: background 0.15s;
  }
  .cell:hover {
    background: color-mix(in srgb, var(--accent) 2%, transparent);
  }
  .cell.selected {
    background: color-mix(in srgb, var(--accent) 4%, transparent);
  }
  /* Selection state indicator, the notebook convention (Jupyter, Databricks,
     VS Code all mark the active row on its leading edge). State, not decoration. */
  .cell::before {
    content: '';
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: transparent;
    transition: background 0.15s;
  }
  .cell.selected::before,
  .cell.running::before {
    background: var(--accent);
  }
  .cell.error::before {
    background: var(--danger);
  }
  /* Running: a single sweep across the top edge. Conveys state, nothing else. */
  .cell.running::after {
    content: '';
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 2px;
    background: linear-gradient(
      90deg,
      transparent 0%,
      var(--accent) 50%,
      transparent 100%
    );
    background-size: 200% 100%;
    animation: shimmer 1.4s linear infinite;
    z-index: 1;
  }
  @keyframes shimmer {
    0% {
      background-position: -200% 0;
    }
    100% {
      background-position: 200% 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .cell.running::after {
      animation: none;
      background: var(--accent);
      opacity: 0.5;
    }
  }

  /* ─── Cell body ──────────────────────────────────────────────── */
  .cell-body {
    flex: 1;
    min-width: 0;
    position: relative;
    padding: 4px 10px 6px 0;
  }

  .cell-input-card {
    position: relative;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    box-shadow: var(--shadow-sm);
    transition:
      border-color 0.15s,
      box-shadow 0.15s;
  }
  .cell-input-card.focused,
  .cell.selected .cell-input-card {
    border-color: var(--accent);
    box-shadow:
      var(--ring-focus, 0 0 0 2px rgba(129, 140, 248, 0.35)), var(--shadow-md);
  }
  .cell.running .cell-input-card {
    border-color: var(--accent);
  }
  .cell.error .cell-input-card {
    border-color: var(--danger);
  }

  /* The action strip floats over the top-right of the body so it costs no
     vertical space. Wrapper is inert; CellToolbar owns its own hit targets. */
  .cell-actions {
    position: absolute;
    top: 4px;
    right: 8px;
    z-index: 2;
  }

  .cell-content {
    padding: 2px 4px 2px 4px;
  }

  /* ─── Collapsed summary ──────────────────────────────────────── */
  .collapsed-summary {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    min-height: 32px;
    padding: 6px 12px;
    border: none;
    background: none;
    color: var(--text-secondary);
    text-align: left;
    cursor: pointer;
  }
  .collapsed-summary:hover {
    color: var(--text);
  }
  .collapsed-summary:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .summary-source {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .summary-source.empty {
    font-family: var(--font-sans, inherit);
    font-style: italic;
    color: var(--text-muted);
  }
  .summary-output {
    flex-shrink: 0;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }

  /* ─── Error ──────────────────────────────────────────────────── */
  .cell-error {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 12px;
    margin-top: 4px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--danger) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger) 25%, transparent);
    font-size: var(--text-xs);
    color: var(--danger);
  }
  .error-icon {
    flex-shrink: 0;
    margin-top: 1px;
  }
  .error-message {
    font-family: var(--font-mono);
    word-break: break-word;
    line-height: 1.5;
  }

  /* ─── Output ─────────────────────────────────────────────────────
     A full-strength rule plus a tinted header bar marks the boundary. The rows
     themselves stay on the plain surface: the embedded grid is transparent, so
     tinting the whole region would wash over the data. */
  .cell-output {
    margin-top: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
    background: var(--bg-surface);
  }
  .cell-output.stale {
    opacity: 0.65;
  }
  .output-header {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 26px;
    padding: 3px 10px;
    font-size: var(--text-xs);
    color: var(--text-muted);
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border-light, var(--border));
  }
  .header-spacer {
    flex: 1;
  }
  /* Mirrors the gutter's `[n]` input counter, so In and Out read as a pair. */
  .out-prompt {
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: var(--weight-bold);
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--accent);
    background: var(--accent-soft);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    white-space: nowrap;
  }
  .out-duration {
    font-size: 10px;
    font-variant-numeric: tabular-nums;
  }
  .out-duration::before {
    content: '·';
    margin-right: 6px;
  }
  .stale-badge {
    padding: 1px 6px;
    border: 1px solid var(--warning);
    border-radius: var(--radius-full);
    color: var(--warning);
    font-size: 10px;
    font-weight: var(--weight-medium);
    cursor: help;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .page-size-label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .page-size-select {
    padding: 2px 5px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-surface);
    color: var(--text);
    font-size: var(--text-xs);
    cursor: pointer;
  }
  .text-output {
    margin: 0;
    padding: 10px 14px;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.6;
    white-space: pre-wrap;
    color: var(--text-secondary);
    max-height: 300px;
    overflow-y: auto;
  }
</style>
