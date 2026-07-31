<script>
  import { untrack } from 'svelte';
  import FilterBar from './FilterBar.svelte';
  import GridMenu from './GridMenu.svelte';
  import {
    normalize,
    applyable,
    addFilter,
    filterByCellValue,
    isComplete,
  } from './filters.js';

  let {
    columns = [],
    rows = [],
    fetchedCount = 0,
    totalCount = null,
    isEnd = false,
    duration = 0,
    error = null,
    tabId = null,
    initFilters = [],
    initSortCol = null,
    initSortDir = 'asc',
    onStateChange = null,
    onNeedMore = null,
    onCountAll = null,
    compact = false,
    loading = false,
    onDescribeFilters = null,
  } = $props();

  const PAGE_SIZE = 200;

  let checkedRows = $state(new Set());
  let barOpen = $state(false);
  let pickerOpen = $state(false);
  let columnWidths = $state({});
  let resizing = $state(null);
  let filters = $state(normalize(initFilters));
  let sortCol = $state(initSortCol);
  let sortDir = $state(initSortDir);
  let page = $state(0);
  let tableWrapperEl = $state(null);

  let columnMenu = $state(null);
  let cellMenu = $state(null);

  // Reset state when tab changes — Svelte reuses the same component instance
  // for the same {#if} branch, so internal $state persists across tab switches.
  // Preserve columnWidths across tab switches so manual column resizing survives
  // page navigation, sort changes, and tab switching.
  $effect(() => {
    // Only re-run on tab switch. Reading initFilters/initSortCol/initSortDir
    // directly would make them dependencies, so every parent re-emit (e.g. a
    // filter refetch) would re-run this and collapse the filter row mid-type.
    void tabId;
    untrack(() => {
      filters = normalize(initFilters);
      sortCol = initSortCol;
      sortDir = initSortDir;
      checkedRows = new Set();
      barOpen = false;
      pickerOpen = false;
      page = 0;
      isFetchingMore = false;
      if (tableWrapperEl) {
        tableWrapperEl.scrollTop = 0;
        tableWrapperEl.scrollLeft = 0;
      }
    });
  });

  // Reset to page 0 when a fresh fetch arrives (fetchedCount resets to ≤ 200
  // after sort/filter change or re-execute in the same tab).
  $effect(() => {
    void fetchedCount;
    if (fetchedCount > 0 && fetchedCount <= PAGE_SIZE) {
      page = 0;
      if (tableWrapperEl) {
        tableWrapperEl.scrollTop = 0;
      }
    }
  });

  // Clamp page so it never points past the fetched data (handles race conditions
  // where goNext advances before fetchedCount catches up, or tab state resets).
  let maxPage = $derived(Math.max(0, Math.ceil(fetchedCount / PAGE_SIZE) - 1));
  $effect(() => {
    void maxPage;
    if (page > maxPage) {
      page = maxPage;
    }
  });

  // Current page's rows
  let pageRows = $derived(rows.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE));

  // "Next" is enabled unless we've reached the end AND the next page isn't cached.
  let canGoNext = $derived(!isEnd || (page + 1) * PAGE_SIZE < fetchedCount);

  // Local guard to prevent racing ahead of in-flight fetches when clicking Next rapidly
  let isFetchingMore = $state(false);

  async function goNext() {
    if (isFetchingMore) return;
    const nextPage = page + 1;
    const nextOffset = nextPage * PAGE_SIZE;
    if (nextOffset >= fetchedCount) {
      // Need to fetch — onNeedMore fetches the next chunk from offset=fetchedCount
      isFetchingMore = true;
      try {
        await onNeedMore?.();
      } finally {
        isFetchingMore = false;
      }
    }
    // Only advance if the next page actually has data now (either it was cached,
    // or the fetch filled it). Importantly: isEnd means "no more rows" — it should
    // NEVER let us advance past the last valid page.
    if (fetchedCount > nextPage * PAGE_SIZE) {
      page = nextPage;
    }
  }

  function goPrev() {
    page = Math.max(0, page - 1);
  }

  function emitChange() {
    // Reset to the first page directly rather than inferring it from a
    // fetchedCount that no longer drops to 0 on refetch.
    page = 0;
    checkedRows = new Set();
    onStateChange?.({ filters, sortCol, sortDir });
  }

  // Filter bar
  let barVisible = $derived(barOpen || filters.length > 0);
  let hasActiveFilters = $derived(applyable(filters).length > 0);

  function toggleBar() {
    // Never hide the bar while a filter exists — Clear all is the way out.
    if (filters.length > 0) {
      barOpen = true;
      return;
    }
    barOpen = !barOpen;
    if (!barOpen) pickerOpen = false;
  }

  function handleFiltersChange(next, { commit }) {
    filters = next;
    if (commit) emitChange();
  }

  function clearFilters() {
    filters = [];
    pickerOpen = false;
    emitChange();
  }

  // Column header menu
  const COLUMN_MENU_ITEMS = [
    { id: 'asc', label: 'Sort ascending' },
    { id: 'desc', label: 'Sort descending' },
    { id: 'clear-sort', label: 'Clear sort' },
    { separator: true },
    { id: 'filter', label: 'Filter by this column' },
    { id: 'copy', label: 'Copy column name' },
  ];

  function openColumnMenu(e, col) {
    e.stopPropagation();
    const r = e.currentTarget.getBoundingClientRect();
    columnMenu = {
      column: col.name,
      typeName: col.type_name,
      x: r.left,
      y: r.bottom + 4,
    };
  }

  function handleColumnMenuSelect(id) {
    const { column, typeName } = columnMenu;
    if (id === 'asc' || id === 'desc') {
      sortCol = column;
      sortDir = id;
      emitChange();
      return;
    }
    if (id === 'clear-sort') {
      sortCol = null;
      sortDir = 'asc';
      emitChange();
      return;
    }
    if (id === 'filter') {
      barOpen = true;
      const next = addFilter(filters, column, typeName);
      const added = next[next.length - 1];
      filters = next;
      if (isComplete(added)) emitChange();
      return;
    }
    if (id === 'copy') {
      navigator.clipboard?.writeText(column).catch(() => {});
    }
  }

  // Cell context menu
  let cellMenuItems = $derived(
    cellMenu === null
      ? []
      : cellMenu.value === null || cellMenu.value === undefined
        ? [
            { id: 'filter', label: 'Filter by is null' },
            { id: 'filter-out', label: 'Filter by is not null' },
            { separator: true },
            { id: 'copy', label: 'Copy value' },
          ]
        : [
            { id: 'filter', label: 'Filter by this value' },
            { id: 'filter-out', label: 'Filter out this value' },
            { separator: true },
            { id: 'copy', label: 'Copy value' },
          ],
  );

  function openCellMenu(e, colIndex, value) {
    const col = columns[colIndex];
    if (!col) return;
    e.preventDefault();
    cellMenu = {
      column: col.name,
      typeName: col.type_name,
      value,
      x: e.clientX,
      y: e.clientY,
    };
  }

  function handleCellMenuSelect(id) {
    const { column, typeName, value } = cellMenu;
    if (id === 'copy') {
      navigator.clipboard?.writeText(formatCell(value)).catch(() => {});
      return;
    }
    barOpen = true;
    filters = filterByCellValue(filters, column, typeName, value, {
      negate: id === 'filter-out',
    });
    emitChange();
  }

  function handleWindowKeydown(e) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'f') {
      e.preventDefault();
      barOpen = true;
      pickerOpen = true;
    }
  }

  function toggleSort(colIndex) {
    const column = columns[colIndex]?.name;
    if (!column) return;
    if (sortCol === column) {
      sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    } else {
      sortCol = column;
      sortDir = 'asc';
    }
    checkedRows = new Set();
    emitChange();
  }

  function sortIndicator(colIndex) {
    if (sortCol !== columns[colIndex]?.name) return '';
    return sortDir === 'asc' ? ' ▴' : ' ▾';
  }

  // --- Column resize ---
  let resizeGuide = $state(null);

  function getColWidth(i) {
    return columnWidths[i] || 150;
  }

  // Keep the table width exactly matching the sum of column widths so
  // table-layout: fixed has NO extra space to redistribute. Without this,
  // resizing one column shifts adjacent columns' rendered widths.
  $effect(() => {
    void columns;
    void columnWidths;
    if (!tableWrapperEl || columns.length === 0) return;
    const total = columns.reduce((sum, _, i) => sum + getColWidth(i), 0);
    const tbl = tableWrapperEl.querySelector('table');
    if (tbl) {
      tbl.style.width = total + 'px';
    }
  });

  function startResize(e, i) {
    e.preventDefault();
    e.stopPropagation();
    const th = e.target.closest('th');
    if (!th) return;
    const thRect = th.getBoundingClientRect();
    const wrapperRect = tableWrapperEl.getBoundingClientRect();
    // The resize guide is position: absolute inside the wrapper, so its
    // left is in wrapper coordinates (= viewport, since the wrapper is
    // position: relative and doesn't scroll). getBoundingClientRect gives
    // viewport coordinates, so thRect.right - wrapperRect.left is already
    // correct — do NOT add scrollLeft (that would double-count scroll).
    resizing = {
      colIndex: i,
      startX: e.clientX,
      startWidth: thRect.width,
      edgeX: thRect.right - wrapperRect.left,
    };
    resizeGuide = resizing.edgeX;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    document.addEventListener('mousemove', onResize);
    document.addEventListener('mouseup', stopResize);
  }

  function onResize(e) {
    if (!resizing) return;
    const diff = e.clientX - resizing.startX;
    const newWidth = Math.max(80, Math.min(800, resizing.startWidth + diff));
    // Clamp the guide to match the clamped width so the glowing line
    // stops moving when the column hits its min/max size.
    const clampedDiff = newWidth - resizing.startWidth;
    resizeGuide = resizing.edgeX + clampedDiff;
    columnWidths = { ...columnWidths, [resizing.colIndex]: newWidth };
  }

  function stopResize() {
    resizing = null;
    resizeGuide = null;
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
    document.removeEventListener('mousemove', onResize);
    document.removeEventListener('mouseup', stopResize);
  }

  // --- Checkbox ---
  function toggleCheckAll() {
    const visibleCount = pageRows.length;
    if (checkedRows.size === visibleCount) {
      checkedRows = new Set();
    } else {
      checkedRows = new Set(
        Array.from({ length: visibleCount }, (_, i) => page * PAGE_SIZE + i),
      );
    }
  }

  function toggleCheck(i) {
    const absoluteIndex = page * PAGE_SIZE + i;
    const next = new Set(checkedRows);
    if (next.has(absoluteIndex)) next.delete(absoluteIndex);
    else next.add(absoluteIndex);
    checkedRows = next;
  }

  // --- Cell formatting ---
  function cellClass(value) {
    if (value === null || value === undefined) return 'cell-null';
    if (typeof value === 'boolean') return 'cell-bool';
    if (typeof value === 'number') return 'cell-number';
    return '';
  }

  // ⚡ Bolt: Cache Intl.NumberFormat instances to avoid recreating them on every cell render.
  // Using toLocaleString() repeatedly with options is ~50x slower than reusing a formatter.
  const floatFormatter = new Intl.NumberFormat(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 6,
  });

  function formatCell(value) {
    if (value === null || value === undefined) return 'NULL';
    if (typeof value === 'boolean') return String(value);
    if (typeof value === 'number') {
      if (Number.isInteger(value)) return value.toLocaleString();
      return floatFormatter.format(value);
    }
    return String(value);
  }
