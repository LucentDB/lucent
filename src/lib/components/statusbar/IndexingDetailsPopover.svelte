<script lang="ts">
  import { onMount } from 'svelte';
  import {
    indexing,
    stageLabel,
    type SchemaIndexingStatus,
  } from '../../stores/indexing.svelte';

  interface Props {
    connectionName?: string;
    databaseName?: string;
    onClose?: () => void;
  }

  let {
    connectionName = 'Active Connection',
    databaseName = '',
    onClose,
  }: Props = $props();

  let backendStatus = $state<SchemaIndexingStatus | null>(null);
  let isRefreshing = $state(false);

  async function loadStatus() {
    backendStatus = await indexing.fetchStatus();
  }

  async function handleSyncDelta() {
    isRefreshing = true;
    try {
      await indexing.syncSchemaIndexing(false);
    } finally {
      setTimeout(() => {
        isRefreshing = false;
        loadStatus();
      }, 500);
    }
  }

  async function handleRebuild() {
    isRefreshing = true;
    try {
      await indexing.syncSchemaIndexing(true);
    } finally {
      setTimeout(() => {
        isRefreshing = false;
        loadStatus();
      }, 500);
    }
  }

  onMount(() => {
    loadStatus();
  });
</script>

<div
  class="indexing-popover"
  role="dialog"
  aria-label="Schema Indexing Details"
