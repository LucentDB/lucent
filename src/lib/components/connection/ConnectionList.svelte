<script lang="ts">
  import ConnectionCard from './ConnectionCard.svelte';
  import type { ConnectionProfile } from '../../stores/connections.svelte';

  let {
    profiles,
    groupedProfiles,
    loading,
    activeProfileId,
    testingIds,
    onSelect,
    onTest,
    onEdit,
    onDelete,
    onDuplicate,
    onNewConnection,
  }: {
    profiles: ConnectionProfile[];
    groupedProfiles: { name: string; profiles: ConnectionProfile[] }[];
    loading: boolean;
    activeProfileId: string | null;
    testingIds: Set<string>;
    onSelect?: (id: string) => void;
    onTest?: (id: string) => void;
    onEdit?: (profile: ConnectionProfile) => void;
    onDelete?: (id: string) => void;
    onDuplicate?: (id: string) => void;
    onNewConnection?: () => void;
  } = $props();

  let searchQuery = $state('');
  let viewMode = $state<'list' | 'grid'>('list');
  let searchInput: HTMLInputElement | undefined = $state();

  // Cache the lowercased search strings for each profile outside the filter loop
  // to avoid repeated string allocations when searchQuery changes.
  let searchableProfiles = $derived(
    profiles.map((p) => ({
      p,
      searchString:
        `${p.name} ${p.params['host'] ?? ''} ${p.params['user'] ?? ''} ${p.params['database'] ?? ''} ${p.group ?? ''}`.toLowerCase(),
    })),
  );

  // Filtered profiles based on search query
  let filteredProfiles = $derived.by(() => {
    if (!searchQuery.trim()) return profiles;
    const q = searchQuery.toLowerCase();
    return searchableProfiles
      .filter((item) => item.searchString.includes(q))
      .map((item) => item.p);
  });

  // Filtered groups (only groups with matching profiles)
  let filteredGroups = $derived.by(() => {
    if (!searchQuery.trim()) return groupedProfiles;
    const ids = new Set(filteredProfiles.map((p) => p.id));
    return groupedProfiles
      .map((g) => ({
        ...g,
        profiles: g.profiles.filter((p) => ids.has(p.id)),
      }))
      .filter((g) => g.profiles.length > 0);
  });

  // Focus the search input when '/' is pressed anywhere in the list
  function handleGlobalKeydown(e: KeyboardEvent) {
    if (e.key === '/' && !e.ctrlKey && !e.metaKey) {
      // Only focus if the search input exists and we're not already typing in an input
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag !== 'INPUT' && tag !== 'TEXTAREA') {
        e.preventDefault();
        searchInput?.focus();
      }
    }
    if (e.key === 'Escape') {
      searchInput?.blur();
      searchQuery = '';
    }
  }
</script>

