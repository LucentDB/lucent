<script lang="ts">
  import IndexingStatusWidget from './IndexingStatusWidget.svelte';

  interface Props {
    connected?: boolean;
    connectionName?: string;
    databaseName?: string;
    driver?: string;
    readOnly?: boolean;
  }

  let {
    connected = false,
    connectionName = '',
    databaseName = '',
    driver = '',
    readOnly = false,
  }: Props = $props();
</script>

<footer class="status-bar" role="status" aria-label="Status Bar">
  <div class="status-left">
    {#if connected}
      <div class="status-item connection">
        <span class="status-dot online" title="Connected"></span>
        <span class="conn-name">{connectionName || 'Connected'}</span>
        {#if databaseName}
          <span class="sep">/</span>
          <span class="db-name">{databaseName}</span>
        {/if}
      </div>
      {#if driver}
        <div class="status-item driver-badge">
          {driver}
        </div>
      {/if}
      {#if readOnly}
        <div class="status-item readonly-badge">RO</div>
      {/if}
    {:else}
      <div class="status-item disconnected">
        <span class="status-dot offline"></span>
        <span>Not connected</span>
      </div>
    {/if}
  </div>

  <div class="status-middle"></div>

  <div class="status-right">
    {#if connected}
      <IndexingStatusWidget {connectionName} {databaseName} />
    {/if}
  </div>
</footer>

<style>
  .status-bar {
    height: 24px;
    background: var(--bg-elevated);
    border-top: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 var(--space-3);
    font-family: var(--font-sans);
    font-size: var(--text-xs);
    color: var(--text-secondary);
    user-select: none;
    z-index: 100;
    flex-shrink: 0;
  }

  .status-left,
  .status-right {
    display: flex;
    align-items: center;
    height: 100%;
    gap: var(--space-2);
  }

  .status-middle {
    flex: 1;
  }

  .status-item {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 100%;
    font-size: 11px;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .status-dot.online {
    background: var(--success);
    box-shadow: 0 0 4px var(--success);
  }

  .status-dot.offline {
    background: var(--text-muted);
  }

  .conn-name {
    font-weight: var(--weight-medium);
    color: var(--text);
  }

  .sep {
    color: var(--text-muted);
    margin: 0 1px;
  }

  .db-name {
    color: var(--text-secondary);
  }

  .driver-badge,
  .readonly-badge {
    padding: 0 4px;
    height: 16px;
    line-height: 16px;
    border-radius: var(--radius-sm);
    background: var(--bg-subtle);
    font-size: 10px;
    font-weight: var(--weight-medium);
    color: var(--text-muted);
    text-transform: uppercase;
  }

  .readonly-badge {
    background: var(--accent-soft);
    color: var(--accent);
  }
</style>
