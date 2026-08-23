<script>
  import { untrack } from 'svelte';
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
  let copied = $state(false);
  let addBtnEl = $state(null);
  let anchorRect = $state(null);

  function typeNameFor(column) {
    return columns.find((c) => c.name === column)?.type_name || 'text';
  }

  // Only complete filters reach the backend, so only they belong in the preview.
  let specs = $derived(
    applyable(filters).map((f) => ({
      column: f.column,
      operator: f.operator,
      value: needsValue(f.operator) ? f.value : null,
    })),
  );
  let specsKey = $derived(JSON.stringify(specs));

  // Keep the preview honest. It previously refreshed only when the panel
  // opened, so editing a chip left stale SQL on screen that no longer matched
  // the query being run — the exact failure the shared Rust builder exists to
  // prevent.
  let queriedKey = null;

  $effect(() => {
    if (!sqlOpen || !onDescribeFilters) return;
    const key = specsKey;
    // Every keystroke in a pending chip re-runs this effect, but a pending
    // filter is not in the query, so the SQL cannot have changed. Skipping the
    // round trip keeps typing free of IPC traffic.
    if (key === queriedKey) return;
    queriedKey = key;

    const payload = untrack(() => specs);
    let stale = false;
    onDescribeFilters(payload)
      .then((text) => {
        if (!stale) sqlText = text;
      })
      .catch(() => {
        if (!stale) sqlText = '';
      });
    // Guards against an out-of-order response overwriting a newer one.
    return () => {
      stale = true;
    };
  });

  function emit(next, commit) {
    onFiltersChange?.(next, { commit });
  }

  function openPicker() {
    if (pickerOpen) {
      onPickerOpenChange?.(false);
      return;
    }
    anchorRect = addBtnEl?.getBoundingClientRect() ?? null;
    onPickerOpenChange?.(true);
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

  async function copySql() {
    try {
      await navigator.clipboard.writeText(sqlText);
      copied = true;
      setTimeout(() => (copied = false), 1400);
    } catch {
      // Clipboard denied — the text is on screen and selectable anyway.
    }
  }
</script>

<div class="filter-bar" class:compact>
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

  <div class="add-wrap">
    <button
      bind:this={addBtnEl}
      class="bar-btn"
      aria-haspopup="listbox"
      aria-expanded={pickerOpen}
      onclick={openPicker}
    >
      <svg
        width="11"
        height="11"
        viewBox="0 0 12 12"
        fill="none"
        stroke="currentColor"
        stroke-width="1.75"
        stroke-linecap="round"
        aria-hidden="true"
      >
        <path d="M6 2v8M2 6h8" />
      </svg>
      Add filter
    </button>
    {#if pickerOpen}
      <ColumnPicker
        {columns}
        {anchorRect}
        onPick={handlePick}
        onClose={() => onPickerOpenChange?.(false)}
      />
    {/if}
  </div>

  {#if filters.length > 0}
    <div class="bar-trailing">
      {#if onDescribeFilters}
        <button
          class="bar-btn ghost"
          class:active={sqlOpen}
          aria-expanded={sqlOpen}
          onclick={() => (sqlOpen = !sqlOpen)}
        >
          SQL
        </button>
      {/if}
      <button class="bar-btn ghost" onclick={handleClearAll}>Clear all</button>
    </div>
  {/if}

  {#if sqlOpen}
    <div class="sql-preview">
      <code>{sqlText || 'No filters applied'}</code>
      <button
        class="bar-btn ghost copy-btn"
        disabled={!sqlText}
        onclick={copySql}
      >
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  {/if}
</div>

<style>
  .filter-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    /* Generous vertical padding: chips are pill-shaped and crowd a tight bar. */
    padding: 7px var(--space-4);
    /* Many filters wrap and scroll the bar rather than squeezing the grid. */
    max-height: 30vh;
    overflow-y: auto;
    border-bottom: 1px solid var(--border);
    background: var(--bg-subtle);
  }
  .filter-bar.compact {
    padding: 5px var(--space-2);
  }
  .conjunction {
    color: var(--text-muted);
    font-size: var(--text-xs);
    /* Binds the two chips it sits between more tightly than the 6px flex gap. */
    margin: 0 -1px;
  }
  .add-wrap {
    position: relative;
    display: inline-flex;
  }
  /* Pushes SQL and Clear all to the far edge, away from the chips they act on. */
  .bar-trailing {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    margin-left: auto;
  }
  .bar-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-full);
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-size: var(--text-sm);
    white-space: nowrap;
    cursor: pointer;
    transition:
      border-color var(--transition-fast),
      color var(--transition-fast),
      background var(--transition-fast);
  }
  .bar-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .bar-btn.ghost {
    border-color: transparent;
    background: transparent;
  }
  .bar-btn.ghost:hover:not(:disabled) {
    border-color: transparent;
    background: var(--bg-hover);
  }
  .bar-btn.ghost.active {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .bar-btn:disabled {
    color: var(--text-muted);
    cursor: default;
  }
  .sql-preview {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-basis: 100%;
    padding: 6px var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-surface);
  }
  .sql-preview code {
    flex: 1;
    overflow-x: auto;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.6;
    color: var(--text);
    white-space: nowrap;
    /* Selectable as a fallback when the clipboard is unavailable. */
    user-select: text;
  }
  .copy-btn {
    flex-shrink: 0;
  }
</style>
