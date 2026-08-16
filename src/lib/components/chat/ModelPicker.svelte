<script>
  let {
    status = 'idle',
    models = [],
    value = '',
    onChange = () => {},
    errorMessage = '',
    providerLabel = 'this provider',
  } = $props();

  let query = $state('');
  let activeIndex = $state(0);
  let queryLower = $derived(query.trim().toLowerCase());
  let matches = $derived(
    models.filter(
      (m) =>
        m.id.toLowerCase().includes(queryLower) ||
        m.displayName.toLowerCase().includes(queryLower),
    ),
  );

  function pick(id) {
    onChange(id);
  }

  function handleManualInput(e) {
    onChange(e.target.value);
  }

  function handleSearchKeydown(e) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      activeIndex = Math.min(activeIndex + 1, matches.length - 1);
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      activeIndex = Math.max(activeIndex - 1, 0);
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      const m = matches[activeIndex];
      if (m) onChange(m.id);
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      query = '';
    }
  }

  $effect(() => {
    // Reset activeIndex whenever the query changes so it never points past
    // the filtered list.
    void query;
    activeIndex = 0;
  });
</script>

<div class="model-picker">
  {#if status === 'idle'}
    <p class="hint">
      Click Fetch Models to load available models for {providerLabel}.
    </p>
  {:else if status === 'loading'}
    <div class="skeleton-list" aria-live="polite" aria-busy="true">
      {#each Array(4) as _, i (i)}
        <div class="skeleton-row" data-testid="model-skeleton-row"></div>
      {/each}
    </div>
  {:else if status === 'success'}
    <div class="search-row">
      <div class="search-box">
        <svg class="search-icon" viewBox="0 0 24 24" width="16" height="16">
          <circle
            cx="11"
            cy="11"
            r="7"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          />
          <path
            d="m20 20-3.5-3.5"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
          />
        </svg>
        <input
          type="text"
          class="search-input"
          placeholder="Search models…"
          bind:value={query}
          aria-label="Search models"
          onkeydown={handleSearchKeydown}
        />
      </div>
      <span class="count"
        >{models.length} model{models.length === 1 ? '' : 's'}</span
      >
    </div>
    <div class="picker-list" role="listbox" aria-label="Available models">
      {#each matches as m, idx (m.id)}
        <button
          type="button"
          class="picker-item"
          class:active={m.id === value || idx === activeIndex}
          role="option"
          aria-selected={m.id === value}
          onclick={() => pick(m.id)}
        >
          <span>{m.displayName}</span>
          {#if m.id === value}<span class="check">✓</span>{/if}
        </button>
      {/each}
      {#if matches.length === 0}
        <div class="picker-empty">Nothing matches "{query.trim()}"</div>
      {/if}
    </div>
  {:else if status === 'error'}
    <div class="error-banner">{errorMessage}</div>
  {/if}

  {#if status !== 'success'}
    <label class="manual-entry">
      Model name
      <input
        type="text"
        {value}
        oninput={handleManualInput}
        placeholder="e.g. gpt-4o"
      />
    </label>
  {/if}
</div>

<style>
  .model-picker {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .hint {
    margin: 0;
    color: var(--text-secondary);
    font-size: 13px;
  }
  .skeleton-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .skeleton-row {
    height: 30px;
    border-radius: var(--radius-sm);
    background: var(--bg-hover);
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 0.6;
    }
    50% {
      opacity: 1;
    }
  }
  .search-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .search-box {
    position: relative;
    flex: 1;
  }
  .search-icon {
    position: absolute;
    left: 10px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-muted);
    pointer-events: none;
  }
  .search-input {
    width: 100%;
    padding: 8px 12px 8px 32px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    color: var(--text);
    font-size: 13.5px;
    transition: border-color 0.12s;
  }
  .search-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .count {
    flex-shrink: 0;
    background: var(--bg-subtle);
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 999px;
  }
  .picker-list {
    max-height: 200px;
    overflow-y: auto;
    border: 1px solid var(--border-light);
    border-radius: var(--radius-md);
    overflow-x: hidden;
  }
  .picker-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    padding: 8px 12px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 14px;
    cursor: pointer;
    text-align: left;
  }
  .picker-item:hover {
    background: var(--bg-hover);
  }
  .picker-item.active {
    background: var(--accent-soft);
    color: var(--text);
  }
  .check {
    color: var(--accent);
  }
  .picker-empty {
    padding: 10px 12px;
    color: var(--text-muted);
    font-size: 13px;
  }
  .error-banner {
    padding: 8px 12px;
    border-radius: var(--radius-sm);
    background: var(--error-bg);
    color: var(--error);
    font-size: 13px;
  }
  .manual-entry {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 13px;
    font-weight: 500;
  }
  .manual-entry input {
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    font-size: 13.5px;
    transition: border-color 0.12s;
  }
  .manual-entry input:focus {
    outline: none;
    border-color: var(--accent);
  }
</style>
