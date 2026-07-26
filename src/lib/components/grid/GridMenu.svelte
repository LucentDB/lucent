<script>
  // Anchored popover menu. Shared by the column header menu and the cell
  // context menu, which differ only in their items.
  let { x = 0, y = 0, items = [], onSelect = null, onClose = null } = $props();

  let focusIndex = $state(-1);
  let menuEl = $state(null);

  let selectable = $derived(
    items
      .map((item, i) => ({ item, i }))
      .filter(({ item }) => !item.separator && !item.disabled),
  );

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
    if (e.key === 'Enter' && focusIndex >= 0) {
      e.preventDefault();
      const item = items[focusIndex];
      if (item && !item.separator && !item.disabled) choose(item.id);
    }
  }

  function handlePointerDown(e) {
    if (menuEl && !menuEl.contains(e.target)) onClose?.();
  }
</script>

<svelte:window onkeydown={handleKeydown} onpointerdown={handlePointerDown} />

<div
  bind:this={menuEl}
  class="grid-menu"
  role="menu"
  tabindex="-1"
  style="left: {x}px; top: {y}px"
>
  {#each items as item, i}
    {#if item.separator}
      <div class="separator" role="separator"></div>
    {:else}
      <button
        class="menu-item"
        class:focused={focusIndex === i}
        role="menuitem"
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
    min-width: 200px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
  }
  .menu-item {
    display: block;
    width: 100%;
    padding: 6px 10px;
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
  .menu-item:disabled {
    color: var(--text-muted);
    cursor: default;
  }
  .separator {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
</style>
