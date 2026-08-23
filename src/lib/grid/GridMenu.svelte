<script>
  // Anchored popover menu. Shared by the column header menu and the cell
  // context menu, which differ only in their items.
  //
  // position: fixed is required, not stylistic — .results-grid sets
  // overflow: hidden, which would clip an absolutely-positioned child.
  import { onMount } from 'svelte';
  import { anchorPosition } from './anchor.js';

  let { x = 0, y = 0, items = [], onSelect = null, onClose = null } = $props();

  let focusIndex = $state(-1);
  let menuEl = $state(null);
  let placed = $state(false);
  // Seeded at the origin rather than at (x, y): the menu stays visibility:hidden
  // until place() measures it, so the initial value is never painted.
  let pos = $state({ x: 0, y: 0 });
  let itemEls = {};

  let selectable = $derived(
    items
      .map((item, i) => ({ item, i }))
      .filter(({ item }) => !item.separator && !item.disabled),
  );

  onMount(() => {
    // Return focus where the user left it, so dismissing a menu doesn't strand
    // keyboard users at the top of the document.
    const returnTo = document.activeElement;
    place();
    menuEl?.focus();
    return () => {
      if (returnTo instanceof HTMLElement && document.contains(returnTo)) {
        returnTo.focus();
      }
    };
  });

  function place() {
    if (!menuEl) return;
    const rect = menuEl.getBoundingClientRect();
    pos = anchorPosition(
      { top: y, bottom: y, left: x },
      { width: rect.width, height: rect.height },
      { width: window.innerWidth, height: window.innerHeight },
      { gap: 0 },
    );
    placed = true;
  }

  function choose(id) {
    onSelect?.(id);
    onClose?.();
  }

  function moveFocus(delta) {
    if (selectable.length === 0) return;
    const current = selectable.findIndex(({ i }) => i === focusIndex);
    const next =
      current === -1
        ? delta > 0
          ? 0
          : selectable.length - 1
        : (current + delta + selectable.length) % selectable.length;
    focusIndex = selectable[next].i;
    // Move real DOM focus, not just a highlight class, so screen readers
    // announce the item and Enter reaches it natively.
    itemEls[focusIndex]?.focus();
  }

  function handleKeydown(e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose?.();
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      moveFocus(1);
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      moveFocus(-1);
      return;
    }
    if (e.key === 'Home') {
      e.preventDefault();
      focusIndex = -1;
      moveFocus(1);
      return;
    }
    if (e.key === 'End') {
      e.preventDefault();
      focusIndex = -1;
      moveFocus(-1);
      return;
    }
    if (e.key === 'Enter' && focusIndex >= 0) {
      // When a menu item holds DOM focus, its native click already fires on
      // Enter — handling it here too would select twice.
      if (e.target instanceof HTMLElement && e.target.dataset.menuItem) return;
      e.preventDefault();
      const item = items[focusIndex];
      if (item && !item.separator && !item.disabled) choose(item.id);
    }
  }

  function handlePointerDown(e) {
    if (menuEl && !menuEl.contains(e.target)) onClose?.();
  }
</script>

<svelte:window
  onkeydown={handleKeydown}
  onpointerdown={handlePointerDown}
  onresize={() => onClose?.()}
/>

<div
  bind:this={menuEl}
  class="grid-menu"
  class:placed
  role="menu"
  tabindex="-1"
  style="left: {pos.x}px; top: {pos.y}px"
>
  {#each items as item, i}
    {#if item.separator}
      <div class="separator" role="separator"></div>
    {:else}
      <button
        bind:this={itemEls[i]}
        class="menu-item"
        class:focused={focusIndex === i}
        role="menuitem"
        data-menu-item="true"
        tabindex={focusIndex === i ? 0 : -1}
        disabled={item.disabled}
        onclick={() => {
          if (!item.disabled) choose(item.id);
        }}
        onmouseenter={() => (focusIndex = i)}
      >
        {item.label}
      </button>
    {/if}
  {/each}
</div>

<style>
  .grid-menu {
    position: fixed;
    z-index: 100;
    min-width: 208px;
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    box-shadow: var(--shadow-lg);
    /* Hidden until measured, so it never flashes at the unclamped position. */
    visibility: hidden;
  }
  .grid-menu.placed {
    visibility: visible;
    animation: menu-in 0.12s cubic-bezier(0.22, 1, 0.36, 1);
  }
  .grid-menu:focus-visible {
    outline: none;
  }
  @keyframes menu-in {
    from {
      opacity: 0;
      transform: translateY(-2px) scale(0.99);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  .menu-item {
    display: block;
    width: 100%;
    padding: 6px var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font-size: var(--text-sm);
    text-align: left;
    cursor: pointer;
  }
  .menu-item:hover:not(:disabled),
  .menu-item.focused:not(:disabled) {
    background: var(--bg-hover);
  }
  /* The container already reads as focused; a ring on each item as you arrow
     through it would strobe. */
  .menu-item:focus-visible {
    outline: none;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .menu-item:disabled {
    color: var(--text-muted);
    cursor: default;
  }
  .separator {
    height: 1px;
    margin: var(--space-1) 6px;
    background: var(--border);
  }

  @media (prefers-reduced-motion: reduce) {
    .grid-menu.placed {
      animation: none;
    }
  }
</style>