</script>

<svelte:window onkeydown={handleWindowKeydown} />

<div class="results-grid" class:compact>
  <!-- Action Toolbar -->
  <div class="toolbar">
    <div class="toolbar-left">
      {#if error}
        <span class="error-summary">{error}</span>
      {:else if fetchedCount > 0}
        <span class="row-count">
          <span class="check-icon">✓</span>
          {#if compact}
            {fetchedCount.toLocaleString()} rows
          {:else}
            {fetchedCount.toLocaleString()} row{fetchedCount === 1 ? '' : 's'} fetched
          {/if}
          {#if totalCount !== null}
            <span class="total-count"
              >({totalCount.toLocaleString()} total)</span
            >
          {/if}
        </span>
        {#if duration > 0}
          <span class="duration">{duration}s</span>
        {/if}
        {#if !compact && totalCount === null && onCountAll}
          <button class="tool-btn count-btn" onclick={onCountAll}
            >Count all rows</button
          >
        {/if}
      {:else}
        <span class="no-results">No results</span>
      {/if}
    </div>
    <div class="toolbar-right">
      {#if compact && totalCount === null && onCountAll && fetchedCount > 0}
        <button
          class="icon-tool-btn"
          onclick={onCountAll}
          title="Count all rows"
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
          >
            <path d="M3 3h18v4H3z" /><path d="M3 10h18v4H3z" /><path
              d="M3 17h18v4H3z"
            />
          </svg>
        </button>
      {/if}
      <button class="tool-btn" class:icon-only={compact} title="Export CSV">
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
          <polyline points="7 10 12 15 17 10" />
          <line x1="12" y1="15" x2="12" y2="3" />
        </svg>
        {#if !compact}<span>Export</span>{/if}
      </button>
      <button
        class="tool-btn filter-btn"
        class:icon-only={compact}
        class:active={barVisible}
        onclick={toggleBar}
        title="Filter"
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
        >
          <polygon points="22 3 2 3 10 12.46 10 19 14 21 14 12.46 22 3" />
        </svg>
        {#if !compact}<span>Filter</span>{/if}
      </button>
    </div>
  </div>

  {#if barVisible}
    <FilterBar
      {columns}
      {filters}
      {compact}
      {pickerOpen}
      {onDescribeFilters}
      onFiltersChange={handleFiltersChange}
      onPickerOpenChange={(open) => (pickerOpen = open)}
    />
  {/if}

  {#if error}
    <div class="error-panel">
      <div class="error-panel-header">
        <span class="error-panel-icon">!</span>
        <span class="error-panel-title">Query Failed</span>
      </div>
      <pre class="error-panel-message">{error}</pre>
    </div>
  {:else if rows.length === 0}
    <!-- Empty state: no rows returned -->
    <div class="empty-state">
      <div class="empty-icon">
        <svg
          width="48"
          height="48"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <path d="M3 9h18" />
          <path d="M9 3v18" />
        </svg>
      </div>
      {#if hasActiveFilters}
        <span class="empty-title">No rows match your filters</span>
        <span class="empty-desc">Loosen or remove a filter to see rows</span>
        <button class="tool-btn" onclick={clearFilters}>Clear filters</button>
      {:else}
        <span class="empty-title">No rows found</span>
        <span class="empty-desc">The query returned no results</span>
      {/if}
    </div>
  {:else if columns.length > 0}
    <div class="table-wrapper" class:loading bind:this={tableWrapperEl}>
      {#if resizeGuide !== null}
        <div class="resize-guide" style="left: {resizeGuide}px"></div>
      {/if}
      <table>
        <thead>
          <tr>
            <th class="row-num">
              <input
                type="checkbox"
                onchange={toggleCheckAll}
                checked={checkedRows.size === pageRows.length &&
                  pageRows.length > 0}
              />
            </th>
            {#each columns as col, i}
              <th
                class="sortable"
                class:active={sortCol === col.name}
                style="width: {getColWidth(i)}px; min-width: 80px;"
                title={col.type_name}
              >
                <div class="col-header">
                  <button
                    class="col-info"
                    aria-label="Sort by {col.name}"
                    onclick={() => toggleSort(i)}
                  >
                    <span class="col-name">{col.name}{sortIndicator(i)}</span>
                    <span class="col-type">{col.type_name}</span>
                  </button>
                  <button
                    class="col-menu-trigger"
                    aria-label="Column actions for {col.name}"
                    aria-expanded={columnMenu?.column === col.name}
                    onclick={(e) => openColumnMenu(e, col)}>⌄</button
                  >
                  <div
                    class="resize-handle"
                    onmousedown={(e) => startResize(e, i)}
                    onclick={(e) => e.stopPropagation()}
                  ></div>
                </div>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each pageRows as row, i}
            <tr class:even={(page * PAGE_SIZE + i) % 2 === 0}>
              <td class="row-num">
                <input
                  type="checkbox"
                  onchange={() => toggleCheck(i)}
                  checked={checkedRows.has(page * PAGE_SIZE + i)}
                />
              </td>
              {#each row as cell, j}
                <td
                  class={cellClass(cell)}
                  style="width: {getColWidth(j)}px; min-width: 80px;"
                  oncontextmenu={(e) => openCellMenu(e, j, cell)}
                >
                  {#if typeof cell === 'boolean'}
                    <span
                      class="bool-badge"
                      class:true={cell}
                      class:false={!cell}>{String(cell)}</span
                    >
                  {:else}
                    <span class="cell-content">{formatCell(cell)}</span>
                  {/if}
                </td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    <!-- Page-based pagination: hidden when all rows fit on one page -->
    {#if !(isEnd && fetchedCount <= PAGE_SIZE)}
      <div class="pagination">
        <span class="page-info">
          Rows {Math.min(
            page * PAGE_SIZE + 1,
            fetchedCount,
          ).toLocaleString()}–{Math.min(
            (page + 1) * PAGE_SIZE,
            fetchedCount,
          ).toLocaleString()}
          {#if totalCount !== null}
            of {totalCount.toLocaleString()}
          {:else if isEnd}
            of {fetchedCount.toLocaleString()}
          {:else}
            of {fetchedCount.toLocaleString()}+
          {/if}
        </span>
        <div class="page-controls">
          <button class="page-btn" onclick={goPrev} disabled={page === 0}
            >&lsaquo; Prev</button
          >
          <span class="page-number">Page {page + 1}</span>
          <button class="page-btn" onclick={goNext} disabled={!canGoNext}
            >Next &rsaquo;</button
          >
        </div>
      </div>
    {/if}
  {/if}
</div>

{#if columnMenu}
  <GridMenu
    x={columnMenu.x}
    y={columnMenu.y}
    items={COLUMN_MENU_ITEMS}
    onSelect={handleColumnMenuSelect}
    onClose={() => (columnMenu = null)}
  />
{/if}

{#if cellMenu}
  <GridMenu
    x={cellMenu.x}
    y={cellMenu.y}
    items={cellMenuItems}
    onSelect={handleCellMenuSelect}
    onClose={() => (cellMenu = null)}
  />
{/if}

<style>
  .results-grid {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--bg-surface);
  }

  /* Toolbar */
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-4);
    height: 40px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-surface);
  }
  .toolbar-left {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .toolbar-right {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .row-count {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-base);
    font-weight: var(--weight-medium);
    color: var(--text);
  }
  .check-icon {
    color: var(--success);
    font-weight: var(--weight-bold);
  }
  .duration {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .error-summary {
    color: var(--danger);
    font-weight: var(--weight-medium);
    font-size: var(--text-base);
  }
  .no-results {
    color: var(--text-muted);
    font-style: italic;
    font-size: var(--text-base);
  }
  .error-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: var(--space-6);
    gap: var(--space-3);
  }
  .error-panel-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .error-panel-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: var(--danger);
    color: white;
    font-size: 13px;
    font-weight: var(--weight-bold);
    line-height: 1;
  }
  .error-panel-title {
    font-size: var(--text-lg);
    font-weight: var(--weight-semibold);
    color: var(--danger);
  }
  .error-panel-message {
    max-width: 600px;
    padding: var(--space-3);
    background: rgba(0, 0, 0, 0.04);
    border-radius: var(--radius-md);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    line-height: 1.6;
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
    overflow-x: auto;
    max-height: 300px;
  }
  :global(.dark) .error-panel-message {
    background: rgba(255, 255, 255, 0.04);
  }
  .tool-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 12px;
    border: 1px solid var(--border);
    border-radius: 20px;
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    cursor: pointer;
    transition: all var(--transition-fast);
  }
  /* Compact (icon-only) mode collapses pill to a square ghost button */
  .tool-btn.icon-only {
    padding: 5px;
    border-radius: var(--radius-md);
    border-color: transparent;
    background: transparent;
  }
  .tool-btn.icon-only:hover {
    border-color: transparent;
    background: var(--bg-hover);
    transform: none;
  }
  .tool-btn.icon-only.active {
    border-color: transparent;
    background: var(--accent-soft);
  }
  .icon-tool-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    transition: all var(--transition-fast);
    flex-shrink: 0;
  }
  .icon-tool-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .tool-btn:hover {
    background: var(--bg-hover);
    border-color: var(--text-muted);
    color: var(--text);
    transform: scale(1.02);
  }
  .tool-btn.active {
    background: var(--accent-soft);
    color: var(--accent);
    border-color: var(--accent);
  }
  .tool-btn svg {
    flex-shrink: 0;
  }

  /* Table */
  .table-wrapper {
    flex: 1;
    overflow: auto;
    border-top: 1px solid var(--grid-line);
    position: relative;
  }
  table {
    width: 100%;
    table-layout: fixed;
    border-collapse: collapse;
    font-size: var(--text-base);
  }

  /* Sticky header */
  thead {
    position: sticky;
    top: 0;
    z-index: 2;
  }
  thead tr:first-child th {
    background: var(--bg-elevated);
    border-bottom: 2px solid var(--grid-header-border);
    box-shadow: 0 1px 0 var(--grid-line);
  }
  th {
    text-align: left;
    padding: var(--space-2) var(--space-3);
    background: var(--bg-elevated);
    border-bottom: 2px solid var(--grid-header-border);
    border-right: 1px solid var(--grid-line);
    font-weight: var(--weight-semibold);
    font-size: var(--text-sm);
    color: var(--text);
    white-space: nowrap;
    user-select: none;
  }
  th:last-child {
    border-right: none;
  }
  th.sortable {
    cursor: pointer;
  }
  th.sortable:hover {
    background: var(--bg-hover);
  }
  th.active .col-name {
    color: var(--accent);
  }
  .col-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
  }
  /* .col-info is now a <button> styled above */
  .col-name {
    font-weight: var(--weight-semibold);
    letter-spacing: 0.04em;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .col-type {
    font-size: var(--text-xs);
    font-weight: var(--weight-normal);
    color: var(--text-muted);
    text-transform: none;
    letter-spacing: 0;
    margin-top: 2px;
  }
  th.row-num {
    width: 44px;
    text-align: center;
    padding: var(--space-2) 4px;
    border-right: 1px solid var(--grid-line);
  }
  th.row-num input {
    cursor: pointer;
  }

  /* Resize handle — positioned relative to th so it sits exactly on the column border */
  th.sortable {
    position: relative;
  }
  .resize-handle {
    position: absolute;
    right: -8px;
    top: 0;
    bottom: 0;
    width: 16px;
    cursor: col-resize;
    z-index: 3;
  }
  .resize-handle::after {
    content: '';
    position: absolute;
    /* Center the 2px indicator within the 16px handle — aligns on the th right border */
    left: 50%;
    transform: translateX(-50%);
    top: 4px;
    bottom: 4px;
    width: 2px;
    background: transparent;
    border-radius: 1px;
  }

  .resize-handle:hover::after {
    background: var(--accent);
  }
  .resize-guide {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--accent);
    z-index: 10;
    pointer-events: none;
  }
  .resize-guide::after {
    content: '';
    position: absolute;
    top: 0;
    left: -3px;
    right: -3px;
    height: 100%;
    background: transparent;
    box-shadow: 0 0 4px 2px var(--accent);
    opacity: 0.5;
  }

  /* Zebra + hover + row borders */
  tr.even td {
    background: var(--bg-subtle);
  }
  tr:hover td {
    background: var(--bg-hover);
  }
  tr:hover td:first-child {
    box-shadow: inset 3px 0 0 var(--accent);
  }
  td {
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--grid-line);
    border-right: 1px solid var(--grid-line);
    white-space: nowrap;
  }
  td:last-child {
    border-right: none;
  }
  td.row-num {
    text-align: center;
    padding: var(--space-2) 4px;
    width: 44px;
    border-right: 1px solid var(--grid-line);
  }
  td.row-num input {
    cursor: pointer;
  }
  td.cell-null {
    color: var(--text-muted);
    font-style: italic;
    text-decoration: underline;
    text-decoration-style: dotted;
    text-underline-offset: 3px;
  }
  td.cell-number {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  td.cell-bool {
    text-align: left;
  }

  /* Boolean badges */
  .bool-badge {
    display: inline-block;
    padding: 2px 10px;
    border-radius: var(--radius-full);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
  }
  .bool-badge.true {
    background: var(--success-bg);
    color: var(--success);
  }
  .bool-badge.false {
    background: var(--danger-bg);
    color: var(--danger);
  }

  /* Cell content ellipsis */
  .cell-content {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Checkbox styling */
  td input[type='checkbox'],
  th input[type='checkbox'] {
    width: 14px;
    height: 14px;
    accent-color: var(--accent);
    cursor: pointer;
  }

  /* Empty state */
  .empty-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-10);
    color: var(--text-muted);
  }
  .empty-icon {
    opacity: 0.3;
  }
  .empty-title {
    font-size: var(--text-lg);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
  }
  .empty-desc {
    font-size: var(--text-base);
    color: var(--text-muted);
  }

  /* Pagination */
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

  /* Count all rows */
  .total-count {
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: var(--weight-normal);
  }
  .count-btn {
    margin-left: var(--space-2);
    font-size: var(--text-sm);
  }
  .table-wrapper.loading tbody {
    opacity: 0.45;
    transition: opacity var(--transition-fast);
  }
  .table-wrapper.loading::before {
    content: '';
    position: sticky;
    top: 0;
    left: 0;
    display: block;
    height: 2px;
    background: linear-gradient(90deg, transparent, var(--accent), transparent);
    animation: filter-loading 1s linear infinite;
    z-index: 4;
  }
  @keyframes filter-loading {
    from {
      background-position: -200px 0;
    }
    to {
      background-position: 200px 0;
    }
  }

  .col-info {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    overflow: hidden;
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .col-menu-trigger {
    flex-shrink: 0;
    width: 18px;
    height: 18px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    line-height: 1;
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--transition-fast);
  }
  th:hover .col-menu-trigger,
  .col-menu-trigger:focus-visible {
    opacity: 1;
  }
</style>
