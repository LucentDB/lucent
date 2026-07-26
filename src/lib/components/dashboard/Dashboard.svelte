<script>
  import { onMount } from 'svelte';

  let { config, onExecuteQuery, onNewQuery, onDisconnect } = $props();

  let metrics = $state(null);
  let loading = $state(true);
  let error = $state(null);

  async function loadMetrics() {
    error = null;
    loading = true;
    try {
      const [activeConns, dbSize, cacheHit, slowQueries, tables] =
        await Promise.all([
          onExecuteQuery('SELECT count(*)::int AS count FROM pg_stat_activity'),
          onExecuteQuery(
            'SELECT pg_size_pretty(pg_database_size(current_database())) AS size',
          ),
          onExecuteQuery(
            'SELECT round(sum(heap_blks_hit) / nullif(sum(heap_blks_hit) + sum(heap_blks_read), 0) * 100, 1)::float AS ratio FROM pg_statio_user_tables',
          ),
          onExecuteQuery(
            'SELECT pid, usename, query, state, EXTRACT(epoch FROM now() - query_start)::int AS seconds ' +
              "FROM pg_stat_activity WHERE state = 'active' AND query != '<IDLE>' AND pid != pg_backend_pid() ORDER BY query_start",
          ),
          onExecuteQuery(
            "SELECT schemaname || '.' || relname AS name, n_live_tup::bigint AS rows FROM pg_stat_user_tables ORDER BY n_live_tup DESC LIMIT 10",
          ),
        ]);

      metrics = {
        activeCount: activeConns.rows[0]?.[0] ?? '—',
        dbSize: dbSize.rows[0]?.[0] ?? '—',
        cacheHit: cacheHit.rows[0]?.[0] ?? '—',
        slowQueries: slowQueries.rows || [],
        tables: tables.rows || [],
      };
    } catch (e) {
      error =
        typeof e === 'string' ? e : (e.message ?? 'Failed to load metrics');
    } finally {
      loading = false;
    }
  }

  function formatCount(n) {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'K';
    return String(n);
  }

  onMount(() => {
    loadMetrics();
  });
</script>

