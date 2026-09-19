<script>
  import { useQueryClient } from '@tanstack/svelte-query';
  import { connections } from '../../stores/connections.svelte';
  import { connectionsQuery } from '../../queries/connections.ts';
  import {
    databasesQuery,
    schemasQuery,
    refreshExplorer as invalidateExplorer,
  } from '../../queries/explorer.ts';
  import { connectionEndpoint } from '../../connection-format';
  import { dbMatches, schemaMatches, objectMatches } from './sidebar-search.ts';
  import SchemaBranch from './SchemaBranch.svelte';
  import DbIcon from '../icons/DbIcon.svelte';

  let { onObjectClick, onDisconnect, onOpenLogs } = $props();

  // Saved profiles live in the connections query now; the store keeps only
  // session state. Resolved here for the switcher.
  const profiles = connectionsQuery();
  let activeProfile = $derived(
    (profiles.data ?? []).find((p) => p.id === connections.activeProfileId) ??
      null,
  );

  let switcherOpen = $state(false);

  let expandedDbs = $state(new Set());
  let expandedSchemas = $state(new Set());
  let expandedGroups = $state(new Set());
  let activeObject = $state(null);

  const queryClient = useQueryClient();
  const conn = $derived(connections.activeProfileId ?? 'inline');

  const dbs = databasesQuery(() => conn);
  const schemas = schemasQuery(
    () => conn,
    () => expandedDbs.size > 0,
  );

  let databases = $derived(dbs.data ?? []);
  let loading = $derived(dbs.isPending);
  let error = $derived(dbs.error ?? schemas.error ?? null);
  let schemasByDb = $derived(
    Object.fromEntries([...expandedDbs].map((db) => [db, schemas.data ?? []])),
  );
  let isRefreshing = $derived(dbs.isFetching || schemas.isFetching);

  // Objects for mounted schema branches, keyed by name. Search filtering and
  // the match badge read this; the branches themselves own the queries.
  let loadedObjects = $state({});
  function handleObjectsLoaded(name, objects) {
    // Safe against update loops: SchemaBranch reports each data array once,
    // so this runs once per fetch cycle, not per observer notification.
    if (loadedObjects[name] === objects) return;
    loadedObjects = { ...loadedObjects, [name]: objects };
    // Opening a schema reveals its object groups directly — the explorer
    // behaviour test asserts objects are visible one click after expanding.
    // Refresh re-seeds, so a refresh never collapses what was being explored.
    const kinds = new Set(expandedGroups);
    for (const o of objects) kinds.add(`${name}|${o.kind}`);
    expandedGroups = kinds;
  }

  // Open the tree on the current database once the catalog arrives — the old
  // loadDatabases seeded expandedDbs the same way. Seed-once: collapsing every
  // database afterwards must stick.
  let expandedSeeded = false;
  $effect(() => {
    const names = (dbs.data ?? [])
      .filter((d) => d.is_current)
      .map((d) => d.name);
    if (!expandedSeeded && names.length > 0) {
      expandedSeeded = true;
      expandedDbs = new Set(names);
    }
  });

  let searchQuery = $state('');
  let searchQueryLower = $derived(searchQuery.toLowerCase());
  let objectLabels = {
    table: 'Tables',
    view: 'Views',
    matview: 'Materialized Views',
    function: 'Functions',
    sequence: 'Sequences',
  };
  let groupOrder = ['table', 'view', 'matview', 'function', 'sequence'];

  async function switchConnection(id) {
    switcherOpen = false;
    await connections.connectToProfile(id).catch(() => {});
  }

  async function disconnectConnection() {
    switcherOpen = false;
    await connections.disconnect();
    onDisconnect?.();
  }

  function toggleDb(name) {
    const next = new Set(expandedDbs);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    expandedDbs = next;
  }

  function toggleSchema(schema) {
    const next = new Set(expandedSchemas);
    if (next.has(schema.name)) next.delete(schema.name);
    else next.add(schema.name);
    expandedSchemas = next;
  }

  function toggleGroup(schemaName, kind) {
    const key = `${schemaName}|${kind}`;
    const next = new Set(expandedGroups);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedGroups = next;
  }

  function handleObjectClick(schema, obj) {
    activeObject = `${schema.name}.${obj.name}`;
    onObjectClick({
      schema: schema.name,
      path: schema.path,
      name: obj.name,
      kind: obj.kind,
    });
  }

  function formatCount(n) {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'K';
    return String(n);
  }

  function groupObjects(objects) {
    const groups = {};
    for (const obj of objects) {
      if (!groups[obj.kind]) groups[obj.kind] = [];
      groups[obj.kind].push(obj);
    }
    return groupOrder
      .filter((k) => groups[k])
      .map((k) => ({ kind: k, label: objectLabels[k], items: groups[k] }));
  }

  async function handleRefresh(event) {
    event.stopPropagation();
    await invalidateExplorer(queryClient, conn);
  }
