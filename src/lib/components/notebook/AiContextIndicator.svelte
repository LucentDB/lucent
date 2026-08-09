<script lang="ts">
  let {
    priorCellCount,
    isBudgetCapped,
  }: {
    priorCellCount: number;
    isBudgetCapped: boolean;
  } = $props();
</script>

<div class="ai-context-badge" class:capped={isBudgetCapped}>
  <span class="ai-context-icon">🧠</span>
  <span class="ai-context-label">
    {priorCellCount} prior cell{priorCellCount === 1 ? '' : 's'}
    {#if isBudgetCapped}
      <span class="ai-context-capped">(budget capped)</span>
    {/if}
  </span>
</div>

<style>
  .ai-context-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }
  .ai-context-badge.capped {
    border-color: var(--warning);
    background: color-mix(in srgb, var(--warning) 10%, transparent);
  }
  .ai-context-icon {
    font-size: var(--text-xs);
    line-height: 1;
  }
  .ai-context-label {
    line-height: 1;
  }
  .ai-context-capped {
    color: var(--warning);
    font-weight: var(--weight-medium);
  }
</style>
