<script lang="ts">
  let {
    qr,
  }: { qr: { sql: string; rowCount: number; executionTimeMs: number } } =
    $props();
</script>

<div class="qr-card">
  <div class="qr-sql"><code>{qr.sql}</code></div>
  <div class="qr-meta">
    <span class="qr-badge">{qr.rowCount} rows</span>
    <span class="qr-time">{qr.executionTimeMs}ms</span>
  </div>
</div>

<style>
  .qr-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
    margin: 8px 0;
  }
  .qr-sql {
    padding: 8px 12px;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }
  .qr-sql code {
    font-family: inherit;
    color: var(--accent);
  }
  .qr-meta {
    display: flex;
    gap: 8px;
    padding: 4px 12px;
    background: var(--bg-subtle);
    align-items: center;
  }
  .qr-badge {
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    color: var(--success);
  }
  .qr-time {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
</style>
