<script>
  import { GUTTER_WIDTH } from './engine.svelte.ts';

  let {
    table,
    columnWidths = {},
    sortIndexOf = () => -1,
    sortDirectionOf = () => false,
    onToggleSort,
    onOpenMenu,
    onResizeStart,
    onResizeKeydown,
    onToggleSelectAllPage,
    /** ID of the column whose menu is open, so aria-expanded can track it. */
    openColumnId = null,
    allPageSelected = false,
  } = $props();

  const startHeaders = $derived(table.getStartHeaderGroups()[0]?.headers ?? []);
  const centerHeaders = $derived(
    table.getCenterHeaderGroups()[0]?.headers ?? [],
  );
  const endHeaders = $derived(table.getEndHeaderGroups()[0]?.headers ?? []);

  /** Badges only earn their space once a second key exists. */
  const sortKeyCount = $derived(
    [...startHeaders, ...centerHeaders, ...endHeaders].filter(
      (h) => sortIndexOf(h.column.id) !== -1,
    ).length,
  );

  /** Column meta carries the display name and index; the id is positional. */
  function metaOf(header) {
    return header.column.columnDef.meta ?? {};
  }

  function widthOf(index) {
    return columnWidths[index] || 150;
  }

  function arrowFor(columnId) {
    const dir = sortDirectionOf(columnId);
    if (!dir) return '';
    return dir === 'asc' ? '▴' : '▾';
  }

  // Sticky insets: `position: sticky` never engages without a non-auto inset,
  // and getStart('start') measures only within the pinning region — it cannot
  // see the gutter th that renders before every start-group cell.
  function startLeftOffset(header) {
    return GUTTER_WIDTH + header.column.getStart('start');
  }

  /** Cumulative width of end-group columns rendered to this column's right. */
  function endRightOffset(header) {
    return header.column.getAfter('end');
  }
</script>

<thead>
  <tr>
    <th class="row-num hdr-start" style="left: 0;">
      <input
        type="checkbox"
        onchange={onToggleSelectAllPage}
        checked={allPageSelected}
        aria-label="Select all rows on this page"
      />
    </th>
    {#each startHeaders as header, i (header.id)}
      {@render headerCell(
        header,
        i === startHeaders.length - 1 ? 'hdr-start pinned-edge' : 'hdr-start',
      )}
    {/each}
    {#each centerHeaders as header (header.id)}
      {@render headerCell(header, 'hdr-center')}
    {/each}
    {#each endHeaders as header, i (header.id)}
      {@render headerCell(header, i === 0 ? 'hdr-end pinned-edge' : 'hdr-end')}
    {/each}
  </tr>
</thead>

{#snippet headerCell(header, extraClass)}
  {@const meta = metaOf(header)}
  <th
    class="sortable {extraClass}"
    class:active={sortDirectionOf(header.column.id) !== false}
    style="width: {widthOf(
      meta.index,
    )}px; min-width: 80px; {extraClass.includes('hdr-start')
      ? `left: ${startLeftOffset(header)}px;`
      : ''}{extraClass.includes('hdr-end')
      ? `right: ${endRightOffset(header)}px;`
      : ''}"
    title={meta.typeName}
  >
    <div class="col-header">
      <button
        class="col-info"
        aria-label="Sort by {meta.name}. Shift-click to add an additional sort key."
        onclick={(e) => onToggleSort(header.column.id, e)}
      >
        <span class="col-name-and-order">
          <span class="col-name">{meta.name}</span>
          {#if sortDirectionOf(header.column.id)}
            <span class="sort-arrow" aria-hidden="true"
              >{arrowFor(header.column.id)}</span
            >
            {#if sortKeyCount > 1}
              <span class="sort-badge">{sortIndexOf(header.column.id) + 1}</span
              >
            {/if}
          {/if}
        </span>
        <span class="col-type">{meta.typeName}</span>
      </button>
      <button
        class="col-menu-trigger"
        aria-label="Column actions for {meta.name}"
        aria-haspopup="menu"
        aria-expanded={openColumnId === header.column.id}
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
{/snippet}

<style>
  /* Sticky header */
  thead {
    position: sticky;
    top: 0;
    z-index: 2;
  }
  thead tr:first-child th {
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--grid-header-border);
  }
  th {
    text-align: left;
    padding: 3px 8px;
    height: 30px;
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--grid-header-border);
    border-right: 1px solid var(--grid-line);
    font-weight: var(--weight-medium);
    font-size: var(--text-xs);
    color: var(--text-secondary);
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
    font-weight: var(--weight-medium);
    font-size: var(--text-sm);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    width: 100%;
  }
  .col-name-and-order {
    display: flex;
    align-items: baseline;
    gap: 3px;
    min-width: 0;
  }
  .sort-arrow {
    font-size: 0.7rem;
    opacity: 0.8;
  }

  .sort-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1rem;
    height: 1rem;
    padding: 0 0.2rem;
    border-radius: 0.5rem;
    font-size: 0.625rem;
    font-variant-numeric: tabular-nums;
    background: var(--accent-soft, rgba(120, 140, 255, 0.18));
    color: var(--accent, #6b7cff);
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
    width: 34px;
    text-align: center;
    padding: 3px 4px;
    border-right: 1px solid var(--grid-line);
  }
  th.row-num input {
    cursor: pointer;
  }
  th input[type='checkbox'] {
    width: 14px;
    height: 14px;
    accent-color: var(--accent);
    cursor: pointer;
  }

  /* Resize handle — positioned relative to th so it sits exactly on the column border */
  th.sortable {
    position: relative;
  }

  /* Pinned columns stick inside the scroller. This overrides the relative
     positioning above for start/end cells (sticky elements are still
     "positioned", so the resize handle keeps anchoring to them). */
  th.hdr-start {
    position: sticky;
    z-index: 2;
  }

  th.hdr-end {
    position: sticky;
    right: 0;
    z-index: 2;
  }

  /* Shadow divider on the inner edge of each pinned region. */
  th.pinned-edge {
    box-shadow: 2px 0 6px -2px rgba(0, 0, 0, 0.55);
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
  /* Revealed on hover, but never hidden from keyboard users or while its menu
     is open — an invisible trigger that still takes focus is a trap. */
  th:hover .col-menu-trigger,
  .col-menu-trigger:focus-visible,
  .col-menu-trigger[aria-expanded='true'] {
    opacity: 1;
  }
</style>
