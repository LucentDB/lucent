<script lang="ts">
  import { untrack } from 'svelte';
  import ThinkingCard from '../chat/ThinkingCard.svelte';
  import ToolCallCard from '../chat/ToolCallCard.svelte';
  import MarkdownBody from './MarkdownBody.svelte';
  import SqlBlock from './SqlBlock.svelte';
  import ResultsGrid from '../grid/ResultsGrid.svelte';
  import { CELL_PAGE_SIZES } from '../../stores/notebook-view.ts';
  import type { ToolCallCard as ToolCallCardType } from '../../stores/chat.svelte.ts';
  import type {
    CellModel,
    NotebookModel,
    TableOutput,
  } from '../../stores/notebook.svelte.ts';

  let {
    cell,
    model,
    onEditSql,
  }: {
    cell: CellModel;
    model: NotebookModel;
    onEditSql?: (sql: string) => void;
  } = $props();

  type TabId = 'response' | 'sql' | 'table';

  let aiState = $derived(cell.ai_state);
  let messages = $derived(aiState?.messages ?? []);
  let toolCalls = $derived.by<ToolCallCardType[]>(
    () => (aiState?.tool_calls ?? []) as ToolCallCardType[],
  );
  let finalSql = $derived(aiState?.final_sql ?? null);
  let response = $derived(aiState?.response ?? null);

  let isRunning = $derived(cell.status === 'running');
  let hasActivity = $derived(messages.length > 0 || toolCalls.length > 0);

  // Interleave thinking messages and tool calls in chronological order.
  // The backend streams them in strict alternation: Think → Tool → Think →
  // Tool → ..., and each array grows in lockstep.  Pairing by index is
  // therefore correct for the current protocol.  If the backend ever sends
  // parallel tool calls or unpairs the two streams, this derivation must be
  // replaced with a merge based on explicit timestamps or sequence numbers.
  // Pattern: Think₁ → ToolCall₁ → Think₂ → ToolCall₂ → ...
  type ActivityItem =
    | { type: 'thinking'; msg: Record<string, unknown> }
    | { type: 'tool'; tool: ToolCallCardType };

  let activityItems = $derived.by<ActivityItem[]>(() => {
    const items: ActivityItem[] = [];
    const msgs = messages.filter(
      (m) => typeof m === 'object' && m !== null && 'thinking' in m,
    );
    const maxLen = Math.max(msgs.length, toolCalls.length);
    for (let i = 0; i < maxLen; i++) {
      // Thinking comes before the tool call it precedes
      if (i < msgs.length) {
        items.push({
          type: 'thinking',
          msg: msgs[i] as Record<string, unknown>,
        });
      }
      if (i < toolCalls.length) {
        items.push({ type: 'tool', tool: toolCalls[i] });
      }
    }
    return items;
  });

  function isTable(o: unknown): o is TableOutput {
    return !!o && typeof o === 'object' && 'columns' in o;
  }
  let tableOutput = $derived(isTable(cell.outputs) ? cell.outputs : null);

  let hasResponse = $derived(!!response && response.length > 0);
  let hasSql = $derived(!!finalSql);
  let hasTable = $derived(!!tableOutput);

  const TABS: { id: TabId; label: string }[] = [
    { id: 'response', label: 'Response' },
    { id: 'sql', label: 'SQL Code' },
    { id: 'table', label: 'Table' },
  ];

  function isEnabled(id: TabId): boolean {
    if (id === 'response') return hasResponse;
    if (id === 'sql') return hasSql;
    return hasTable;
  }

  /**
   * Only tabs that hold something. Three permanent tabs, two of them greyed
   * out, advertised results the cell had not produced — on a cell that had not
   * run yet, the whole strip was a promise of nothing.
   */
  let availableTabs = $derived(TABS.filter((t) => isEnabled(t.id)));

  let activeTab = $state<TabId>('response');
  let sqlCopied = $state(false);
  let activityOpen = $state(true);
  let defaultedForToken = $state<number | null>(null);

  /**
   * Defaults the tab once per run, keyed on run_token. Defaulting on every
   * content change yanked the tab back mid-stream and overrode the user's
   * choice. The second branch only repairs a selection that lost its content.
   */
  $effect(() => {
    const token = cell.run_token ?? 0;
    const tabs = availableTabs;
    const current = untrack(() => activeTab);
    if (defaultedForToken !== token) {
      defaultedForToken = token;
      activeTab = tabs[0]?.id ?? 'response';
    } else if (tabs.length > 0 && !tabs.some((t) => t.id === current)) {
      activeTab = tabs[0].id;
    }
  });

  // Collapse the activity log once the run finishes, but keep it available.
  $effect(() => {
    if (!isRunning) activityOpen = false;
  });

  // Reads the reactive cell.view mirror (written by cellView.put) rather than
  // only the internal Map — the Map is plain, so a $derived over it alone would
  // never re-evaluate after fetchMore/applyState/setPageSize.
  let view = $derived(cell.view ?? model.cellView.stateFor(cell.id));

  // Only worth offering when the result outgrows the smallest page size.
  let showPageSize = $derived(
    view.pageable && !(view.isEnd && view.fetchedCount <= CELL_PAGE_SIZES[0]),
  );

  function copySql(sql: string) {
    navigator.clipboard?.writeText(sql);
    sqlCopied = true;
    setTimeout(() => (sqlCopied = false), 2000);
  }

  function formatDuration(ms: number | null): string {
    if (!ms) return '';
    return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
  }

  let statusLine = $derived.by(() => {
    if (isRunning) return 'Thinking…';
    const parts: string[] = [];
    if (toolCalls.length) {
      parts.push(
        `${toolCalls.length} tool ${toolCalls.length === 1 ? 'call' : 'calls'}`,
      );
    }
    const d = formatDuration(cell.duration_ms);
    if (d) parts.push(d);
    return parts.join(' · ');
  });

  // Results are hidden mid-run: a half-streamed table is worse than none.
  let showTabs = $derived(!isRunning && availableTabs.length > 0);

  // Nothing produced and nothing in progress: render no chrome at all.
  let hasAnything = $derived(
    isRunning || hasActivity || showTabs || !!statusLine,
  );
