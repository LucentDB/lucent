<script lang="ts">
  import { onMount } from 'svelte';
  import { history } from '../../stores/history.svelte';
  import HistoryEntry from './HistoryEntry.svelte';

  let {
    onRerun,
    onClose,
  }: {
    onRerun?: (sql: string) => void;
    onClose?: () => void;
  } = $props();

  let activeTab = $state<'history' | 'saved'>('history');
  let searchInput: HTMLInputElement | undefined = $state();

  onMount(() => {
    history.loadHistory();
  });

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === '/' && !e.ctrlKey && !e.metaKey) {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag !== 'INPUT' && tag !== 'TEXTAREA') {
        e.preventDefault();
        searchInput?.focus();
      }
    }
    if (e.key === 'Escape') {
      searchInput?.blur();
    }
  }

  async function handleSearch(value: string) {
    history.setSearch(value);
  }

  function handleCopy(sql: string) {
    navigator.clipboard.writeText(sql).catch(() => {});
  }

  function handleClear() {
    history.clearHistory();
  }
</script>

<div class="history-panel" onkeydown={handleKeydown}>
  <!-- Header -->
  <div class="panel-header">
    <h2 class="panel-title">Query History</h2>
    <button class="close-btn" onclick={() => onClose?.()}>
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
      >
        <line x1="18" y1="6" x2="6" y2="18" />
        <line x1="6" y1="6" x2="18" y2="18" />
      </svg>
    </button>
  </div>

  <!-- Tabs -->
  <div class="tab-bar">
    <button
      class="tab"
      class:active={activeTab === 'history'}
      onclick={() => {
        activeTab = 'history';
        history.loadHistory();
      }}>History</button
    >
    <button
      class="tab"
      class:active={activeTab === 'saved'}
      onclick={() => {
        activeTab = 'saved';
        history.setFavoritesOnly(true);
      }}>Saved</button
    >
    {#if activeTab === 'history'}
      <button class="clear-btn" onclick={handleClear} title="Clear all history">
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <polyline points="3 6 5 6 21 6" />
          <path
            d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"
          />
        </svg>
      </button>
    {/if}
  </div>

  <!-- Search -->
  <div class="search-box">
    <svg
      class="search-icon"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
    >
      <circle cx="11" cy="11" r="8" />
      <line x1="21" y1="21" x2="16.65" y2="16.65" />
    </svg>
    <input
      bind:this={searchInput}
      type="text"
      placeholder="Search queries...  (/)"
      value={history.searchQuery}
      oninput={(e) => handleSearch((e.target as HTMLInputElement).value)}
    />
    {#if history.searchQuery}
      <button class="clear-search" onclick={() => handleSearch('')}>
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <line x1="18" y1="6" x2="6" y2="18" />
          <line x1="6" y1="6" x2="18" y2="18" />
        </svg>
      </button>
    {/if}
  </div>

  <!-- Content -->
  <div class="panel-content">
    {#if history.loading}
      <div class="state-msg">Loading...</div>
    {:else if activeTab === 'saved' && history.entries.length === 0}
      <div class="state-msg">
        <p>No saved queries</p>
        <p class="state-sub">Star queries to save them for later</p>
      </div>
    {:else if history.entries.length === 0}
      <div class="state-msg">
        <p>No query history yet</p>
        <p class="state-sub">Run a query to see it here</p>
      </div>
    {:else}
      {#each history.groupedEntries as group}
        <div class="date-group">
          <div class="group-header">
            <span class="group-label">{group.label}</span>
            <span class="group-count">{group.entries.length}</span>
          </div>
          {#each group.entries as entry (entry.id)}
            <HistoryEntry
              entry={entry as any}
              onToggleFavorite={(id) => history.toggleFavorite(id)}
              onDelete={(id) => history.deleteEntry(id)}
              onRerun={(sql) => onRerun?.(sql)}
              onCopy={handleCopy}
            />
          {/each}
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .history-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-surface);
    border-left: 1px solid var(--border);
    width: 360px;
    flex-shrink: 0;
  }
  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  .panel-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    margin: 0;
  }
  .close-btn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
  }
  .close-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .tab-bar {
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--border);
    padding: 0 16px;
  }
  .tab {
    padding: 8px 14px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: 13px;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    transition:
      color 0.1s,
      border-color 0.1s;
  }
  .tab.active {
    color: var(--text);
    border-bottom-color: var(--accent);
    font-weight: 500;
  }
  .tab:hover {
    color: var(--text);
  }
  .clear-btn {
    margin-left: auto;
    padding: 4px 8px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .clear-btn:hover {
    color: var(--error);
    background: color-mix(in srgb, var(--error) 10%, transparent);
  }
  .search-box {
    position: relative;
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
  }
  .search-icon {
    position: absolute;
    left: 24px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-muted);
    pointer-events: none;
  }
  .search-box input {
    width: 100%;
    padding: 6px 10px 6px 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    color: var(--text);
    font-size: 13px;
    outline: none;
    box-sizing: border-box;
  }
  .search-box input:focus {
    border-color: var(--accent);
  }
  .clear-search {
    position: absolute;
    right: 24px;
    top: 50%;
    transform: translateY(-50%);
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }
  .panel-content {
    flex: 1;
    overflow-y: auto;
    padding: 8px 0;
  }
  .state-msg {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 48px 24px;
    color: var(--text-muted);
    font-size: 14px;
    text-align: center;
  }
  .state-sub {
    font-size: 12px;
    margin-top: 4px;
  }
  .date-group {
  }
  .group-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px 4px;
  }
  .group-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .group-count {
    font-size: 10px;
    color: var(--text-muted);
    background: var(--bg-hover);
    padding: 0 5px;
    border-radius: 99px;
  }
</style>