>
  <header class="popover-header">
    <div class="header-title">
      <div class="icon-wrap">
        <svg
          class="db-icon"
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <ellipse cx="12" cy="5" rx="9" ry="3"></ellipse>
          <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3"></path>
          <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5"></path>
        </svg>
      </div>
      <div>
        <div class="title-text">Schema Indexing</div>
        <div class="subtitle-text">
          <span class="badge conn">{connectionName}</span>
          {#if databaseName}
            <span class="badge db">{databaseName}</span>
          {/if}
        </div>
      </div>
    </div>
    <button class="close-btn" onclick={onClose} aria-label="Close">
      <svg
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
      >
        <line x1="18" y1="6" x2="6" y2="18"></line>
        <line x1="6" y1="6" x2="18" y2="18"></line>
      </svg>
    </button>
  </header>

  <div class="popover-body">
    <!-- Status summary banner -->
    {#if !indexing.isComplete}
      <div class="status-card active">
        <div class="status-card-header">
          <span class="spinner" aria-hidden="true"></span>
          <span class="status-headline">{stageLabel(indexing.stage)}</span>
          <span class="percent-tag">{indexing.percent}%</span>
        </div>
        <div class="progress-track">
          <div class="progress-fill" style="width: {indexing.percent}%"></div>
        </div>
        {#if indexing.text}
          <div class="status-detail">{indexing.text}</div>
        {/if}
      </div>
    {:else}
      <div class="status-card ready">
        <div class="status-card-header">
          <svg
            class="check-icon"
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.5"
          >
            <polyline points="20 6 9 17 4 12"></polyline>
          </svg>
          <span class="status-headline">Schema index is up to date</span>
          <span class="tier-tag"
            >{backendStatus?.tier === 'fully_enriched'
              ? 'Semantic Tier-2'
              : 'Fast Tier-1'}</span
          >
        </div>
        <div class="status-subtext">
          {#if backendStatus?.tableCount}
            {backendStatus.tableCount} relations indexed ({backendStatus.columnCount}
            columns{#if backendStatus.viewCount > 0}, {backendStatus.viewCount} views{/if})
          {:else}
            Delta caching active & verified
          {/if}
        </div>
      </div>
    {/if}

    <!-- Delta statistics section -->
    <div class="section">
      <div class="section-label">Delta Scan Results</div>
      <div class="delta-grid">
        <div class="delta-chip added" title="New tables/views discovered">
          <span class="delta-num">+{indexing.delta.added}</span>
          <span class="delta-name">added</span>
        </div>
        <div
          class="delta-chip modified"
          title="Tables with altered columns/constraints"
        >
          <span class="delta-num">~{indexing.delta.modified}</span>
          <span class="delta-name">modified</span>
        </div>
        <div class="delta-chip deleted" title="Dropped tables/views removed">
          <span class="delta-num">-{indexing.delta.deleted}</span>
          <span class="delta-name">deleted</span>
        </div>
        <div
          class="delta-chip unchanged"
          title="Tables untouched (re-used cache)"
        >
          <span class="delta-num">={indexing.delta.unchanged}</span>
          <span class="delta-name">unchanged</span>
        </div>
      </div>
    </div>

    <!-- Pipeline steps checklist -->
    <div class="section">
      <div class="section-label">Pipeline Execution</div>
      <div class="pipeline-steps">
        <div class="step-row done">
          <div class="step-dot">✓</div>
          <div class="step-info">
            <div class="step-name">Catalog Discovery</div>
            <div class="step-desc">
              Harvested tables, views, and foreign keys
            </div>
          </div>
        </div>
        <div class="step-row {indexing.stage === 'delta' ? 'running' : 'done'}">
          <div class="step-dot">{indexing.stage === 'delta' ? '•' : '✓'}</div>
          <div class="step-info">
            <div class="step-name">Differential Hashing</div>
            <div class="step-desc">
              Deterministic table signature comparison
            </div>
          </div>
        </div>
        <div
          class="step-row {indexing.stage === 'sampling'
            ? 'running'
            : indexing.isComplete
              ? 'done'
              : 'pending'}"
        >
          <div class="step-dot">
            {indexing.stage === 'sampling'
              ? '•'
              : indexing.isComplete
                ? '✓'
                : '○'}
          </div>
          <div class="step-info">
            <div class="step-name">Selective Value Sampling</div>
            <div class="step-desc">
              Sampled values strictly for changed relations
            </div>
          </div>
        </div>
        <div
          class="step-row {indexing.stage === 'embedding'
            ? 'running'
            : indexing.isComplete
              ? 'done'
              : 'pending'}"
        >
          <div class="step-dot">
            {indexing.stage === 'embedding'
              ? '•'
              : indexing.isComplete
                ? '✓'
                : '○'}
          </div>
          <div class="step-info">
            <div class="step-name">Vector Embeddings</div>
            <div class="step-desc">
              Single-flight ONNX with BLAKE3 cache lookup
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Performance Stats -->
    <div class="section stats-section">
      <div class="section-label">Performance & Cache</div>
      <div class="stats-row">
        <div class="stat-item">
          <span class="stat-label">Cache Hits</span>
          <span class="stat-val">{indexing.stats.cacheHits}</span>
        </div>
        <div class="stat-item">
          <span class="stat-label">Embeds Computed</span>
          <span class="stat-val">{indexing.stats.embeddingsComputed}</span>
        </div>
        <div class="stat-item">
          <span class="stat-label">Elapsed Time</span>
          <span class="stat-val">{indexing.stats.elapsedMs}ms</span>
        </div>
      </div>
    </div>
  </div>

  <footer class="popover-footer">
    <button
      class="action-btn secondary"
      onclick={handleRebuild}
      disabled={!indexing.isComplete || isRefreshing}
      title="Drop local vector cache and rebuild all embeddings"
    >
      Rebuild Index
    </button>
    <button
      class="action-btn primary"
      onclick={handleSyncDelta}
      disabled={!indexing.isComplete || isRefreshing}
      title="Check catalog and process only changed relations"
    >
      {#if isRefreshing}
        <span class="spinner-sm"></span> Syncing…
      {:else}
        Sync Delta
      {/if}
    </button>
  </footer>
</div>

<style>
  .indexing-popover {
    width: 360px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-float);
    color: var(--text);
    font-family: var(--font-sans);
    font-size: var(--text-sm);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    z-index: 1000;
  }

  .popover-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border-light);
    background: var(--bg-elevated);
  }

  .header-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .icon-wrap {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-md);
    background: var(--accent-soft);
    color: var(--accent);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .title-text {
    font-weight: var(--weight-semibold);
    font-size: var(--text-base);
    line-height: 1.2;
  }

  .subtitle-text {
    display: flex;
    gap: 4px;
    margin-top: 2px;
  }

  .badge {
    font-size: 10px;
    font-weight: var(--weight-medium);
    padding: 1px 5px;
    border-radius: var(--radius-sm);
    background: var(--bg-subtle);
    color: var(--text-secondary);
  }

  .badge.db {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 4px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .close-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }

  .popover-body {
    padding: var(--space-3) var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-height: 420px;
    overflow-y: auto;
  }

  .status-card {
    padding: var(--space-3);
    border-radius: var(--radius-md);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .status-card.active {
    background: var(--accent-soft);
    border: 1px solid var(--accent-muted);
  }

  .status-card.ready {
    background: var(--success-bg);
    border: 1px solid oklch(0.58 0.16 145 / 0.3);
  }

  .status-card-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .status-headline {
    font-weight: var(--weight-medium);
    font-size: var(--text-sm);
    flex: 1;
  }

  .percent-tag,
  .tier-tag {
    font-size: 11px;
    font-weight: var(--weight-semibold);
    padding: 1px 6px;
    border-radius: var(--radius-full);
    background: var(--bg-surface);
  }

  .percent-tag {
    color: var(--accent);
  }

  .tier-tag {
    color: var(--success);
  }

  .check-icon {
    color: var(--success);
  }

  .status-subtext,
  .status-detail {
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }

  .progress-track {
    height: 4px;
    background: var(--bg-surface);
    border-radius: 2px;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s ease-out;
  }

  .spinner {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid var(--accent);
    border-top-color: transparent;
    animation: spin 0.8s linear infinite;
  }

  .spinner-sm {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    animation: spin 0.8s linear infinite;
    margin-right: 4px;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .section-label {
    font-size: 11px;
    font-weight: var(--weight-semibold);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }

  .delta-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-2);
  }

  .delta-chip {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--bg-subtle);
  }

  .delta-num {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-weight: var(--weight-bold);
  }

  .delta-name {
    font-size: 10px;
    color: var(--text-muted);
  }

  .delta-chip.added .delta-num {
    color: var(--success);
  }
  .delta-chip.modified .delta-num {
    color: var(--warning);
  }
  .delta-chip.deleted .delta-num {
    color: var(--danger);
  }
  .delta-chip.unchanged .delta-num {
    color: var(--text-secondary);
  }

  .pipeline-steps {
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: var(--bg-subtle);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
  }

  .step-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
  }

  .step-dot {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 10px;
    font-weight: bold;
    flex-shrink: 0;
    margin-top: 1px;
  }

  .step-row.done .step-dot {
    background: var(--success-bg);
    color: var(--success);
  }

  .step-row.running .step-dot {
    background: var(--accent-soft);
    color: var(--accent);
    animation: pulse 1s infinite alternate;
  }

  .step-row.pending .step-dot {
    background: var(--bg-hover);
    color: var(--text-muted);
  }

  @keyframes pulse {
    from {
      opacity: 0.6;
    }
    to {
      opacity: 1;
    }
  }

  .step-info {
    display: flex;
    flex-direction: column;
  }

  .step-name {
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
  }

  .step-desc {
    font-size: 10px;
    color: var(--text-muted);
  }

  .stats-row {
    display: flex;
    justify-content: space-between;
    background: var(--bg-subtle);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
  }

  .stat-item {
    display: flex;
    flex-direction: column;
  }

  .stat-label {
    font-size: 10px;
    color: var(--text-muted);
  }

  .stat-val {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
  }

  .popover-footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    background: var(--bg-elevated);
    border-top: 1px solid var(--border-light);
  }

  .action-btn {
    padding: 4px 10px;
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.1s ease;
  }

  .action-btn.secondary {
    background: var(--bg-surface);
    border-color: var(--border);
    color: var(--text-secondary);
  }

  .action-btn.secondary:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text);
  }

  .action-btn.primary {
    background: var(--accent);
    color: var(--accent-foreground);
  }

  .action-btn.primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .action-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