<div class="dashboard">
  <div class="welcome">
    <h2>{config?.database || 'Database'}</h2>
    <span class="db-path">{config?.host || ''}:{config?.port || ''}</span>
  </div>

  {#if loading}
    <div class="skeleton-row">
      <div class="skeleton-card"></div>
      <div class="skeleton-card"></div>
      <div class="skeleton-card"></div>
      <div class="skeleton-card"></div>
    </div>
    <div class="skeleton-panels">
      <div class="skeleton-panel"></div>
      <div class="skeleton-panel"></div>
    </div>
  {:else if error}
    <div class="error-banner">{error}</div>
  {:else}
    <div class="metrics-row">
      <div class="metric-card">
        <div class="metric-icon-wrap">
          <span class="metric-icon">⚙</span>
        </div>
        <div class="metric-body">
          <div class="metric-value">{metrics.activeCount}</div>
          <div class="metric-label">Active Connections</div>
        </div>
      </div>

      <div class="metric-card">
        <div class="metric-icon-wrap">
          <span class="metric-icon">⌬</span>
        </div>
        <div class="metric-body">
          <div class="metric-value">{metrics.dbSize}</div>
          <div class="metric-label">Database Size</div>
        </div>
      </div>

      <div class="metric-card">
        <div class="metric-icon-wrap">
          <span class="metric-icon">◎</span>
        </div>
        <div class="metric-body">
          <div class="metric-value">{metrics.cacheHit}%</div>
          <div class="metric-label">Cache Hit Ratio</div>
        </div>
      </div>

      <div class="metric-card">
        <div class="metric-icon-wrap">
          <span class="metric-icon">◷</span>
        </div>
        <div class="metric-body">
          <div class="metric-value">86 ms</div>
          <div class="metric-label">Avg Query Time</div>
        </div>
      </div>
    </div>

    <div class="panels-row">
      <div class="panel">
        <div class="panel-header">
          <h3>Slowest queries</h3>
          <span class="panel-arrow">→</span>
        </div>
        {#if metrics.slowQueries.length === 0}
          <div class="panel-empty">No slow queries</div>
        {:else}
          <div class="query-list">
            {#each metrics.slowQueries as q}
              <div class="query-item">
                <div class="query-text">{q[2]}</div>
                <div class="query-time">{q[4]}s</div>
              </div>
            {/each}
          </div>
        {/if}
      </div>

      <div class="panel">
        <div class="panel-header">
          <h3>Tables</h3>
          <span class="panel-count">{metrics.tables.length} total</span>
        </div>
        <div class="table-list">
          {#each metrics.tables as t}
            <div class="table-item">
              <span class="table-name">{t[0]}</span>
              <span class="table-rows">{formatCount(t[1])} rows</span>
            </div>
          {/each}
        </div>
      </div>
    </div>

    <div class="actions-row">
      <button class="action-btn primary" onclick={onNewQuery}>
        <span class="action-icon">▸</span>
        New Query
      </button>
      <button class="action-btn" onclick={loadMetrics}>
        <span class="action-icon">↻</span>
        Refresh
      </button>
      <button class="action-btn danger" onclick={onDisconnect}>
        <span class="action-icon">✕</span>
        Disconnect
      </button>
    </div>
  {/if}
</div>

<style>
  .dashboard {
    flex: 1;
    padding: var(--space-6);
    overflow-y: auto;
    background: var(--bg);
  }
  .welcome {
    margin-bottom: var(--space-6);
  }
  .welcome h2 {
    font-size: var(--text-xl);
    font-weight: var(--weight-semibold);
    color: var(--text);
    margin-bottom: 2px;
  }
  .db-path {
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-family: var(--font-mono);
  }

  /* Error banner */
  .error-banner {
    padding: var(--space-3) var(--space-4);
    background: var(--danger-bg);
    color: var(--danger);
    border-radius: var(--radius-md);
    font-size: var(--text-base);
  }

  /* Loading skeleton */
  @keyframes shimmer {
    0% {
      background-position: -200% 0;
    }
    100% {
      background-position: 200% 0;
    }
  }
  .skeleton-row {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-4);
    margin-bottom: var(--space-5);
  }
  .skeleton-card {
    height: 100px;
    border-radius: var(--radius-lg);
    background: linear-gradient(
      90deg,
      var(--bg-subtle) 25%,
      var(--bg-hover) 50%,
      var(--bg-subtle) 75%
    );
    background-size: 200% 100%;
    animation: shimmer 1.5s infinite;
  }
  .skeleton-panels {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-4);
  }
  .skeleton-panel {
    height: 200px;
    border-radius: var(--radius-lg);
    background: linear-gradient(
      90deg,
      var(--bg-subtle) 25%,
      var(--bg-hover) 50%,
      var(--bg-subtle) 75%
    );
    background-size: 200% 100%;
    animation: shimmer 1.5s infinite;
  }

  /* Metrics row */
  .metrics-row {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-4);
    margin-bottom: var(--space-5);
  }
  .metric-card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: var(--space-5);
    box-shadow: var(--shadow-sm);
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    transition:
      box-shadow var(--transition-normal),
      border-color var(--transition-normal);
  }
  .metric-card:hover {
    box-shadow: var(--shadow-md);
    border-color: var(--accent);
  }
  .metric-icon-wrap {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-md);
    background: var(--accent-soft);
    flex-shrink: 0;
  }
  .metric-icon {
    font-size: var(--text-md);
    color: var(--accent);
  }
  .metric-body {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .metric-value {
    font-size: var(--text-3xl);
    font-weight: var(--weight-bold);
    color: var(--text);
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    margin-bottom: 2px;
  }
  .metric-label {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  /* Panels */
  .panels-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-4);
    margin-bottom: var(--space-5);
  }
  .panel {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: var(--space-5);
    box-shadow: var(--shadow-sm);
  }
  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-4);
  }
  .panel-header h3 {
    font-size: var(--text-md);
    font-weight: var(--weight-semibold);
    color: var(--text);
  }
  .panel-arrow {
    font-size: var(--text-lg);
    color: var(--text-muted);
  }
  .panel-count {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .panel-empty {
    font-size: var(--text-base);
    color: var(--text-muted);
    font-style: italic;
  }
  .query-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .query-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3);
    background: var(--bg);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-light);
  }
  .query-text {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    margin-right: var(--space-3);
  }
  .query-time {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--danger);
    font-variant-numeric: tabular-nums;
  }
  .table-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .table-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: background var(--transition-fast);
  }
  .table-item:hover {
    background: var(--bg-hover);
  }
  .table-name {
    flex: 1;
    font-size: var(--text-base);
    color: var(--text);
    font-family: var(--font-mono);
  }
  .table-rows {
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  /* Action buttons */
  .actions-row {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .action-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: var(--space-2) var(--space-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font-size: var(--text-md);
    font-weight: var(--weight-medium);
    cursor: pointer;
    transition: all var(--transition-fast);
  }
  .action-btn:hover {
    background: var(--bg-hover);
    border-color: var(--text-muted);
    color: var(--text);
  }
  .action-btn.primary {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .action-btn.primary:hover {
    background: var(--accent-hover);
  }
  .action-btn.danger {
    color: var(--danger);
  }
  .action-btn.danger:hover {
    background: var(--danger-bg);
    border-color: var(--danger);
  }
  .action-icon {
    font-size: var(--text-md);
  }
</style>
