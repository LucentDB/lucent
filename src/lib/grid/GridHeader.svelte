<script>
  let {
    table,
    columnWidths = {},
    sortIndicatorFor = () => '',
    onToggleSort,
    onOpenMenu,
    onResizeStart,
    onResizeKeydown,
    onToggleCheckAll,
    allChecked = false,
  } = $props();

  const headers = $derived(table.getHeaderGroups()[0]?.headers ?? []);

  /** Column meta carries the display name and index; the id is positional. */
  function metaOf(header) {
    return header.column.columnDef.meta ?? {};
  }

  function widthOf(index) {
    return columnWidths[index] || 150;
  }
</script>

<thead>
  <tr>
    <th class="row-num">
      <input type="checkbox" onchange={onToggleCheckAll} checked={allChecked} />
    </th>
    {#each headers as header (header.id)}
      {@const meta = metaOf(header)}
      <th
        class="sortable"
        class:active={header.column.getIsSorted() !== false}
        style="width: {widthOf(meta.index)}px; min-width: 80px;"
        title={meta.typeName}
      >
        <div class="col-header">
          <button
            class="col-info"
            aria-label="Sort by {meta.name}"
            onclick={() => onToggleSort(header.column.id)}
          >
            <span class="col-name"
              >{meta.name}{sortIndicatorFor(header.column.id)}</span
            >
            <span class="col-type">{meta.typeName}</span>
          </button>
          <button
            class="col-menu-trigger"
            aria-label="Column actions for {meta.name}"
            aria-haspopup="menu"
            onclick={(e) => onOpenMenu(e, header.column.id)}
          >
            <svg
              width="10"
              height="10"
              viewBox="0 0 12 12"
              fill="none"
              stroke="currentColor"
              stroke-width="1.75"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M3 4.5 6 7.5 9 4.5" />
            </svg>
          </button>
          <!-- A button, not a div with role="separator": a focusable separator
               is valid ARIA but Svelte's a11y checker treats the role as
               non-interactive, and a button gives the same keyboard affordance
               without the lint exception. -->
          <button
            class="resize-handle"
            aria-label="Resize {meta.name} column, currently {widthOf(
              meta.index,
            )} pixels"
            onmousedown={(e) => onResizeStart(e, meta.index)}
            onkeydown={(e) => onResizeKeydown(e, meta.index)}
            onclick={(e) => e.stopPropagation()}
          ></button>
        </div>
      </th>
    {/each}
  </tr>
</thead>

<style>
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
    padding: 4px 8px;
    height: 38px;
    background: var(--bg-elevated);
    border-bottom: 2px solid var(--grid-header-border);
    border-right: 1px solid var(--grid-line);
    font-weight: var(--weight-semibold);
    font-size: var(--text-xs);
    color: var(--text);
    white-space: nowrap;
    user-select: none;
    box-sizing: border-box;
  }
  th:last-child {
    border-right: none;
  }
  th.sortable {
    cursor: pointer;
  }
  .col-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 4px;
    width: 100%;
    height: 100%;
  }
  .col-info {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    gap: 1px;
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: inherit;
    cursor: pointer;
    overflow: hidden;
    min-width: 0;
    flex: 1;
    text-align: left;
  }
  .col-name {
    font-weight: var(--weight-semibold);
    font-size: 12px;
    letter-spacing: 0.01em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    width: 100%;
  }
  .col-type {
    font-size: 10px;
    font-family: var(--font-mono);
    font-weight: var(--weight-normal);
    color: var(--text-muted);
    text-transform: lowercase;
    opacity: 0.75;
    letter-spacing: 0;
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    width: 100%;
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
    padding: 0;
    border: none;
    background: transparent;
    cursor: col-resize;
    z-index: 3;
  }
  /* The 2px indicator is drawn by ::after, so the default ring would sit 16px
     wide over the neighbouring column. Show the indicator instead. */
  .resize-handle:focus-visible {
    outline: none;
  }
  .resize-handle:focus-visible::after {
    background: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
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

  .col-menu-trigger {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 18px;
    height: 18px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    opacity: 0;
    transition:
      opacity var(--transition-fast),
      background var(--transition-fast),
      color var(--transition-fast);
  }
  .col-menu-trigger:hover {
    background: var(--bg-surface);
    color: var(--text);
  }
  /* Revealed on hover, but never hidden from keyboard users — an invisible
     trigger that still takes focus is a trap. When Task 8 wires the column
     menu through this component it must also re-add the
     `.col-menu-trigger[aria-expanded='true'] { opacity: 1 }` rule alongside
     the aria-expanded attribute (present in ResultsGrid.svelte today). */
  th:hover .col-menu-trigger,
  .col-menu-trigger:focus-visible {
    opacity: 1;
  }
</style>
