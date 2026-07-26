<script>
  import FilterChip from './FilterChip.svelte';
  import ColumnPicker from './ColumnPicker.svelte';
  import {
    addFilter,
    updateFilter,
    removeFilter,
    applyable,
    isComplete,
    needsValue,
  } from './filters.js';

  let {
    columns = [],
    filters = [],
    pickerOpen = false,
    onFiltersChange = null,
    onPickerOpenChange = null,
    onDescribeFilters = null,
    compact = false,
  } = $props();

  // The chip to autofocus: the one just added by the picker.
  let focusId = $state(null);
  let sqlOpen = $state(false);
  let sqlText = $state('');

  function typeNameFor(column) {
    return columns.find((c) => c.name === column)?.type_name || 'text';
  }

  function emit(next, commit) {
    onFiltersChange?.(next, { commit });
  }

  function handlePick(column) {
    const next = addFilter(filters, column, typeNameFor(column));
    const added = next[next.length - 1];
    focusId = added.id;
    // A filter needing a value is pending — no query until it has one.
    emit(next, isComplete(added));
  }

  function handleChipChange(id, patch) {
    emit(updateFilter(filters, id, patch), false);
  }

  function handleChipCommit(id) {
    emit(updateFilter(filters, id, {}), true);
  }

  function handleChipRemove(id) {
    if (focusId === id) focusId = null;
    emit(removeFilter(filters, id), true);
  }

  function handleClearAll() {
    focusId = null;
    emit([], true);
  }

  async function toggleSql() {
    sqlOpen = !sqlOpen;
    if (sqlOpen) await refreshSql();
  }

  async function refreshSql() {
    if (!onDescribeFilters) return;
    const specs = applyable(filters).map((f) => ({
      column: f.column,
      operator: f.operator,
      value: needsValue(f.operator) ? f.value : null,
    }));
    try {
      sqlText = await onDescribeFilters(specs);
    } catch {
      sqlText = '';
    }
  }

  async function copySql() {
    try {
      await navigator.clipboard.writeText(sqlText);
    } catch {
      // Clipboard denied — nothing useful to do, the text is on screen.
    }
  }
</script>

<div class="filter-bar" class:compact>
  <div class="chip-strip">
    {#each filters as filter, i (filter.id)}
      {#if i > 0}<span class="conjunction">and</span>{/if}
      <FilterChip
        {filter}
        typeName={typeNameFor(filter.column)}
        autofocus={filter.id === focusId}
        onChange={(patch) => handleChipChange(filter.id, patch)}
        onCommit={() => handleChipCommit(filter.id)}
        onRemove={() => handleChipRemove(filter.id)}
      />
    {/each}
  </div>

  <div class="bar-actions">
    <div class="add-wrap">
      <button class="bar-btn" onclick={() => onPickerOpenChange?.(!pickerOpen)}>
        <span aria-hidden="true">+</span> Add filter
      </button>
      {#if pickerOpen}
        <ColumnPicker
          {columns}
          onPick={handlePick}
          onClose={() => onPickerOpenChange?.(false)}
        />
      {/if}
    </div>

    {#if onDescribeFilters && filters.length > 0}
      <button class="bar-btn ghost" class:active={sqlOpen} onclick={toggleSql}>
        SQL
      </button>
    {/if}

    {#if filters.length > 0}
      <button class="bar-btn ghost" onclick={handleClearAll}>Clear all</button>
    {/if}
  </div>

  {#if sqlOpen}
    <div class="sql-preview">
      <code>{sqlText || '(no filters applied)'}</code>
      <button class="bar-btn ghost" onclick={copySql}>Copy</button>
    </div>
  {/if}
</div>

<style>
  .filter-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--border);
    background: var(--bg-subtle);
  }
  .chip-strip {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    max-height: 96px;
    overflow-y: auto;
  }
  .conjunction {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
  .bar-actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .add-wrap {
    position: relative;
  }
  .bar-btn {
    padding: 3px 10px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-full);
    background: transparent;
    color: var(--text-secondary);
    font-size: var(--text-sm);
    cursor: pointer;
    transition: all var(--transition-fast);
  }
  .bar-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .bar-btn.ghost {
    border-style: none;
  }
  .bar-btn.ghost.active {
    color: var(--accent);
  }
  .sql-preview {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-basis: 100%;
    padding: 6px 8px;
    border-radius: var(--radius-sm);
    background: var(--bg-surface);
    overflow-x: auto;
  }
  .sql-preview code {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text);
    white-space: nowrap;
  }
  .filter-bar.compact {
    padding: var(--space-1) var(--space-2);
  }
</style>
