<script>
  import { onMount } from 'svelte';

  let { commands = [], onSelect, onClose } = $props();
  let query = $state('');
  let selectedIndex = $state(0);
  let inputEl;

  // Pre-compute lowercase values for commands to avoid allocations during filtering.
  // Using a parallel cache pattern to preserve the original object identity.
  let commandsCache = $derived(
    commands.map((c) => ({
      item: c,
      labelLower: c.label.toLowerCase(),
      searchLower: (c.searchText || '').toLowerCase(),
    })),
  );

  // Normalize the query once per change, not once per filtered command —
  // toLowerCase() inside the filter loop allocates per item.
  let queryLower = $derived(query.toLowerCase());

  let filtered = $derived(
    commandsCache
      .filter(
        (c) =>
          !query ||
          c.labelLower.includes(queryLower) ||
          c.searchLower.includes(queryLower),
      )
      .map((c) => c.item),
  );

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

  function iconSvg(id) {
    const icons = {
      terminal:
        '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>',
      notebook:
        '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H20v20H6.5a2.5 2.5 0 0 1 0-5H20"/></svg>',
      theme:
        '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/></svg>',
      sparkles:
        '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/><path d="M20 3v4"/><path d="M22 5h-4"/></svg>',
      unplug:
        '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m19 5 3-3"/><path d="m2 22 3-3"/><path d="M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4Z"/><path d="M7.5 13.5 10 11"/><path d="M10.5 16.5 13 14"/><path d="m12 6 6 6 2.3-2.3a2.4 2.4 0 0 0 0-3.4l-2.6-2.6a2.4 2.4 0 0 0-3.4 0Z"/></svg>',
    };
    return (
      icons[id] ||
      '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"/></svg>'
    );
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
    <div class="search-row">
      <svg
        class="search-prefix-icon"
        width="22"
        height="22"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2.5"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <circle cx="11" cy="11" r="8" /><path d="M21 21l-4.35-4.35" />
      </svg>
      <input
        bind:this={inputEl}
        type="text"
        bind:value={query}
        placeholder="Search commands or tables…"
        onkeydown={handleKeydown}
      />
    </div>
    {#if filtered.length > 0}
      <div class="results">
        {#each filtered as item, i}
          <button
            class="item"
            class:selected={i === selectedIndex}
            onclick={() => handleItemClick(item)}
            onmouseenter={() => (selectedIndex = i)}
          >
            <span class="item-icon">{@html iconSvg(item.icon)}</span>
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
        <span class="empty-icon">
          <svg
            width="32"
            height="32"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            ><circle cx="11" cy="11" r="8" /><path d="M21 21l-4.35-4.35" /></svg
          >
        </span>
        <span>No results for "{query}"</span>
      </div>
    {/if}
    <div class="footer">
      <div class="footer-item"><kbd>Tab</kbd> to navigate</div>
      <div class="footer-item"><kbd>Enter</kbd> to select</div>
      <div class="footer-item"><kbd>Esc</kbd> to close</div>
    </div>
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
    padding-top: 14vh;
    background: rgba(0, 0, 0, 0.35);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    animation: overlay-in 0.12s ease;
  }
  @keyframes overlay-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }
  .palette {
    width: 600px;
    max-width: 92vw;
    max-height: 70vh;
    background: rgba(255, 255, 255, 0.94);
    backdrop-filter: blur(40px) saturate(180%);
    -webkit-backdrop-filter: blur(40px) saturate(180%);
    border: 1px solid rgba(255, 255, 255, 0.5);
    border-radius: var(--radius-xl);
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.04),
      0 24px 48px -12px rgba(167, 139, 250, 0.15),
      var(--shadow-float);
    overflow: hidden;
    display: flex;
    flex-direction: column;
    animation: palette-in 0.14s cubic-bezier(0.16, 1, 0.3, 1);
  }
  @keyframes palette-in {
    from {
      opacity: 0;
      transform: scale(0.96) translateY(-8px);
    }
    to {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }
  :global(.dark) .palette {
    background: rgba(22, 22, 30, 0.94);
    border-color: rgba(255, 255, 255, 0.08);
    box-shadow:
      0 0 0 1px rgba(255, 255, 255, 0.08),
      0 24px 48px -12px rgba(167, 139, 250, 0.15),
      var(--shadow-float);
  }
  .search-row {
    position: relative;
    display: flex;
    align-items: center;
    border-bottom: 1px solid var(--border-light);
  }
  .search-prefix-icon {
    position: absolute;
    left: 20px;
    color: var(--accent);
    pointer-events: none;
    flex-shrink: 0;
  }
  input {
    width: 100%;
    padding: 20px 20px 20px 56px;
    border: none;
    background: transparent;
    font-size: 18px;
    font-weight: 500;
    color: var(--text);
    outline: none;
    letter-spacing: -0.015em;
  }
  input::placeholder {
    color: var(--text-muted);
    font-weight: 400;
  }
  .results {
    flex: 1;
    overflow-y: auto;
    padding: 12px;
  }
  .item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 12px 14px;
    border: none;
    border-radius: var(--radius-lg);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    transition: all 0.15s cubic-bezier(0.16, 1, 0.3, 1);
  }
  .item.selected {
    background: var(--accent-soft);
    color: var(--text);
  }
  .item:hover:not(.selected) {
    background: var(--bg-hover);
  }
  .item-icon {
    width: 36px;
    height: 36px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-md);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 16px;
    flex-shrink: 0;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.06);
    transition: all 0.15s ease;
  }
  .item.selected .item-icon {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
  }
  .item-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .item-label {
    font-size: 15px;
    font-weight: 500;
    letter-spacing: -0.01em;
  }
  .item-desc {
    font-size: 13px;
    color: var(--text-muted);
  }
  .item.selected .item-desc {
    color: var(--text-secondary);
  }
  kbd {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-muted);
    background: var(--bg-subtle);
    padding: 3px 7px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    font-family: var(--font-mono);
    white-space: nowrap;
    flex-shrink: 0;
    letter-spacing: 0.02em;
  }
  .item.selected kbd {
    color: var(--text-secondary);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-color: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .empty {
    padding: 40px 24px;
    text-align: center;
    color: var(--text-muted);
    font-size: var(--text-md);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  .empty-icon {
    font-size: 32px;
    opacity: 0.4;
  }
  .footer {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 16px;
    padding: 10px 20px;
    background: rgba(0, 0, 0, 0.015);
    border-top: 1px solid var(--border);
    font-size: 12px;
    color: var(--text-muted);
  }
  :global(.dark) .footer {
    background: rgba(255, 255, 255, 0.015);
  }
  .footer-item {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .footer kbd {
    font-size: 10px;
    padding: 2px 5px;
    background: transparent;
  }
</style>
