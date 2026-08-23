<script>
  let {
    page,
    firstRowNumber,
    lastRowNumber,
    fetchedCount,
    totalCount = null,
    // Restores the legacy footer's three text branches (spec §3: unchanged
    // UX): while more rows may still stream in, the fetched count is
    // provisional and shown with a trailing +.
    isEnd = true,
    canGoNext,
    isFetchingMore = false,
    onNext,
    onPrev,
    /** Tighter footer padding inside embedded grids (moved from the parent's
        stylesheet — Svelte scoping would orphan it there). */
    embedded = false,
  } = $props();
</script>

<div class="pagination" class:embedded>
  <span class="page-info">
    Rows {firstRowNumber.toLocaleString()}–{lastRowNumber.toLocaleString()}
    {#if totalCount !== null}
      of {totalCount.toLocaleString()}
    {:else if isEnd}
      of {fetchedCount.toLocaleString()}
    {:else}
      of {fetchedCount.toLocaleString()}+
    {/if}
  </span>
  <div class="page-controls">
    <button
      class="page-btn"
      onclick={onPrev}
      disabled={page === 0}
      aria-label="Previous page"
      type="button">&lsaquo; Prev</button
    >
    <span class="page-number">Page {page + 1}</span>
    <button
      class="page-btn"
      onclick={onNext}
      disabled={!canGoNext || isFetchingMore}
      aria-label="Next page"
      type="button">Next &rsaquo;</button
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
  .page-number {
    font-size: var(--text-sm);
    color: var(--text-secondary);
    padding: 0 var(--space-2);
    font-weight: var(--weight-medium);
  }
  /* Embedded grids tighten the footer (carried from ResultsGrid's old
     `.results-grid.embedded .pagination` rule). */
  .pagination.embedded {
    padding: 4px 8px;
  }
</style>