</script>

<div class="sidebar">
  <!-- Connection Switcher -->
  {#if (profiles.data ?? []).length > 0}
    <div class="connection-switcher">
      <button
        class="switcher-btn"
        onclick={() => (switcherOpen = !switcherOpen)}
      >
        <!-- Database icon -->
        <span class="switcher-db-icon">
          <DbIcon kind="database" size={13} strokeWidth={1.6} />
        </span>
        <span class="switcher-name">
          {activeProfile?.name ?? 'Select connection'}
        </span>
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          class="switcher-chevron"
          class:open={switcherOpen}
        >
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>
      {#if switcherOpen}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="switcher-dropdown" onclick={() => (switcherOpen = false)}>
          {#each profiles.data ?? [] as p}
            <button
              class="switcher-item"
              class:active={p.id === connections.activeProfileId}
              onclick={() => switchConnection(p.id)}
            >
              <span class="switcher-item-name">{p.name}</span>
              <span class="switcher-item-host">{connectionEndpoint(p)}</span>
            </button>
          {/each}
          {#if connections.status === 'connected'}
            <div class="switcher-divider"></div>
            <button class="switcher-item danger" onclick={disconnectConnection}>
              Disconnect
            </button>
          {/if}
        </div>
      {/if}
    </div>
  {/if}

  <div class="search-box">
    <svg
      class="search-icon"
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      <circle cx="11" cy="11" r="8" /><path d="M21 21l-4.35-4.35" />
    </svg>
    <input
      type="text"
      placeholder="Search objects..."
      bind:value={searchQuery}
    />
    {#if searchQuery}
      <button
        class="clear-btn"
        onclick={() => (searchQuery = '')}
        title="Clear"
      >
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <line x1="18" y1="6" x2="6" y2="18" /><line
            x1="6"
            y1="6"
            x2="18"
            y2="18"
          />
        </svg>
      </button>
    {/if}
  </div>

  {#if error}
    <div class="sidebar-error selectable">{error}</div>
  {/if}

  <div class="tree">
    {#if loading}
      <div class="loading-root">
        <span class="skeleton-line"></span>
        <span class="skeleton-line short"></span>
        <span class="skeleton-line"></span>
      </div>
    {:else if databases.length === 0}
      <div class="empty-root">Connect to a database to explore</div>
    {:else}
      {#each databases.filter( (d) => dbMatches(d.name, schemasByDb[d.name], loadedObjects, searchQueryLower) ) as db}
        <div class="tree-node">
          <div class="db-row-wrap">
            <button class="node-row db-row" onclick={() => toggleDb(db.name)}>
              <svg
                class="chevron"
                class:open={expandedDbs.has(db.name)}
                width="14"
                height="14"
                viewBox="0 0 16 16"
                fill="none"
              >
                <path
                  d="M6 4l4 4-4 4"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </svg>
              <span class="node-icon db-icon">
                <DbIcon kind="database" size={13} strokeWidth={1.6} />
              </span>
              <span class="node-name" class:current={db.is_current}
                >{db.name}</span
              >
            </button>
            {#if db.is_current}
              <button
                class="refresh-btn"
                class:spinning={isRefreshing}
                onclick={handleRefresh}
                disabled={isRefreshing}
                title="Refresh explorer"
                aria-label="Refresh explorer"
                type="button"
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
                  <path d="M20 11a8 8 0 0 0-14.9-4M4 5v4h4" />
                  <path d="M4 13a8 8 0 0 0 14.9 4M20 19v-4h-4" />
                </svg>
              </button>
            {/if}
          </div>

          {#if expandedDbs.has(db.name)}
            <div class="children">
              {#if schemas.isPending}
                <div class="loading-line">Loading…</div>
              {:else if schemasByDb[db.name]}
                {#each schemasByDb[db.name].filter( (s) => schemaMatches(s, loadedObjects[s.name], searchQueryLower) ) as schema}
                  <div class="schema-node">
                    <button
                      class="node-row schema-row"
                      onclick={() => toggleSchema(schema)}
                    >
                      <svg
                        class="chevron"
                        class:open={expandedSchemas.has(schema.name) ||
                          !!searchQuery}
                        width="14"
                        height="14"
                        viewBox="0 0 16 16"
                        fill="none"
                      >
                        <path
                          d="M6 4l4 4-4 4"
                          stroke="currentColor"
                          stroke-width="1.5"
                          stroke-linecap="round"
                          stroke-linejoin="round"
                        />
                      </svg>
                      <DbIcon
                        kind="schema"
                        size={13}
                        strokeWidth={1.6}
                        class="node-icon schema-icon"
                        open={expandedSchemas.has(schema.name) || !!searchQuery}
                      />
                      <span class="schema-name">{schema.name}</span>
                      {#if searchQuery && loadedObjects[schema.name]}
                        {@const matchCount = loadedObjects[schema.name].filter(
                          (o) => objectMatches(o.name, searchQueryLower),
                        ).length}
                        <span class="count-badge" class:match={matchCount > 0}
                          >{matchCount}</span
                        >
                      {/if}
                    </button>

                    {#if expandedSchemas.has(schema.name) || searchQuery}
                      <div class="children">
                        <SchemaBranch
                          {conn}
                          {schema}
                          expanded={expandedSchemas.has(schema.name) ||
                            !!searchQuery}
                          ondata={handleObjectsLoaded}
                        >
                          {#snippet children(objects)}
                            {#each groupObjects(objects)
                              .map( (g) => ({ ...g, items: g.items.filter( (o) => objectMatches(o.name, searchQueryLower) ) }) )
                              .filter((g) => g.items.length > 0) as group}
                              <div class="group-node">
                                <button
                                  class="group-header"
                                  class:open={expandedGroups.has(
                                    `${schema.name}|${group.kind}`,
                                  ) || !!searchQuery}
                                  onclick={() =>
                                    toggleGroup(schema.name, group.kind)}
                                >
                                  <svg
                                    class="chevron"
                                    class:open={expandedGroups.has(
                                      `${schema.name}|${group.kind}`,
                                    ) || !!searchQuery}
                                    width="12"
                                    height="12"
                                    viewBox="0 0 16 16"
                                    fill="none"
                                  >
                                    <path
                                      d="M6 4l4 4-4 4"
                                      stroke="currentColor"
                                      stroke-width="1.5"
                                      stroke-linecap="round"
                                      stroke-linejoin="round"
                                    />
                                  </svg>
                                  <span class="group-label">{group.label}</span>
                                  <span class="group-count"
                                    >{group.items.length}</span
                                  >
                                </button>
                                {#if expandedGroups.has(`${schema.name}|${group.kind}`) || searchQuery}
                                  {#each group.items as obj}
                                    <button
                                      class="object-item"
                                      class:active={activeObject ===
                                        `${schema.name}.${obj.name}`}
                                      onclick={() =>
                                        handleObjectClick(schema, obj)}
                                    >
                                      <DbIcon
                                        kind={obj.kind}
                                        size={12}
                                        strokeWidth={1.6}
                                        class="obj-icon {obj.kind}"
                                      />
                                      <span class="object-name">{obj.name}</span
                                      >
                                      {#if obj.row_count !== null && obj.row_count > 0}
                                        <span class="row-badge"
                                          >{formatCount(obj.row_count)}</span
                                        >
                                      {/if}
                                    </button>
                                  {/each}
                                {/if}
                              </div>
                            {/each}
                          {/snippet}
                        </SchemaBranch>
                      </div>
                    {/if}
                  </div>
                {/each}
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    {/if}
  </div>

  <div class="sidebar-footer">
    <button class="footer-btn" onclick={onOpenLogs} title="Worker logs">
      <svg
        width="13"
        height="13"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
      >
        <polyline points="4 17 10 11 4 5" /><line
          x1="12"
          y1="19"
          x2="20"
          y2="19"
        />
      </svg>
      <span>Logs</span>
    </button>
  </div>
</div>

<style>
  .sidebar {
    background: var(--bg-surface);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    user-select: none;
    width: 100%;
  }

  /* ── Connection Switcher ─────────────────────────── */
  .connection-switcher {
    position: relative;
    padding: 8px 8px 6px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .switcher-btn {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-elevated);
    color: var(--text);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    transition: all var(--transition-fast);
  }
  .switcher-btn:hover {
    border-color: var(--accent);
    background: var(--bg-hover);
  }
  .switcher-db-icon {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-md);
    background: var(--accent-soft);
    color: var(--accent);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .switcher-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .switcher-chevron {
    flex-shrink: 0;
    color: var(--text-muted);
    transition: transform var(--transition-fast);
  }
  .switcher-chevron.open {
    transform: rotate(180deg);
  }
  .switcher-dropdown {
    position: absolute;
    top: calc(100% - 2px);
    left: 8px;
    right: 8px;
    z-index: 100;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow:
      0 8px 24px rgba(0, 0, 0, 0.12),
      0 2px 8px rgba(0, 0, 0, 0.08);
    overflow: hidden;
    padding: 4px;
  }
  :global(.dark) .switcher-dropdown {
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  }
  .switcher-item {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 7px 10px;
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text);
    font-size: 12px;
    cursor: pointer;
    text-align: left;
    transition: background var(--transition-fast);
  }
  .switcher-item:hover {
    background: var(--bg-hover);
  }
  .switcher-item.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .switcher-item.danger {
    color: var(--danger);
  }
  .switcher-item.danger:hover {
    background: var(--danger-bg);
  }
  .switcher-item-name {
    font-weight: 500;
    font-size: 12.5px;
  }
  .switcher-item-host {
    font-size: 10.5px;
    color: var(--text-muted);
    font-family: var(--font-mono);
  }
  .switcher-divider {
    border-top: 1px solid var(--border);
    margin: 3px 0;
  }

  /* ── Search ──────────────────────────────────────── */
  .search-box {
    position: relative;
    padding: 8px 8px 7px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .search-icon {
    position: absolute;
    left: 17px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-muted);
    pointer-events: none;
  }
  .search-box input {
    width: 100%;
    padding: 6px 28px 6px 30px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-elevated);
    color: var(--text);
    font-size: 12.5px;
    outline: none;
    transition:
      border-color var(--transition-fast),
      box-shadow var(--transition-fast);
    box-sizing: border-box;
  }
  .search-box input::placeholder {
    color: var(--text-muted);
  }
  .search-box input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .clear-btn {
    position: absolute;
    right: 14px;
    top: 50%;
    transform: translateY(-50%);
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 2px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color var(--transition-fast);
  }
  .clear-btn:hover {
    color: var(--text);
  }

  /* ── State messages ───────────────────────────────── */
  .sidebar-error {
    padding: 6px 12px;
    font-size: 11.5px;
    color: var(--danger);
    background: var(--danger-bg);
    border-bottom: 1px solid rgba(239, 68, 68, 0.2);
  }
  .loading-root {
    padding: 16px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .skeleton-line {
    display: block;
    height: 12px;
    border-radius: var(--radius-sm);
    background: linear-gradient(
      90deg,
      var(--bg-hover) 25%,
      var(--bg-elevated) 50%,
      var(--bg-hover) 75%
    );
    background-size: 200% 100%;
    animation: shimmer 1.4s infinite;
  }
  .skeleton-line.short {
    width: 55%;
  }
  @keyframes shimmer {
    0% {
      background-position: 200% 0;
    }
    100% {
      background-position: -200% 0;
    }
  }
  .empty-root {
    padding: 24px 16px;
    font-size: 12.5px;
    color: var(--text-muted);
    text-align: center;
    line-height: 1.5;
  }
  .loading-line {
    padding: 5px 12px 5px 20px;
    font-size: 11px;
    color: var(--text-muted);
    font-style: italic;
  }

  /* ── Tree container ───────────────────────────────── */
  .tree {
    flex: 1;
    overflow-y: auto;
    padding: 6px 0 12px;
  }
  .tree::-webkit-scrollbar {
    width: 3px;
  }
  .tree::-webkit-scrollbar-thumb {
    background: transparent;
    border-radius: var(--radius-full);
  }
  .tree:hover::-webkit-scrollbar-thumb {
    background: var(--border);
  }

  .tree-node {
    margin-bottom: 0;
  }

  /* ── Generic tree row ──────────────────────────────── */
  /* DB level row */
  .node-row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 5px 8px 5px 6px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    border-radius: 0;
    transition: background var(--transition-fast);
    min-height: 30px;
  }
  .node-row:hover {
    background: var(--bg-hover);
  }

  .db-row-wrap {
    display: flex;
    align-items: stretch;
  }
  .db-row {
    flex: 1;
    min-width: 0;
    padding-left: 6px;
  }
  .refresh-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 30px;
    margin: 2px 4px 2px 0;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition:
      background var(--transition-fast),
      color var(--transition-fast);
  }
  .refresh-btn:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--accent);
  }
  .refresh-btn:disabled {
    cursor: default;
    opacity: 0.65;
  }
  .refresh-btn.spinning svg {
    animation: explorer-refresh-spin 0.8s linear infinite;
  }
  @keyframes explorer-refresh-spin {
    to {
      transform: rotate(360deg);
    }
  }

  .schema-row {
    padding-left: 16px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    min-height: 27px;
  }
  .schema-row:hover {
    color: var(--text);
  }

  /* ── Chevron ────────────────────────────────────── */
  .chevron {
    flex-shrink: 0;
    color: var(--text-muted);
    transition: transform 0.15s ease;
    opacity: 0.6;
  }
  .chevron.open {
    transform: rotate(90deg);
  }

  /* ── Icons ───────────────────────────────────────── */
  .node-icon {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .db-icon {
    color: var(--accent);
    width: 20px;
    height: 20px;
    background: var(--accent-soft);
    border-radius: var(--radius-sm);
    padding: 3px;
    box-sizing: border-box;
  }
  .schema-row :global(.schema-icon) {
    color: #f59e0b;
  }

  /* ── Labels ──────────────────────────────────────── */
  .node-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 600;
  }
  .node-name.current {
    color: var(--accent);
  }

  .schema-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ── Count / badge ────────────────────────────────── */
  .count-badge {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-muted);
    flex-shrink: 0;
    min-width: 16px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .count-badge.match {
    color: var(--accent);
    font-weight: 600;
  }
  .group-count {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-muted);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
  .row-badge {
    font-size: 10px;
    font-weight: 500;
    color: var(--text-muted);
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }

  /* ── Children indentation ────────────────────────────── */
  .children {
    padding-left: 6px;
  }

  /* ── Group header (Tables / Views / …) ─────────────────── */
  .group-header {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 8px 3px 20px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    cursor: pointer;
    text-align: left;
    border-radius: 0;
    transition:
      background var(--transition-fast),
      color var(--transition-fast);
    min-height: 24px;
  }
  .group-header:hover {
    background: var(--bg-hover);
    color: var(--text-secondary);
  }
  .group-label {
    flex: 1;
  }

  /* ── Object items ──────────────────────────────────── */
  .object-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px 3px 30px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
    cursor: pointer;
    text-align: left;
    border-radius: 0;
    position: relative;
    transition: background var(--transition-fast);
    min-height: 26px;
  }
  .object-item::before {
    content: '';
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: transparent;
    border-radius: 0 1px 1px 0;
    transition: background var(--transition-fast);
  }
  .object-item:hover {
    background: var(--bg-hover);
  }
  .object-item.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .object-item.active::before {
    background: var(--accent);
  }
  .object-item.active :global(.obj-icon) {
    opacity: 1;
  }

  /* ── Object icons ──────────────────────────────────── */
  /* Applied on child Lucide <svg> via DbIcon class prop */
  .object-item :global(.obj-icon) {
    flex-shrink: 0;
    width: 13px;
    height: 13px;
    opacity: 0.85;
    transition: opacity var(--transition-fast);
  }
  .object-item :global(.obj-icon.table) {
    color: #10b981;
  } /* emerald */
  .object-item :global(.obj-icon.view) {
    color: #6366f1;
  } /* indigo  */
  .object-item :global(.obj-icon.matview) {
    color: #0ea5e9;
  } /* sky */
  .object-item :global(.obj-icon.function) {
    color: #a855f7;
  } /* purple */
  .object-item :global(.obj-icon.sequence) {
    color: #f59e0b;
  } /* amber  */

  .object-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: 11.5px;
  }
  .object-item.active .object-name {
    color: var(--accent);
  }

  /* ── Footer ───────────────────────────────────────── */
  .sidebar-footer {
    flex-shrink: 0;
    border-top: 1px solid var(--border);
    padding: 6px 8px;
  }
  .footer-btn {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 6px 10px;
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    transition: all var(--transition-fast);
  }
  .footer-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .footer-btn svg {
    flex-shrink: 0;
  }
</style>