<div class="connection-list" onkeydown={handleGlobalKeydown}>
  <!-- Toolbar -->
  <div class="list-toolbar">
    <div class="search-wrapper">
      <svg
        class="search-icon"
        width="16"
        height="16"
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
        class="search-input"
        placeholder="Search connections...  (/)"
        bind:value={searchQuery}
      />
    </div>
    <div class="toolbar-actions">
      <button
        class="view-toggle"
        onclick={() => (viewMode = viewMode === 'list' ? 'grid' : 'list')}
        title="Toggle view"
      >
        {#if viewMode === 'list'}
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <rect x="3" y="3" width="7" height="7" />
            <rect x="14" y="3" width="7" height="7" />
            <rect x="3" y="14" width="7" height="7" />
            <rect x="14" y="14" width="7" height="7" />
          </svg>
        {:else}
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <line x1="8" y1="6" x2="21" y2="6" />
            <line x1="8" y1="12" x2="21" y2="12" />
            <line x1="8" y1="18" x2="21" y2="18" />
            <line x1="3" y1="6" x2="3.01" y2="6" />
            <line x1="3" y1="12" x2="3.01" y2="12" />
            <line x1="3" y1="18" x2="3.01" y2="18" />
          </svg>
        {/if}
      </button>
      <button class="new-btn" onclick={() => onNewConnection?.()}>
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
        >
          <line x1="12" y1="5" x2="12" y2="19" />
          <line x1="5" y1="12" x2="19" y2="12" />
        </svg>
        New
      </button>
    </div>
  </div>

  <!-- Content -->
  <div class="list-content">
    {#if loading}
      <div class="loading-state">
        <div class="spinner-lg"></div>
        <span>Loading connections...</span>
      </div>
    {:else if filteredProfiles.length === 0}
      <div class="empty-state">
        {#if searchQuery}
          <p>No connections matching "{searchQuery}"</p>
          <button class="clear-btn" onclick={() => (searchQuery = '')}
            >Clear search</button
          >
        {:else}
          <svg
            width="48"
            height="48"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            class="empty-icon"
          >
            <ellipse cx="12" cy="5" rx="9" ry="3" />
            <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" />
            <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
          </svg>
          <p>No saved connections</p>
          <p class="empty-sub">Create a connection to get started</p>
          <button class="new-btn" onclick={() => onNewConnection?.()}
            >New Connection</button
          >
        {/if}
      </div>
    {:else if viewMode === 'grid'}
      <div class="grid-view">
        {#each filteredProfiles as profile (profile.id)}
          <ConnectionCard
            {profile}
            active={profile.id === activeProfileId}
            testing={testingIds.has(profile.id)}
            viewMode="grid"
            {onSelect}
            {onTest}
            {onEdit}
            {onDelete}
            {onDuplicate}
          />
        {/each}
      </div>
    {:else}
      <div class="list-view">
        {#each filteredGroups as group}
          {#if group.name}
            <div class="group-header">
              <span class="group-name">{group.name}</span>
              <span class="group-count">{group.profiles.length}</span>
            </div>
          {/if}
          {#each group.profiles as profile (profile.id)}
            <ConnectionCard
              {profile}
              active={profile.id === activeProfileId}
              testing={testingIds.has(profile.id)}
              viewMode="list"
              {onSelect}
              {onTest}
              {onEdit}
              {onDelete}
              {onDuplicate}
            />
          {/each}
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .connection-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .list-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0 12px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .search-wrapper {
    flex: 1;
    position: relative;
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
    padding: 8px 10px 8px 32px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    color: var(--text);
    font-size: 13px;
    outline: none;
    transition: border-color 0.12s;
  }
  .search-input:focus {
    border-color: var(--accent);
  }
  .toolbar-actions {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .view-toggle {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    color: var(--text-secondary);
    cursor: pointer;
  }
  .view-toggle:hover {
    background: var(--bg-hover);
  }
  .new-btn {
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 7px 14px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-md);
    background: var(--accent);
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.12s;
  }
  .new-btn:hover {
    background: var(--accent-hover);
  }
  .list-content {
    padding: 8px 0;
  }
  .loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 48px 0;
    color: var(--text-muted);
    font-size: 14px;
  }
  .spinner-lg {
    width: 32px;
    height: 32px;
    border: 3px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 48px 0;
    color: var(--text-muted);
    text-align: center;
  }
  .empty-icon {
    margin-bottom: 8px;
    opacity: 0.4;
  }
  .empty-sub {
    font-size: 13px;
    color: var(--text-muted);
  }
  .clear-btn {
    padding: 6px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    color: var(--text);
    cursor: pointer;
    font-size: 13px;
  }
  .grid-view {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 8px;
  }
  .list-view {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .group-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 4px 4px;
    margin-top: 8px;
  }
  .group-header:first-child {
    margin-top: 0;
  }
  .group-name {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .group-count {
    font-size: 11px;
    color: var(--text-muted);
    background: var(--bg-hover);
    padding: 0 6px;
    border-radius: 99px;
  }
</style>
