<script lang="ts">
  // Shared context menu for tab strips. AppHeader builds the items for DB
  // tabs and ChatPanel builds them for chat tabs, so both expose the same
  // close actions with identical visuals.
  import type { TabMenuAction, TabMenuItem } from './tab-menu.ts';

  let {
    x = 0,
    y = 0,
    items,
    onClose,
  }: {
    x?: number;
    y?: number;
    items: TabMenuItem[];
    onClose: () => void;
  } = $props();

  const MARGIN = 8;
  let menuEl = $state<HTMLDivElement | null>(null);
  let pos = $state({ x: 0, y: 0 });
  let placed = $state(false);

  $effect(() => {
    const returnTo = document.activeElement;
    return () => {
      if (returnTo instanceof HTMLElement && document.contains(returnTo)) {
        returnTo.focus();
      }
    };
  });

  // Runs after mount, and again if the anchor coordinates change while this
  // instance is alive (a second right-click can update the same instance).
  $effect(() => {
    void x;
    void y;
    place();
  });

  /** Clamp the menu into the viewport so a tab at the window edge still
   *  opens a fully reachable menu. `position: fixed` coordinates. */
  function place() {
    if (!menuEl) return;
    const rect = menuEl.getBoundingClientRect();
    pos = {
      x: Math.max(MARGIN, Math.min(x, window.innerWidth - rect.width - MARGIN)),
      y: Math.max(
        MARGIN,
        Math.min(y, window.innerHeight - rect.height - MARGIN),
      ),
    };
    placed = true;
  }

  function choose(item: TabMenuAction) {
    if (item.disabled) return;
    item.action();
    onClose();
  }

  function getEnabledItems(): HTMLButtonElement[] {
    if (!menuEl) return [];
    return Array.from(
      menuEl.querySelectorAll<HTMLButtonElement>('.menu-item:not(:disabled)'),
    );
  }

  function moveFocus(delta: number) {
    const buttons = getEnabledItems();
    if (buttons.length === 0) return;
    const current = buttons.indexOf(
      document.activeElement as HTMLButtonElement,
    );
    const next =
      current === -1
        ? delta > 0
          ? 0
          : buttons.length - 1
        : (current + delta + buttons.length) % buttons.length;
    buttons[next]?.focus();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
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
      getEnabledItems()[0]?.focus();
      return;
    }
    if (e.key === 'End') {
      e.preventDefault();
      const buttons = getEnabledItems();
      buttons[buttons.length - 1]?.focus();
      return;
    }
  }

  function handlePointerDown(e: PointerEvent) {
    if (menuEl && e.target instanceof Node && !menuEl.contains(e.target)) {
      onClose();
    }
  }
</script>

<svelte:window
  onkeydown={handleKeydown}
  onpointerdown={handlePointerDown}
  onresize={onClose}
/>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  bind:this={menuEl}
  class="context-menu"
  class:placed
  role="menu"
  tabindex="-1"
  style="left:{pos.x}px; top:{pos.y}px"
  onclick={(e) => e.stopPropagation()}
>
  {#each items as item, i (i)}
    {#if 'separator' in item}
      <div class="menu-separator" role="separator"></div>
    {:else}
      <button
        class="menu-item"
        role="menuitem"
        disabled={item.disabled}
        onclick={() => choose(item)}
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
          aria-hidden="true"
        >
          {#if item.icon === 'save'}
            <path
              d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"
            />
            <polyline points="17 21 17 13 7 13 7 21" />
            <polyline points="7 3 7 8 15 8" />
          {:else if item.icon === 'save-as'}
            <path
              d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"
            />
            <line x1="12" y1="11" x2="12" y2="17" />
            <line x1="9" y1="14" x2="15" y2="14" />
          {:else if item.icon === 'open-notebook'}
            <path
              d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"
            />
          {:else if item.icon === 'close'}
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          {:else if item.icon === 'close-others'}
            <rect x="3" y="3" width="18" height="18" rx="2" />
            <line x1="9" y1="3" x2="9" y2="21" />
            <line x1="15" y1="3" x2="15" y2="21" />
          {:else if item.icon === 'close-right'}
            <line x1="3" y1="12" x2="21" y2="12" />
            <polyline points="15 6 21 12 15 18" />
          {:else if item.icon === 'close-left'}
            <line x1="3" y1="12" x2="21" y2="12" />
            <polyline points="9 6 3 12 9 18" />
          {:else if item.icon === 'close-all'}
            <rect x="3" y="3" width="18" height="18" rx="2" />
            <path d="M9 3v18" />
            <path d="M15 3v18" />
          {/if}
        </svg>
        {item.label}
        {#if item.shortcut}
          <span class="menu-shortcut">{item.shortcut}</span>
        {/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .context-menu {
    position: fixed;
    z-index: 9999;
    min-width: 200px;
    background: rgba(255, 255, 255, 0.96);
    backdrop-filter: blur(20px) saturate(180%);
    -webkit-backdrop-filter: blur(20px) saturate(180%);
    border: 1px solid rgba(0, 0, 0, 0.08);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-float);
    padding: 5px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    /* Hidden until measured, so the menu never flashes at the unclamped
       position when the cursor is near a viewport edge. */
    visibility: hidden;
    animation: menu-in 0.1s ease-out;
  }
  .context-menu.placed {
    visibility: visible;
  }
  @keyframes menu-in {
    from {
      opacity: 0;
      transform: scale(0.95);
    }
    to {
      opacity: 1;
      transform: scale(1);
    }
  }
  :global(.dark) .context-menu {
    background: rgba(22, 22, 30, 0.96);
    border-color: rgba(255, 255, 255, 0.08);
    box-shadow: var(--shadow-float);
  }
  .menu-shortcut {
    margin-left: auto;
    padding-left: 16px;
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }
  .menu-separator {
    height: 1px;
    background: var(--border);
    margin: 4px 8px;
  }
  .menu-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px;
    font-size: 13px;
    color: var(--text);
    background: transparent;
    border: none;
    border-radius: var(--radius-md);
    cursor: pointer;
    text-align: left;
    white-space: nowrap;
    user-select: none;
    transition: background var(--transition-fast);
  }
  .menu-item:hover:not(:disabled),
  .menu-item:focus-visible:not(:disabled) {
    background: var(--accent-soft);
    color: var(--accent);
    outline: none;
  }
  .menu-item:disabled {
    color: var(--text-muted);
    cursor: default;
  }
  .menu-item svg {
    flex-shrink: 0;
    color: var(--text-muted);
  }
  .menu-item:hover:not(:disabled) svg,
  .menu-item:focus-visible:not(:disabled) svg {
    color: var(--accent);
  }
</style>
