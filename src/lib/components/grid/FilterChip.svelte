<script>
  import { onDestroy } from 'svelte';
  import {
    operatorsFor,
    needsValue,
    isComplete,
    valuePlaceholderFor,
  } from './filters.js';

  let {
    filter,
    typeName = 'text',
    autofocus = false,
    onChange = null,
    onCommit = null,
    onRemove = null,
  } = $props();

  // Value typing debounces so a multi-character filter costs one query, not
  // one per keystroke. Each chip owns its own timer.
  const DEBOUNCE_MS = 275;
  let timer = null;
  // The last value the parent applied, so Escape can revert to it.
  let committedValue = filter.value;
  let inputEl = $state(null);

  onDestroy(() => clearTimeout(timer));

  $effect(() => {
    if (autofocus) inputEl?.focus();
  });

  function commitNow() {
    clearTimeout(timer);
    timer = null;
    committedValue = filter.value;
    onCommit?.();
  }

  function handleValueInput(e) {
    onChange?.({ value: e.target.value });
    clearTimeout(timer);
    timer = setTimeout(commitNow, DEBOUNCE_MS);
  }

  function handleOperatorChange(e) {
    onChange?.({ operator: e.target.value });
    commitNow();
  }

  function handleKeydown(e) {
    if (e.key === 'Enter') {
      e.preventDefault();
      commitNow();
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      // An empty chip on Escape means "I didn't mean to add this".
      if (!committedValue) {
        clearTimeout(timer);
        onRemove?.();
        return;
      }
      clearTimeout(timer);
      timer = null;
      onChange?.({ value: committedValue });
      inputEl?.blur();
    }
  }

  function handleRemove() {
    clearTimeout(timer);
    onRemove?.();
  }
</script>

<div class="filter-chip" class:pending={!isComplete(filter)}>
  <span class="chip-column">{filter.column}</span>
  <select
    class="chip-operator"
    aria-label="Filter operator for {filter.column}"
    value={filter.operator}
    onchange={handleOperatorChange}
  >
    {#each operatorsFor(typeName) as op}
      <option value={op.value}>{op.label}</option>
    {/each}
  </select>
  {#if needsValue(filter.operator)}
    <input
      bind:this={inputEl}
      class="chip-value"
      type="text"
      aria-label="Filter value for {filter.column}"
      placeholder={valuePlaceholderFor(typeName)}
      value={filter.value ?? ''}
      oninput={handleValueInput}
      onkeydown={handleKeydown}
    />
  {/if}
  <button
    class="chip-remove"
    aria-label="Remove filter on {filter.column}"
    onclick={handleRemove}>×</button
  >
</div>

<style>
  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 4px 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-full);
    background: var(--bg-surface);
    font-size: var(--text-sm);
  }
  .filter-chip.pending {
    border-style: dashed;
    border-color: var(--text-muted);
  }
  .chip-column {
    color: var(--text);
    font-weight: var(--weight-medium);
  }
  .chip-operator {
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    cursor: pointer;
    padding: 2px;
  }
  .chip-value {
    width: 90px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--text-sm);
    outline: none;
  }
  .chip-value::placeholder {
    color: var(--text-muted);
  }
  .chip-remove {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border: none;
    border-radius: var(--radius-full);
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-base);
    line-height: 1;
    cursor: pointer;
  }
  .chip-remove:hover {
    background: var(--danger-bg);
    color: var(--danger);
  }
</style>
