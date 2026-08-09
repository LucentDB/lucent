<script>
  // Searchable column list for "Add filter".
  //
  // position: fixed, anchored off a caller-supplied rect. It cannot be
  // absolutely positioned: .results-grid sets overflow: hidden, which clipped
  // this dropdown when it opened downward into the table.
  import { onMount } from 'svelte';
  import { anchorPosition, availableHeight } from './anchor.js';

  let {
    columns = [],
    anchorRect = null,
    onPick = null,
    onClose = null,
  } = $props();

  const WIDTH = 264;

  let query = $state('');
  let inputEl = $state(null);
  let listEl = $state(null);
  let rootEl = $state(null);
  let activeIndex = $state(0);
  let pos = $state({ x: 0, y: 0 });
  let maxListHeight = $state(240);
  let placed = $state(false);
  let optionEls = {};

  let matches = $derived(
    columns.filter((c) =>
      c.name.toLowerCase().includes(query.trim().toLowerCase()),
    ),
  );

  // A narrowed list can be shorter than the previous active index.
  let active = $derived(
    matches.length === 0
      ? null
      : matches[Math.min(activeIndex, matches.length - 1)],
  );

  onMount(() => {
    place();
    inputEl?.focus();
  });

  function place() {
    const rect = anchorRect ??
      rootEl?.parentElement?.getBoundingClientRect() ?? {
        top: 0,
        bottom: 0,
        left: 0,
      };
    const viewport = { width: window.innerWidth, height: window.innerHeight };
    const estimated = Math.min(240, Math.max(80, columns.length * 28)) + 52;
    pos = anchorPosition(
      { top: rect.top, bottom: rect.bottom, left: rect.left },
      { width: WIDTH, height: estimated },
      viewport,
    );
    // Let the list shrink rather than run off-screen on a short window.
    maxListHeight = Math.min(
      240,
      Math.max(96, availableHeight(pos.y, viewport) - 52),
    );
    placed = true;
  }

  function pick(name) {
    onPick?.(name);
    onClose?.();
  }

  // scrollIntoView is absent in some non-browser DOM implementations; the
  // optional call keeps keyboard navigation working where it isn't available.
  function reveal(index) {
    optionEls[index]?.scrollIntoView?.({ block: 'nearest' });
  }

  function move(delta) {
    if (matches.length === 0) return;
    activeIndex =
      (Math.min(activeIndex, matches.length - 1) + delta + matches.length) %
      matches.length;
    reveal(activeIndex);
  }

  function handleKeydown(e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose?.();
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      move(1);
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      move(-1);
      return;
    }
    if (e.key === 'Home') {
      e.preventDefault();
      activeIndex = 0;
      reveal(0);
      return;
    }
    if (e.key === 'End') {
      e.preventDefault();
      activeIndex = matches.length - 1;
      reveal(activeIndex);
      return;
    }
    if (e.key === 'Enter' && active) {
      e.preventDefault();
      pick(active.name);
    }
  }

  function handlePointerDown(e) {
    if (rootEl && !rootEl.contains(e.target)) onClose?.();
  }

  function handleInput() {
    activeIndex = 0;
  }
</script>

<svelte:window onpointerdown={handlePointerDown} onresize={() => onClose?.()} />

<div
  bind:this={rootEl}
  class="column-picker"
  class:placed
  style="left: {pos.x}px; top: {pos.y}px; width: {WIDTH}px"
>
  <input
    bind:this={inputEl}
    class="picker-search"
    type="text"
    role="combobox"
    aria-expanded="true"
    aria-controls="column-picker-list"
    aria-activedescendant={active ? `column-option-${active.name}` : undefined}
    aria-label="Search columns to filter"
    placeholder="Search columns…"
    bind:value={query}
    oninput={handleInput}
    onkeydown={handleKeydown}
  />
  <div
    bind:this={listEl}
    id="column-picker-list"
    class="picker-list"
    role="listbox"
    aria-label="Columns"
    style="max-height: {maxListHeight}px"
  >
    {#each matches as col, i (col.name)}
      <button
        bind:this={optionEls[i]}
        id="column-option-{col.name}"
        class="picker-item"
        class:active={active?.name === col.name}
        role="option"
        aria-selected={active?.name === col.name}
        tabindex="-1"
        onclick={() => pick(col.name)}
        onmousemove={() => (activeIndex = i)}
      >
        <span class="picker-name">{col.name}</span>
        <span class="picker-type">{col.type_name}</span>
      </button>
    {/each}
    {#if matches.length === 0}
      <div class="picker-empty">No matching columns</div>
    {/if}
  </div>
  <div class="picker-hint" aria-live="polite">
    {#if matches.length === 0}
      Nothing matches “{query.trim()}”
    {:else}
      {matches.length} column{matches.length === 1 ? '' : 's'} · ↑↓ to move · Enter
      to add
    {/if}
  </div>
</div>

<style>
  .column-picker {
    position: fixed;
    z-index: 100;
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    box-shadow: var(--shadow-lg);
    visibility: hidden;
  }
  .column-picker.placed {
    visibility: visible;
    animation: picker-in 0.12s cubic-bezier(0.22, 1, 0.36, 1);
  }
  @keyframes picker-in {
    from {
      opacity: 0;
      transform: translateY(-3px) scale(0.99);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  .picker-search {
    width: 100%;
    padding: 6px var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    color: var(--text);
    font-size: var(--text-sm);
    outline: none;
  }
  .picker-search:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .picker-list {
    margin-top: var(--space-1);
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .picker-item {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
    width: 100%;
    padding: 5px var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    cursor: pointer;
    text-align: left;
  }
  .picker-item.active {
    background: var(--accent-soft);
  }
  .picker-item.active .picker-name {
    color: var(--accent);
    font-weight: var(--weight-medium);
  }
  .picker-name {
    color: var(--text);
    font-size: var(--text-sm);
  }
  .picker-type {
    flex-shrink: 0;
    color: var(--text-muted);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }
  .picker-empty {
    padding: var(--space-2);
    color: var(--text-muted);
    font-size: var(--text-sm);
  }
  .picker-hint {
    padding: 5px var(--space-2) 2px;
    border-top: 1px solid var(--border-light);
    margin-top: var(--space-1);
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  @media (prefers-reduced-motion: reduce) {
    .column-picker.placed {
      animation: none;
    }
  }
</style>
