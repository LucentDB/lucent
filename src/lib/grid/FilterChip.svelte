<script>
  import { onDestroy } from 'svelte';
  import GridMenu from './GridMenu.svelte';
  import {
    operatorsFor,
    operatorLabel,
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
  // one per keystroke. Each chip owns its own timer, so typing in one chip
  // never cancels another's pending commit.
  const DEBOUNCE_MS = 275;
  const MIN_CH = 5;
  const MAX_CH = 24;

  let timer = null;
  // The last value the parent applied, so Escape can revert to it.
  let committedValue = $state(null);
  let inputEl = $state(null);
  let operatorEl = $state(null);
  let menuPos = $state(null);

  // Seed from the incoming filter without making it a reactive dependency:
  // re-seeding on every parent update would defeat Escape-to-revert.
  $effect(() => {
    if (committedValue === null) committedValue = filter.value ?? '';
  });

  onDestroy(() => clearTimeout(timer));

  $effect(() => {
    if (autofocus) inputEl?.focus();
  });

  let applied = $derived(isComplete(filter));

  // Size the input to its content so a long value is readable and a short one
  // doesn't leave dead space. A fixed width silently truncated longer values.
  let valueWidth = $derived(
    Math.min(MAX_CH, Math.max(MIN_CH, (filter.value ?? '').length + 1)),
  );

  let operatorItems = $derived(
    operatorsFor(typeName).map((op) => ({ id: op.value, label: op.label })),
  );

  function commitNow() {
    clearTimeout(timer);
    timer = null;
    committedValue = filter.value ?? '';
    onCommit?.();
  }

  function handleValueInput(e) {
    onChange?.({ value: e.target.value });
    clearTimeout(timer);
    timer = setTimeout(commitNow, DEBOUNCE_MS);
  }

  function openOperatorMenu() {
    if (menuPos) {
      menuPos = null;
      return;
    }
    const r = operatorEl.getBoundingClientRect();
    menuPos = { x: r.left, y: r.bottom + 4 };
  }

  function handleOperatorSelect(operator) {
    onChange?.({ operator });
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

<div class="filter-chip" class:pending={!applied}>
  <span class="chip-column">{filter.column}</span>

  <button
    bind:this={operatorEl}
    class="chip-operator"
    aria-label="Filter operator for {filter.column}"
    aria-haspopup="menu"
    aria-expanded={menuPos !== null}
    onclick={openOperatorMenu}
  >
    {operatorLabel(filter.operator, typeName)}
    <svg
      class="chip-caret"
      width="8"
      height="8"
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

  {#if menuPos}
    <GridMenu
      x={menuPos.x}
      y={menuPos.y}
      items={operatorItems}
      onSelect={handleOperatorSelect}
      onClose={() => (menuPos = null)}
    />
  {/if}

  {#if needsValue(filter.operator)}
    <input
      bind:this={inputEl}
      class="chip-value"
      type="text"
      aria-label="Filter value for {filter.column}"
      placeholder={valuePlaceholderFor(typeName)}
      value={filter.value ?? ''}
      style="width: {valueWidth}ch"
      spellcheck="false"
      autocomplete="off"
      oninput={handleValueInput}
      onkeydown={handleKeydown}
    />
  {/if}

  <button
    class="chip-remove"
    aria-label="Remove filter on {filter.column}"
    onclick={handleRemove}
  >
    <svg
      width="10"
      height="10"
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      stroke-width="1.75"
      stroke-linecap="round"
      aria-hidden="true"
    >
      <path d="M3 3l6 6M9 3l-6 6" />
    </svg>
  </button>
</div>

<style>
  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    height: 24px;
    padding: 0 3px 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    font-size: var(--text-sm);
    animation: chip-in 0.14s cubic-bezier(0.22, 1, 0.36, 1);
  }
  @keyframes chip-in {
    from {
      opacity: 0;
      transform: scale(0.94);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  /* Incomplete: dashed and untinted, so "not yet filtering" is legible at a
     glance without reading the value. */
  .filter-chip.pending {
    border-style: dashed;
    border-color: var(--text-muted);
    background: transparent;
  }
  .chip-column {
    color: var(--text);
    font-weight: var(--weight-medium);
    white-space: nowrap;
  }
  .chip-operator {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px 4px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--accent);
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    white-space: nowrap;
    cursor: pointer;
    transition: background var(--transition-fast);
  }
  .chip-operator:hover {
    background: var(--bg-hover);
  }
  .filter-chip.pending .chip-operator {
    color: var(--text-secondary);
  }
  .chip-caret {
    flex-shrink: 0;
    opacity: 0.7;
  }
  .chip-value {
    min-width: 5ch;
    max-width: 24ch;
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
    cursor: pointer;
    transition:
      background var(--transition-fast),
      color var(--transition-fast);
  }
  .chip-remove:hover {
    background: var(--danger-bg);
    color: var(--danger);
  }

  @media (prefers-reduced-motion: reduce) {
    .filter-chip {
      animation: none;
    }
  }
</style>
