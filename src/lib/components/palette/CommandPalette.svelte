<script>
  import { onMount } from 'svelte';

  let { commands = [], onSelect, onClose } = $props();
  let query = $state('');
  let selectedIndex = $state(0);
  let inputEl;

  // ⚡ Bolt: Cache query string transformation outside the filter loop to prevent
  // repeated string allocations on every iteration, improving search performance.
  let filtered = $derived.by(() => {
    if (!query) return commands;
    const q = query.toLowerCase();
    return commands.filter(
      (c) =>
        c.label.toLowerCase().includes(q) ||
        (c.searchText || '').toLowerCase().includes(q),
    );
  });

  $effect(() => {
    selectedIndex = 0;
  });

  function handleKeydown(e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      selectedIndex = Math.min(selectedIndex + 1, filtered.length - 1);
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      selectedIndex = Math.max(selectedIndex - 1, 0);
    }
    if (e.key === 'Enter' && filtered[selectedIndex]) {
      e.preventDefault();
      onSelect(filtered[selectedIndex]);
    }
  }

  function handleItemClick(item) {
    onSelect(item);
  }

  onMount(() => {
    if (inputEl) inputEl.focus();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="overlay" onclick={onClose} onkeydown={handleKeydown}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="palette"
    onclick={(e) => e.stopPropagation()}
    onkeydown={handleKeydown}
  >
    <input
      bind:this={inputEl}
      type="text"
      bind:value={query}
      placeholder="Search commands or tables..."
      onkeydown={handleKeydown}
    />
    {#if filtered.length > 0}
      <div class="results">
        {#each filtered as item, i}
          <button
            class="item"
            class:selected={i === selectedIndex}
            onclick={() => handleItemClick(item)}
            onmouseenter={() => (selectedIndex = i)}
          >
            <span class="item-icon">{item.icon || '›'}</span>
            <div class="item-content">
              <span class="item-label">{item.label}</span>
              {#if item.description}
                <span class="item-desc">{item.description}</span>
              {/if}
            </div>
            {#if item.shortcut}
              <kbd>{item.shortcut}</kbd>
            {/if}
          </button>
        {/each}
      </div>
    {:else}
      <div class="empty">
        <span class="empty-icon">⌕</span>
        <span>No results for "{query}"</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 15vh;
    background: rgba(0, 0, 0, 0.4);
    backdrop-filter: blur(32px);
    -webkit-backdrop-filter: blur(32px);
  }
  .palette {
    width: 540px;
    max-width: 90vw;
    max-height: 60vh;
    background: rgba(255, 255, 255, 0.92);
    backdrop-filter: blur(32px);
    -webkit-backdrop-filter: blur(32px);
    border: 1px solid rgba(255, 255, 255, 0.3);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  :global(.dark) .palette {
    background: rgba(24, 24, 27, 0.92);
    border-color: rgba(255, 255, 255, 0.1);
  }
  input {
    width: 100%;
    padding: 14px 16px;
    border: none;
    border-bottom: 1px solid var(--border);
    background: transparent;
    font-size: 15px;
    color: var(--text);
    outline: none;
  }
  input::placeholder {
    color: var(--text-muted);
  }
  .results {
    flex: 1;
    overflow-y: auto;
    padding: 4px;
  }
  .item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .item.selected {
    background: var(--accent);
    color: #fff;
  }
  .item:hover:not(.selected) {
    background: var(--bg-hover);
  }
  .item-icon {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 16px;
    flex-shrink: 0;
  }
  .item.selected .item-icon {
    background: rgba(255, 255, 255, 0.2);
    color: #fff;
  }
  .item-content {
    flex: 1;
    display: flex;
    flex-direction: column;
  }
  .item-label {
    font-size: 14px;
    font-weight: 500;
  }
  .item-desc {
    font-size: 12px;
    color: var(--text-muted);
  }
  .item.selected .item-desc {
    color: rgba(255, 255, 255, 0.7);
  }
  kbd {
    font-size: var(--text-xs);
    color: var(--text-muted);
    background: var(--bg-subtle);
    padding: 2px 6px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border-light);
    font-family: var(--font-mono);
  }
  .item.selected kbd {
    color: rgba(255, 255, 255, 0.7);
    background: rgba(255, 255, 255, 0.1);
    border-color: rgba(255, 255, 255, 0.2);
  }
  .empty {
    padding: var(--space-6);
    text-align: center;
    color: var(--text-muted);
    font-size: var(--text-md);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
  }
  .empty-icon {
    font-size: var(--text-2xl);
    opacity: 0.5;
  }
</style>
