<script lang="ts">
  import type { HistoryEntry as HistoryEntryType } from '../../stores/history.svelte';

  let {
    entry,
    onToggleFavorite,
    onDelete,
    onRerun,
    onCopy,
  }: {
    entry: HistoryEntryType;
    onToggleFavorite?: (id: string) => void;
    onDelete?: (id: string) => void;
    onRerun?: (sql: string) => void;
    onCopy?: (sql: string) => void;
  } = $props();

  function truncateSql(sql: string, maxLen = 80): string {
    const line = sql.split('\n')[0];
    return line.length > maxLen ? line.slice(0, maxLen) + '…' : line;
  }

  function formatDuration(ms: number): string {
    if (ms < 1) return '<1ms';
    if (ms < 1000) return `${ms}ms`;
    return `${(ms / 1000).toFixed(2)}s`;
  }

  function formatRowCount(n: number | null): string {
    if (n === null) return '';
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M rows`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K rows`;
    return `${n} rows`;
  }

  function formatTimestamp(iso: string): string {
    const d = new Date(iso);
    const now = new Date();
    const diffMs = now.getTime() - d.getTime();
    const mins = Math.floor(diffMs / 60000);
    if (mins < 1) return 'Just now';
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    return d.toLocaleDateString();
  }
</script>

<div
  class="history-entry"
  class:error={entry.status === 'error'}
  class:favorite={entry.favorite}
>
  <div class="entry-main" onclick={() => onRerun?.(entry.sql)}>
    <div class="entry-sql">{truncateSql(entry.sql)}</div>
    <div class="entry-meta">
      <span class="meta-duration" class:slow={entry.durationMs > 1000}>
        {formatDuration(entry.durationMs)}
      </span>
      {#if entry.rowCount !== null}
        <span class="meta-sep">·</span>
        <span class="meta-rows">{formatRowCount(entry.rowCount)}</span>
      {/if}
      {#if entry.status === 'error'}
        <span class="meta-error">error</span>
      {/if}
      <span class="meta-sep">·</span>
      <span class="meta-time">{formatTimestamp(entry.executedAt)}</span>
      {#if entry.connectionName}
        <span class="meta-sep">·</span>
        <span class="meta-conn">{entry.connectionName}</span>
      {/if}
    </div>
  </div>
  <div class="entry-actions">
    <button
      class="action-btn"
      class:faved={entry.favorite}
      title={entry.favorite ? 'Unfavorite' : 'Favorite'}
      onclick={() => onToggleFavorite?.(entry.id)}
    >
      <svg
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill={entry.favorite ? 'currentColor' : 'none'}
        stroke="currentColor"
        stroke-width="2"
      >
        <polygon
          points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"
        />
      </svg>
    </button>
    <button
      class="action-btn"
      title="Copy SQL"
      onclick={() => onCopy?.(entry.sql)}
    >
      <svg
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
      >
        <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
        <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
      </svg>
    </button>
    <button
      class="action-btn danger"
      title="Delete"
      onclick={() => onDelete?.(entry.id)}
    >
      <svg
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
      >
        <polyline points="3 6 5 6 21 6" />
        <path
          d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"
        />
      </svg>
    </button>
  </div>
</div>

<style>
  .history-entry {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 12px;
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: background 0.1s;
  }
  .history-entry:hover {
    background: var(--bg-hover);
  }
  .history-entry.error {
    border-left: 3px solid var(--error);
  }
  .history-entry.favorite {
    background: color-mix(in srgb, var(--warning) 6%, transparent);
  }
  .entry-main {
    flex: 1;
    min-width: 0;
  }
  .entry-sql {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    margin-bottom: 2px;
  }
  .entry-meta {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-muted);
    flex-wrap: wrap;
  }
  .meta-duration {
    font-variant-numeric: tabular-nums;
  }
  .meta-duration.slow {
    color: var(--warning);
  }
  .meta-sep {
    color: var(--border);
  }
  .meta-error {
    color: var(--error);
    font-weight: 500;
  }
  .meta-rows {
    color: var(--text-secondary);
  }
  .meta-time {
  }
  .meta-conn {
    color: var(--text-secondary);
  }
  .entry-actions {
    display: flex;
    gap: 2px;
    opacity: 0;
    flex-shrink: 0;
    transition: opacity 0.1s;
  }
  .history-entry:hover .entry-actions,
  .history-entry:focus-within .entry-actions {
    opacity: 1;
  }
  .action-btn {
    width: 26px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
  }
  .action-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .action-btn.danger:hover {
    background: color-mix(in srgb, var(--error) 15%, transparent);
    color: var(--error);
  }
  .action-btn.faved {
    color: var(--warning);
  }
</style>
