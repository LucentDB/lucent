<script lang="ts">
  import type { CellKind } from '../../stores/notebook.svelte.ts';

  let {
    onAdd,
  }: {
    onAdd?: (kind: CellKind) => void;
  } = $props();

  let open = $state(false);
  let btnEl: HTMLButtonElement;
  let popupStyle = $state('');

  function toggle() {
    if (!open && btnEl) {
      const rect = btnEl.getBoundingClientRect();
      // Position above if too close to bottom
      const spaceBelow = window.innerHeight - rect.bottom;
      const popupHeight = 140; // approximate
      if (spaceBelow < popupHeight) {
        popupStyle = `position: fixed; bottom: ${window.innerHeight - rect.top + 4}px; left: ${rect.left}px; z-index: 1000;`;
      } else {
        popupStyle = `position: fixed; top: ${rect.bottom + 4}px; left: ${rect.left}px; z-index: 1000;`;
      }
    }
    open = !open;
  }

  function add(kind: CellKind) {
    open = false;
    onAdd?.(kind);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') open = false;
  }
</script>

<div class="add-cell-button" class:open>
  <button
    class="add-btn"
    bind:this={btnEl}
    onclick={toggle}
    aria-label="Add cell"
    aria-expanded={open}
  >
    <span class="add-icon">+</span>
  </button>
  {#if open}
    <button
      class="add-backdrop"
      onclick={() => (open = false)}
      aria-label="Close cell type menu"
      tabindex="-1"
    ></button>
    <div
      class="add-popup"
      role="menu"
      style={popupStyle}
      onkeydown={handleKeydown}
    >
      <button class="add-option" role="menuitem" onclick={() => add('sql')}>
        <span class="option-icon">▶</span>
        <span class="option-label">SQL Cell</span>
      </button>
      <button
        class="add-option"
        role="menuitem"
        onclick={() => add('markdown')}
      >
        <span class="option-icon">M</span>
        <span class="option-label">Markdown Cell</span>
      </button>
      <button class="add-option" role="menuitem" onclick={() => add('ai')}>
        <span class="option-icon">✨</span>
        <span class="option-label">AI Cell</span>
      </button>
    </div>
  {/if}
</div>

<style>
  .add-cell-button {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    height: 10px;
    opacity: 0;
    transition:
      height 0.15s ease,
      opacity 0.15s ease;
  }
  .add-cell-button:hover,
  .add-cell-button:focus-within,
  .add-cell-button.open {
    height: 26px;
    opacity: 1;
  }
  /* A hairline that reads as an insertion point, rather than a floating circle
     repeated after every cell. */
  .add-cell-button::before {
    content: '';
    position: absolute;
    left: 44px;
    right: 12px;
    height: 1px;
    background: var(--accent);
    opacity: 0.35;
  }
  .add-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border: 1px solid var(--border);
    border-radius: 50%;
    background: var(--bg-surface);
    color: var(--text-muted);
    cursor: pointer;
    z-index: 1;
    transition: all 0.15s;
  }
  .add-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .add-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .add-icon {
    font-size: 13px;
    line-height: 1;
  }
  .add-backdrop {
    position: fixed;
    inset: 0;
    z-index: 999;
    border: none;
    background: transparent;
  }
  .add-popup {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-lg);
    min-width: 160px;
    padding: 4px;
  }
  .add-option {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 8px 12px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--text-sm);
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .add-option:hover {
    background: var(--bg-hover);
  }
  .option-icon {
    width: 20px;
    text-align: center;
    font-size: var(--text-sm);
  }
</style>