</script>

{#if hasAnything}
  <div class="ai-output">
    <!-- One header bar carries both the tab strip and the run status, so the
         boundary between the prompt above and the answer below is a single
         rule rather than two stacked meta bars. -->
    {#if showTabs || statusLine}
      <div class="output-header">
        {#if showTabs}
          <div class="tabs-header" role="tablist">
            {#each availableTabs as tab}
              <button
                class="tab-btn"
                class:active={activeTab === tab.id}
                role="tab"
                aria-selected={activeTab === tab.id}
                title={tab.label}
                onclick={() => (activeTab = tab.id)}
                type="button"
              >
                {tab.label}
              </button>
            {/each}
          </div>
          <!-- Only when tabs precede it, so a lone status line stays left-aligned. -->
          <span class="header-spacer"></span>
        {/if}
        {#if statusLine}
          <button
            class="activity-status"
            class:inert={!hasActivity}
            onclick={() => hasActivity && (activityOpen = !activityOpen)}
            disabled={!hasActivity}
            type="button"
            aria-expanded={hasActivity ? activityOpen : undefined}
            title={hasActivity ? 'Show what the model did' : ''}
          >
            <span class="activity-label">{statusLine}</span>
            {#if hasActivity}
              <svg
                class="chevron"
                class:open={activityOpen}
                width="10"
                height="10"
                viewBox="0 0 16 16"
                fill="none"
                aria-hidden="true"
              >
                <path
                  d="M6 4l4 4-4 4"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </svg>
            {/if}
          </button>
        {/if}
      </div>
    {/if}

    {#if hasActivity && activityOpen}
      <div class="activity">
        <div class="activity-body">
          {#each activityItems as item}
            {#if item.type === 'thinking'}
              <ThinkingCard
                content={item.msg.thinking as string}
                durationMs={isRunning
                  ? undefined
                  : ((item.msg.durationMs as number | undefined) ??
                    cell.duration_ms ??
                    1000)}
              />
            {:else}
              <ToolCallCard tool={item.tool} cellCompleted={!isRunning} />
            {/if}
          {/each}
        </div>
      </div>
    {/if}

    {#if showTabs}
      <div class="tabs-body">
        {#if activeTab === 'response' && response}
          <div class="pad">
            <MarkdownBody source={response} />
          </div>
        {:else if activeTab === 'sql' && finalSql}
          <div class="sql-panel">
            <div class="sql-actions">
              {#if onEditSql}
                <button
                  class="icon-btn insert-sql-btn"
                  onclick={() => onEditSql?.(finalSql)}
                  type="button"
                  title="Insert into next SQL cell"
                  aria-label="Insert into next SQL cell"
                >
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                  >
                    <path d="M12 5v14M5 12h14" />
                  </svg>
                </button>
              {/if}
              <button
                class="icon-btn"
                onclick={() => copySql(finalSql)}
                type="button"
                title={sqlCopied ? 'Copied' : 'Copy SQL'}
                aria-label="Copy SQL"
              >
                {#if sqlCopied}
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                  >
                    <polyline points="20 6 9 17 4 12" />
                  </svg>
                {:else}
                  <svg
                    width="14"
                    height="14"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                  >
                    <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
                    <path
                      d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"
                    />
                  </svg>
                {/if}
              </button>
            </div>
            <SqlBlock code={finalSql} />
          </div>
        {:else if activeTab === 'table' && tableOutput}
          {#if showPageSize}
            <div class="table-meta">
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
            </div>
          {/if}
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
              filters: import('../../ipc/notebook').FilterSpec[];
              sortCol: string | null;
              sortDir: 'asc' | 'desc';
            }) => model.cellView.applyState(cell.id, s)}
            onNeedMore={() => model.cellView.fetchMore(cell.id)}
            onCountAll={() => model.cellView.countAll(cell.id)}
          />
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  /* Full-strength rule above: the prompt is input, everything here is not.
     Matches the SQL cell's output boundary so both cell kinds read alike. */
  .ai-output {
    display: flex;
    flex-direction: column;
    gap: 0;
    border-top: 1px solid var(--border);
  }

  /* ─── Output header: tab strip left, run status right ──────────── */
  .output-header {
    display: flex;
    align-items: stretch;
    min-height: 30px;
    padding: 0 10px;
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border-light, var(--border));
  }
  .header-spacer {
    flex: 1;
  }

  .activity {
    padding: 6px 12px;
    border-bottom: 1px solid var(--border-light, var(--border));
    background: var(--bg-subtle);
  }
  .activity-status {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px;
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-muted);
    font-size: var(--text-xs);
    text-align: left;
    cursor: pointer;
    align-self: center;
    transition: color 0.15s;
  }
  .activity-status:hover:not(.inert) {
    color: var(--text);
  }
  /* No log to open: still informative, just not a control. */
  .activity-status.inert {
    cursor: default;
  }
  .activity-status:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .activity-label {
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .chevron {
    flex-shrink: 0;
    color: var(--text-muted);
    transition: transform 0.15s ease;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .activity-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 320px;
    overflow-y: auto;
  }

  /* Tabs sit inside the header bar, so they carry no background of their own.
     The active marker is flush with the header's bottom rule. */
  .tabs-header {
    display: flex;
    gap: 0;
    margin-bottom: -1px;
  }
  .tab-btn {
    padding: 7px 12px;
    border: none;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    cursor: pointer;
    transition:
      color 0.15s,
      border-color 0.15s;
    white-space: nowrap;
  }
  .tab-btn:hover {
    color: var(--text);
  }
  .tab-btn.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
    font-weight: var(--weight-semibold);
  }
  .tab-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .tabs-body {
    min-height: 0;
  }
  .pad {
    padding: 12px 14px;
  }

  /* SQL panel: actions float over the top-right of the code block. */
  .sql-panel {
    position: relative;
    background: var(--bg-subtle);
  }
  .sql-actions {
    position: absolute;
    top: 6px;
    right: 8px;
    display: flex;
    align-items: center;
    gap: 2px;
    z-index: 1;
  }
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition:
      background 0.12s,
      color 0.12s,
      border-color 0.12s;
  }
  .icon-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
    border-color: var(--border);
  }
  .table-meta {
    display: flex;
    align-items: center;
    padding: 5px 12px;
    font-size: var(--text-xs);
    color: var(--text-muted);
    border-bottom: 1px solid var(--border-light, var(--border));
    background: var(--bg-subtle);
  }
  .page-size-label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .page-size-select {
    padding: 2px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-surface);
    color: var(--text);
    font-size: var(--text-xs);
    cursor: pointer;
  }
</style>
