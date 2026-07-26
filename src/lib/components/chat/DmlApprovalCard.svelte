<script lang="ts">
  let {
    dml,
    onRun,
    onCancel,
  }: {
    dml: {
      sql: string;
      description: string;
      estimatedRowsAffected: number | null;
    };
    onRun?: () => void;
    onCancel?: () => void;
  } = $props();
</script>

<div class="dml-card">
  <div class="dml-hdr">
    <span class="dml-icon">🔒</span>
    <span class="dml-title">Review DML Statement</span>
  </div>
  <p class="dml-desc">{dml.description}</p>
  <pre class="dml-sql"><code>{dml.sql}</code></pre>
  {#if dml.estimatedRowsAffected !== null}
    <div class="dml-blast">
      <span class="blast-icon">⚠️</span>
      <span
        >Estimated rows affected: <strong
          >{dml.estimatedRowsAffected.toLocaleString()}</strong
        ></span
      >
    </div>
  {/if}
  <div class="dml-actions">
    <button class="btn-cancel" onclick={onCancel}>Cancel</button>
    <button class="btn-run" onclick={onRun}>Execute</button>
  </div>
</div>

<style>
  .dml-card {
    border: 1px solid var(--warning);
    border-radius: var(--radius-md);
    padding: 14px;
    margin: 8px 0;
    background: var(--bg-surface);
  }
  .dml-hdr {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 8px;
  }
  .dml-icon {
    font-size: var(--text-lg);
  }
  .dml-title {
    font-weight: var(--weight-semibold);
    font-size: var(--text-md);
  }
  .dml-desc {
    font-size: var(--text-sm);
    color: var(--text-secondary);
    margin-bottom: 8px;
  }
  .dml-sql {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    background: var(--bg-subtle);
    padding: 10px 12px;
    border-radius: var(--radius-sm);
    overflow-x: auto;
    margin-bottom: 8px;
    line-height: 1.5;
  }
  .dml-sql code {
    font-family: inherit;
  }
  .dml-blast {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
    color: var(--warning);
    margin-bottom: 10px;
    padding: 6px 10px;
    background: color-mix(in srgb, var(--warning) 10%, transparent);
    border-radius: var(--radius-sm);
  }
  .blast-icon {
    font-size: 14px;
  }
  .dml-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
  .btn-run {
    background: var(--danger);
    color: #fff;
    border: none;
    padding: 7px 20px;
    border-radius: var(--radius-md);
    cursor: pointer;
    font-weight: var(--weight-semibold);
    font-size: var(--text-sm);
    transition: opacity var(--transition-fast);
  }
  .btn-run:hover {
    opacity: 0.9;
  }
  .btn-cancel {
    background: transparent;
    border: 1px solid var(--border);
    padding: 7px 20px;
    border-radius: var(--radius-md);
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-secondary);
    transition: all var(--transition-fast);
  }
  .btn-cancel:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
</style>
