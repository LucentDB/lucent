<script>
  import { untrack, onDestroy } from 'svelte';
  import FilterBar from './FilterBar.svelte';
  import GridMenu from './GridMenu.svelte';
  import GridHeader from './GridHeader.svelte';
  import GridBody from './GridBody.svelte';
  import GridPagination from './GridPagination.svelte';
  import { createGridEngine } from './engine.svelte.ts';
  import { createPagedStream } from './pagedStream.svelte.ts';
  import { formatCell } from './format.js';
  import { toTsv, toCsv } from './serialize.js';
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
    truncated = false,
    duration = 0,
    error = null,
    tabId = null,
    initFilters = [],
    initSorting = null,
    onStateChange = null,
    onNeedMore = null,
    onCountAll = null,
    compact = false,
    loading = false,
    onDescribeFilters = null,
    pageSize = 200,
    embedded = false,
    summary = null,
  } = $props();

  let filters = $state(normalize($state.snapshot(initFilters)));
  let columnWidths = $state({});
  let barOpen = $state(false);
  let pickerOpen = $state(false);
  let resizing = $state(null);
  let resizeGuide = $state(null);
  /** True while a cell-range drag is in flight; mouseenter extends only then. */
  let dragging = $state(false);
  let tableWrapperEl = $state(null);
  let columnMenu = $state(null);
  let cellMenu = $state(null);

  // Clears the document-level drag listeners if the grid unmounts mid-drag.
  // Without this, an unmount leaves mousemove/mouseup attached to `document`
  // and the body cursor stuck at col-resize until the next mouseup.
  let activeDragCleanup = null;

  // Both factories open $effect scopes, so both are called here at init.
  // Every reactive input crosses as a getter — a value would silently freeze
  // the grid on page one. Spec §3.2.
  //
  // ORDER MATTERS: `stream` is constructed FIRST. The engine's
  // onSortingChange callback closes over `stream` via emitChange, and `const`
  // bindings are in their temporal dead zone until initialised — so creating
  // the engine first would throw a ReferenceError if it ever emitted during
  // construction.
  const stream = createPagedStream({
    get rows() {
      return rows;
    },
    get fetchedCount() {
      return fetchedCount;
    },
    get isEnd() {
      return isEnd;
    },
    get totalCount() {
      return totalCount;
    },
    get pageSize() {
      return pageSize;
    },
    get tabId() {
      return tabId;
    },
    onNeedMore: () => onNeedMore?.(),
    onScrollReset: () => {
      if (tableWrapperEl) tableWrapperEl.scrollTop = 0;
    },
  });

  const engine = createGridEngine({
    get columns() {
      return columns;
    },
    get rows() {
      return rows;
    },
    get initialSorting() {
      return initSorting ?? [];
    },
    get initialFilters() {
      return filters;
    },
    onSortingChange: () => emitChange(),
  });

  // Reset local UI state on tab switch. Reading initFilters directly would make
  // it a dependency, so every parent re-emit would collapse the filter row
  // mid-type. Paging resets inside the stream via its own tabId effect; only
  // the purely local UI state is handled here.
  //
  // Sorting is ALSO restored to the incoming tab's saved sort: Svelte reuses
  // this component instance across tabs, so without this the previous tab's
  // header indicators and emitted sort would leak into the new one. The
  // restore must not look like a user-initiated change — no refetch, no
  // onStateChange — which is what restoringTabState suppresses.
  let restoringTabState = false;

  $effect(() => {
    void tabId;
    untrack(() => {
      filters = normalize(initFilters);
      engine.clearRowSelection();
      engine.clearCellSelection();
      barOpen = false;
      pickerOpen = false;
      restoringTabState = true;
      try {
        engine.table.setSorting(initSorting ?? []);
      } finally {
        restoringTabState = false;
      }
      if (tableWrapperEl) {
        tableWrapperEl.scrollTop = 0;
        tableWrapperEl.scrollLeft = 0;
      }
    });
  });

  function emitChange() {
    if (restoringTabState) return; // a tab switch restores state, it does not change it
    stream.reset();
    engine.clearRowSelection();
    // A committed sort/filter refetches with new order or rows; positional
    // cell ranges would point at different values afterwards.
    engine.clearCellSelection();
    onStateChange?.({ filters, sorting: engine.sorting });
  }

  function toggleSort(columnId, event) {
    const column = engine.table.getColumn(columnId);
    if (!column) return;
    // Passing the event lets Table apply isMultiSortEvent, so shift-click
    // appends a key instead of replacing the sort.
    column.getToggleSortingHandler()?.(event);
  }

  function sortIndexOf(columnId) {
    return engine.sortIndexOf(columnId);
  }

  function sortDirectionOf(columnId) {
    return engine.table.getColumn(columnId)?.getIsSorted() ?? false;
  }

  function onCellMouseDown(rowIndex, columnId, event) {
    if (event.button !== 0) return;
    event.preventDefault();
    dragging = true;
    if (event.shiftKey) engine.extendCellSelection({ rowIndex, columnId });
    else engine.startCellSelection({ rowIndex, columnId });
    // A drag can end anywhere, including outside the window.
    const stop = () => {
      dragging = false;
      window.removeEventListener('mouseup', stop);
    };
    window.addEventListener('mouseup', stop);
    activeDragCleanup = stop;
  }

  function onCellMouseEnter(rowIndex, columnId) {
    if (!dragging) return;
    engine.extendCellSelection({ rowIndex, columnId });
  }

  const ARROWS = {
    ArrowUp: 'up',
    ArrowDown: 'down',
    ArrowLeft: 'left',
    ArrowRight: 'right',
  };

  /**
   * What Cmd+C copies, in priority order: an active cell range, else a row
   * selection, else nothing (so the browser's own copy still works on, say,
   * selected text in the filter bar).
   */
  function copySelection(format = 'tsv') {
    const cells = engine.selectedCellRangesData();
    // `payload`, not `rows` — the local must not shadow the accumulated-buffer prop.
    const payload =
      cells.length > 0
        ? cells
        : engine.selectedRowIndices().map((i) => rows[i]).filter(Boolean);
    if (payload.length === 0) return false;
    const text = format === 'csv' ? toCsv(payload) : toTsv(payload);
    navigator.clipboard?.writeText(text).catch(() => {});
    return true;
  }

  function handleGridKeydown(e) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'c') {
      if (copySelection('tsv')) e.preventDefault();
      return;
    }
    if (ARROWS[e.key]) {
      e.preventDefault();
      engine.moveCellSelection(ARROWS[e.key], e.shiftKey);
      // Keep the focused cell on screen when arrowing past the viewport edge.
      queueMicrotask(() => {
        tableWrapperEl
          ?.querySelector('td.focused')
          ?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
      });
      return;
    }
    if ((e.metaKey || e.ctrlKey) && e.key === 'a') {
      e.preventDefault();
      engine.selectAllCells();
      return;
    }
    if (e.key === 'Escape') {
      engine.clearCellSelection();
    }
  }

  /** Absolute indices of gutter-selected rows, for cell styling and copy. */
  const selectedRows = $derived(new Set(engine.selectedRowIndices()));

  function selectRow(absolute, opts) {
    engine.selectRow(absolute, opts);
  }

  function toggleSelectAllPage() {
    const offset = stream.page * pageSize;
    const onPage = stream.pageRows.map((_, i) => offset + i);
    if (onPage.length === 0) return;
    if (onPage.every((i) => selectedRows.has(i))) {
      engine.clearRowSelection();
      return;
    }
    // Union semantics: every not-yet-selected page row joins; selections on
    // this page and other pages survive. A per-row toggle would REMOVE rows
    // that were already selected — the opposite of what a select-all
    // affordance means.
    engine.selectRows(onPage.filter((i) => !selectedRows.has(i)));
  }

  // Filter bar
  let barVisible = $derived(barOpen || filters.length > 0);
  let hasActiveFilters = $derived(applyable(filters).length > 0);

  /**
   * Counting is only worth offering when rows might exist beyond what we hold.
   * Once isEnd is true the fetched count IS the total, so the button would run
   * a COUNT(*) to restate a number already on screen.
   */
  let canCountAll = $derived(
    !!onCountAll && totalCount === null && !isEnd && fetchedCount > 0,
  );

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
    { id: 'pin-left', label: 'Pin to left' },
    { id: 'pin-right', label: 'Pin to right' },
    { id: 'unpin', label: 'Unpin' },
    { separator: true },
    { id: 'filter', label: 'Filter by this column' },
    { id: 'copy', label: 'Copy column name' },
  ];

  // Spec §4.4: the pin affordances need horizontal room to be worth it.
  // Below a 640px container they hide; cell-range selection stays at every
  // width. The wrapper measures itself, so embedded mounts gate correctly too.
  let wrapperWidth = $state(0);
  const pinningAvailable = $derived(wrapperWidth >= 640);
  const columnMenuItems = $derived(
    pinningAvailable
      ? COLUMN_MENU_ITEMS
      : COLUMN_MENU_ITEMS.filter(
          (i) => !String(i.id ?? '').startsWith('pin') && i.id !== 'unpin',
        ),
  );

  /** The header passes a column ID; meta carries the display name/type. */
  function openColumnMenu(e, colId) {
    e.stopPropagation();
    const column = engine.table.getColumn(colId);
    if (!column) return;
    const meta = column.columnDef.meta ?? {};
    const r = e.currentTarget.getBoundingClientRect();
    columnMenu = {
      id: colId,
      column: meta.name,
      typeName: meta.typeName,
      x: r.left,
      y: r.bottom + 4,
    };
  }

  function handleColumnMenuSelect(action) {
    const { column, typeName, id } = columnMenu;
    if (action === 'asc' || action === 'desc') {
      // An explicit desc argument makes toggleSorting a deterministic SET —
      // re-picking the current direction keeps it sorted that way.
      engine.table.getColumn(id)?.toggleSorting(action === 'desc');
      return;
    }
    if (action === 'clear-sort') {
      engine.table.getColumn(id)?.clearSorting();
      return;
    }
    if (action === 'pin-left') return engine.pinColumn(id, 'left');
    if (action === 'pin-right') return engine.pinColumn(id, 'right');
    if (action === 'unpin') return engine.pinColumn(id, false);
    if (action === 'filter') {
      barOpen = true;
      const next = addFilter(filters, column, typeName);
      const added = next[next.length - 1];
      filters = next;
      if (isComplete(added)) emitChange();
      return;
    }
    if (action === 'copy') {
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
            { separator: true },
            { id: 'copy-selection-tsv', label: 'Copy selection (TSV)' },
            { id: 'copy-selection-csv', label: 'Copy selection (CSV)' },
          ]
        : [
            { id: 'filter', label: 'Filter by this value' },
            { id: 'filter-out', label: 'Filter out this value' },
            { separator: true },
            { id: 'copy', label: 'Copy value' },
            { separator: true },
            { id: 'copy-selection-tsv', label: 'Copy selection (TSV)' },
            { id: 'copy-selection-csv', label: 'Copy selection (CSV)' },
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
    if (id === 'copy-selection-tsv') return void copySelection('tsv');
    if (id === 'copy-selection-csv') return void copySelection('csv');
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

  // --- Column resize ---
  const MIN_COL_WIDTH = 80;
  const MAX_COL_WIDTH = 800;

  function getColWidth(i) {
    return columnWidths[i] || 150;
  }

  /**
   * Keyboard equivalent of dragging the resize handle. The handle is a focusable
   * separator, so it needs to respond to arrows or it is a focus trap that does
   * nothing.
   */
  function handleResizeKeydown(e, i) {
    const STEP = e.shiftKey ? 40 : 8;
    let next = null;
    if (e.key === 'ArrowLeft') next = getColWidth(i) - STEP;
    else if (e.key === 'ArrowRight') next = getColWidth(i) + STEP;
    else if (e.key === 'Home') next = MIN_COL_WIDTH;
    else if (e.key === 'End') next = MAX_COL_WIDTH;
    if (next === null) return;
    e.preventDefault();
    columnWidths = {
      ...columnWidths,
      [i]: Math.max(MIN_COL_WIDTH, Math.min(MAX_COL_WIDTH, next)),
    };
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
    activeDragCleanup = stopResize;
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
    activeDragCleanup = null;
  }

  onDestroy(() => {
    activeDragCleanup?.();
  });
</script>

<svelte:window onkeydown={handleWindowKeydown} />

<div class="results-grid" class:compact class:embedded>
  <!-- Action Toolbar -->
  <div class="toolbar">
    <div class="toolbar-left">
      {#if error}
        <span class="error-summary">{error}</span>
      {:else if fetchedCount > 0}
        <span class="row-count">
          <span class="check-icon">✓</span>
          {#if compact || isEnd}
            <!-- Complete result: the count is the count, nothing was "fetched
                 so far". -->
            {fetchedCount.toLocaleString()} row{fetchedCount === 1 ? '' : 's'}
          {:else}
            {fetchedCount.toLocaleString()} row{fetchedCount === 1 ? '' : 's'} fetched
          {/if}
          {#if totalCount !== null && totalCount !== fetchedCount}
            <span class="total-count"
              >({totalCount.toLocaleString()} total)</span
            >
          {/if}
        </span>
        {#if truncated}
          <span class="truncated-note"
            >— stopped at {fetchedCount.toLocaleString()} rows; the query was cancelled
            on the server</span
          >
        {/if}
        {#if duration > 0}
          <span class="duration">{duration}s</span>
        {/if}
        {#if canCountAll && !compact}
          <button class="tool-btn count-btn" onclick={onCountAll}
            >Count all rows</button
          >
        {/if}
      {:else if summary}
        <span class="no-results">{summary}</span>
      {:else}
        <span class="no-results">No results</span>
      {/if}
    </div>
    <div class="toolbar-right">
      {#if canCountAll && compact}
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
      {:else if summary}
        <span class="empty-title">{summary}</span>
      {:else}
        <span class="empty-title">No rows found</span>
        <span class="empty-desc">The query returned no results</span>
      {/if}
    </div>
  {:else if columns.length > 0}
    <div
      class="table-wrapper"
      class:loading
      bind:this={tableWrapperEl}
      bind:clientWidth={wrapperWidth}
      onkeydown={handleGridKeydown}
    >
      {#if loading}
        <div class="refetch-bar" role="status" aria-label="Refreshing rows">
          <span class="refetch-bar-fill"></span>
        </div>
      {/if}
      {#if resizeGuide !== null}
        <div class="resize-guide" style="left: {resizeGuide}px"></div>
      {/if}
      <table>
        <GridHeader
          table={engine.table}
          {columnWidths}
          {sortIndexOf}
          {sortDirectionOf}
          onToggleSort={toggleSort}
          onOpenMenu={openColumnMenu}
          onResizeStart={startResize}
          onResizeKeydown={handleResizeKeydown}
          onToggleSelectAllPage={toggleSelectAllPage}
          openColumnId={columnMenu?.id ?? null}
          allPageSelected={stream.pageRows.length > 0 &&
            stream.pageRows.every(
              (_, i) => selectedRows.has(stream.page * pageSize + i),
            )}
        />
        <GridBody
          table={engine.table}
          pageRows={stream.pageRows}
          pageOffset={stream.page * pageSize}
          {columnWidths}
          {selectedRows}
          onSelectRow={selectRow}
          onCellContextMenu={openCellMenu}
          {onCellMouseDown}
          {onCellMouseEnter}
        />
      </table>
    </div>

    <!-- Page-based pagination: hidden when all rows fit on one page -->
    {#if !stream.fitsOnePage}
      <GridPagination
        page={stream.page}
        firstRowNumber={stream.firstRowNumber}
        lastRowNumber={stream.lastRowNumber}
        {fetchedCount}
        {totalCount}
        {isEnd}
        canGoNext={stream.canGoNext}
        isFetchingMore={stream.isFetchingMore}
        onNext={stream.goNext}
        onPrev={stream.goPrev}
        {embedded}
      />
    {/if}
  {/if}
</div>

{#if columnMenu}
  <GridMenu
    x={columnMenu.x}
    y={columnMenu.y}
    items={columnMenuItems}
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

  /* Embedded: the cell already provides framing, so the grid drops its own. */
  .results-grid.embedded {
    border: none;
    border-radius: 0;
    background: transparent;
  }
  /* Header/body cells live in child components now — Svelte scoping needs
     :global for these descendant selectors to reach them. */
  .results-grid.embedded :global(th),
  .results-grid.embedded :global(td) {
    padding-top: 2px;
    padding-bottom: 2px;
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
  .truncated-note {
    color: var(--warning);
    font-size: var(--text-sm);
    margin-left: 0.5rem;
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
  /* Refetching keeps the previous rows readable but clearly stale, rather than
     blanking the grid — blanking used to unmount the filter UI entirely.
     tbody renders inside GridBody — Svelte scoping needs :global. */
  .table-wrapper.loading :global(tbody) {
    opacity: 0.5;
    transition: opacity var(--transition-normal);
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
  /* An indeterminate bar driven by transform, not background-position: a
     gradient sized to its element cannot be swept by shifting its position,
     and transform is the only property here that stays off the paint path. */
  .refetch-bar {
    position: sticky;
    top: 0;
    left: 0;
    z-index: 5;
    height: 2px;
    overflow: hidden;
    background: var(--accent-soft);
  }
  .refetch-bar-fill {
    display: block;
    width: 40%;
    height: 100%;
    background: var(--accent);
    border-radius: var(--radius-full);
    animation: refetch-sweep 1.1s cubic-bezier(0.65, 0, 0.35, 1) infinite;
    will-change: transform;
  }
  @keyframes refetch-sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(250%);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .refetch-bar-fill {
      width: 100%;
      animation: refetch-pulse 1.4s ease-in-out infinite;
    }
    @keyframes refetch-pulse {
      0%,
      100% {
        opacity: 0.35;
      }
      50% {
        opacity: 1;
      }
    }
  }
</style>
