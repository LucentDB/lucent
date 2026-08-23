<script>
  let {
    page,
    firstRowNumber,
    lastRowNumber,
    fetchedCount,
    totalCount = null,
    canGoNext,
    isFetchingMore = false,
    onNext,
    onPrev,
  } = $props();
</script>

<div class="pagination">
  <span class="page-info">
    Rows {firstRowNumber.toLocaleString()}–{lastRowNumber.toLocaleString()}
    {#if totalCount !== null}
      of {totalCount.toLocaleString()}
    {:else}
      of {fetchedCount.toLocaleString()} fetched
    {/if}
  </span>
  <div class="page-controls">
    <button
      class="page-btn"
      onclick={onPrev}
      disabled={page === 0}
      aria-label="Previous page"
      type="button">Previous</button
    >
    <button
      class="page-btn"
      onclick={onNext}
      disabled={!canGoNext || isFetchingMore}
      aria-label="Next page"
      type="button">Next</button
    >
  </div>
</div>

<style>
  .pagination {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-4);
    border-top: 1px solid var(--border);
    background: var(--bg-surface);
  }
  .page-info {
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }
  .page-controls {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .page-btn {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    color: var(--text);
    padding: 4px 12px;
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
    cursor: pointer;
    transition: all var(--transition-fast);
  }
  .page-btn:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .page-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>
