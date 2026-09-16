<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import {
    listMemories,
    saveMemoryManual,
    deleteMemory,
    toggleMemoryStatus,
    resolveDrift,
    exportMemoriesMarkdown,
    importMemoriesMarkdown,
    listGoldenQueries,
    deleteGoldenQuery,
    runConsolidation,
    type MemoryItem,
    type GoldenQuery,
    type DriftAlert,
  } from '../../ipc/ai.ts';
  import {
    driftAlertsFor,
    removeDriftAlert,
  } from '../../stores/memoryDrift.svelte.ts';

  let {
    isOpen = $bindable(false),
    connectionId = '',
    onClose,
    triggerEl = null,
  }: {
    isOpen?: boolean;
    connectionId?: string;
    onClose?: () => void;
    /**
     * Element that opened the dialog. Focus returns here on close. Passed
     * explicitly rather than read back from `document.activeElement`: making
     * the background inert strips focus to <body> in a real browser, so by the
     * time an effect runs the opener is already gone.
     */
    triggerEl?: HTMLElement | null;
  } = $props();

  let activeTab = $state<'active' | 'archived' | 'drift' | 'golden'>('active');
  let searchQuery = $state('');
  let loading = $state(false);
  let statusMessage = $state<string | null>(null);

  // Two-step delete confirmation (F-I1). Holds the id of the row whose Delete
  // button is armed to "Confirm?"; a second click on the same row deletes.
  let confirmDeleteId = $state<string | null>(null);

  let activeMemories = $state<MemoryItem[]>([]);
  let archivedMemories = $state<MemoryItem[]>([]);
  let driftAlerts = $state<DriftAlert[]>([]);
  let goldenQueries = $state<GoldenQuery[]>([]);

  // Genuine drift alerts returned by backend consolidation (F-I2). `refresh()`
  // synthesizes a fallback from stale memories for display, but must never
  // clobber real alerts when the backend reported them.
  let genuineDriftAlerts = $state<DriftAlert[]>([]);
  // Which connection the retained alerts belong to, so switching connections
  // cannot leak one database's drift alerts into another.
  let genuineDriftKey = $state<string | null>(null);

  // Add rule modal state
  let showAddModal = $state(false);
  let newCategory = $state<'metric' | 'join' | 'quirk' | 'preference'>(
    'metric',
  );
  let newKeyPhrase = $state('');
  let newRuleText = $state('');
  let newSqlSnippet = $state('');
  let addError = $state<string | null>(null);

  // Import modal state
  let showImportModal = $state(false);
  let importMarkdownContent = $state('');
  let importError = $state<string | null>(null);

  // Dialog focus/background management (F-C3). The dialog is portaled to
  // <body> so the real app chrome behind the fixed overlay — not just the chat
  // panel — can be marked inert while the modal is up. Svelte 5 attaches
  // delegated listeners to `document` as well as the mount container, so events
  // from the portaled subtree keep working.
  let portalHost = $state<HTMLElement | null>(null);
  let drawerEl = $state<HTMLDivElement>();
  let addModalEl = $state<HTMLDivElement>();
  let importModalEl = $state<HTMLDivElement>();
  let searchInputEl = $state<HTMLInputElement>();

  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    portalHost = node;
    return {
      destroy() {
        node.remove();
        portalHost = null;
      },
    };
  }

  const FOCUSABLE_SELECTOR =
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  const effectiveConnectionKey = $derived(connectionId || 'global');

  export async function refresh() {
    if (!effectiveConnectionKey) return;
    const key = effectiveConnectionKey;
    loading = true;
    try {
      // Drop retained alerts that belong to a different connection before
      // deciding whether a synthesized fallback is needed.
      if (genuineDriftKey !== key) {
        genuineDriftAlerts = [];
        genuineDriftKey = null;
      }
      // Genuine Gate 1 alerts pushed by the background indexer (B-C5). These
      // are preferred over both the last manual-consolidation report and the
      // synthesized fallback below (F-I2).
      const eventAlerts = driftAlertsFor(key);
      if (eventAlerts.length > 0) {
        genuineDriftAlerts = eventAlerts;
        genuineDriftKey = key;
      }
      const all = await listMemories(key, true);
      activeMemories = all.filter((m) => m.status === 'active' && !m.tombstone);
      archivedMemories = all.filter(
        (m) => m.status === 'archived' && !m.tombstone,
      );
      const stale = all.filter(
        (m) => m.status === 'stale_invalid' || m.tombstone,
      );
      const synthesizedAlerts = stale.map((m) => ({
        memory_id: m.id,
        rule_text: m.rule_text,
        reason:
          'Referenced table or column was altered or dropped in live catalog',
        schema_name: 'public',
        table_name: m.key_phrase,
        column_name: null,
      }));
      // Prefer genuine alerts from `runConsolidation`; synthesize only when the
      // backend reported none (F-I2).
      driftAlerts =
        genuineDriftAlerts.length > 0 ? genuineDriftAlerts : synthesizedAlerts;

      goldenQueries = await listGoldenQueries(key);
    } catch (e) {
      statusMessage = `Failed to load memories: ${String(e)}`;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    // Read the connection key inline so Svelte tracks it at the effect boundary
    // rather than relying on `refresh()`'s async body to register the dependency
    // (F-I4), and gate the call explicitly on a usable key.
    const key = effectiveConnectionKey;
    if (!isOpen || !key) return;
    // Track the drift store so a `memory:drift_detected` event arriving while
    // the drawer is open refreshes the alert list (B-C5).
    void driftAlertsFor(key);
    refresh();
  });

  // Inert every background element outside the portaled dialog, then move focus
  // to the search input on open and hand it back to the explicit trigger on
  // close. `triggerEl` is captured by the parent before the background is
  // inerted, so it survives the browser stripping focus to <body>.
  $effect(() => {
    const host = portalHost;
    if (!isOpen || !host) return;

    const background = Array.from(document.body.children).filter(
      (el) => el !== host,
    );
    for (const el of background) el.setAttribute('inert', '');

    tick().then(() => searchInputEl?.focus());

    return () => {
      for (const el of background) el.removeAttribute('inert');
      const target = triggerEl;
      if (target?.isConnected) {
        tick().then(() => {
          if (target.isConnected) target.focus();
        });
      }
    };
  });

  /**
   * W3C APG modal focus trap. Wraps only at the boundaries so the browser's
   * native Tab order still runs between controls; if focus is outside the
   * active root (e.g. a child modal just opened over a drawer control) the
   * next Tab pulls it back in.
   */
  function trapTab(e: KeyboardEvent) {
    const root = showAddModal
      ? addModalEl
      : showImportModal
        ? importModalEl
        : drawerEl;
    if (!root) return;

    const focusables = Array.from(
      root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
    );
    if (focusables.length === 0) {
      e.preventDefault();
      root.focus();
      return;
    }

    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    const active = document.activeElement;
    if (e.shiftKey) {
      if (active === first || !root.contains(active)) {
        e.preventDefault();
        last.focus();
      }
    } else if (active === last || !root.contains(active)) {
      e.preventDefault();
      first.focus();
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === 'm') {
      e.preventDefault();
      isOpen = !isOpen;
      return;
    }
    if (!isOpen) return;

    if (e.key === 'Escape') {
      // Stacked dismiss: the topmost layer closes first, so an open child
      // modal never discards the whole drawer (F-C4).
      if (showAddModal) {
        showAddModal = false;
        return;
      }
      if (showImportModal) {
        showImportModal = false;
        return;
      }
      isOpen = false;
      onClose?.();
      return;
    }

    if (e.key === 'Tab') {
      trapTab(e);
    }
  }

  onMount(() => {
    window.addEventListener('keydown', handleKeydown);
  });

  onDestroy(() => {
    window.removeEventListener('keydown', handleKeydown);
  });

  async function handleAddRule() {
    if (!newKeyPhrase.trim() || !newRuleText.trim()) {
      addError = 'Key phrase and rule text are required';
      return;
    }
    try {
      await saveMemoryManual(
        effectiveConnectionKey,
        newCategory,
        newKeyPhrase.trim(),
        newRuleText.trim(),
        newSqlSnippet.trim() || undefined,
        'connection',
      );
      showAddModal = false;
      newKeyPhrase = '';
      newRuleText = '';
      newSqlSnippet = '';
      addError = null;
      await refresh();
    } catch (e) {
      addError = String(e);
    }
  }

  async function handleDeleteMemory(id: string) {
    try {
      await deleteMemory(id);
      removeDriftAlert(effectiveConnectionKey, id);
      genuineDriftAlerts = genuineDriftAlerts.filter((a) => a.memory_id !== id);
      await refresh();
    } catch (e) {
      statusMessage = `Delete failed: ${String(e)}`;
    }
  }

  /** Two-step delete (F-I1): first click arms the row, second click deletes it. */
  async function requestDeleteMemory(id: string) {
    if (confirmDeleteId !== id) {
      confirmDeleteId = id;
      return;
    }
    confirmDeleteId = null;
    await handleDeleteMemory(id);
  }

  async function handleToggleStatus(id: string, newStatus: string) {
    try {
      await toggleMemoryStatus(id, newStatus);
      await refresh();
    } catch (e) {
      statusMessage = `Update failed: ${String(e)}`;
    }
  }

  async function handleResolveDrift(
    id: string,
    resolution: 'revalidate' | 'dismiss',
  ) {
    try {
      await resolveDrift(id, resolution);
      // A resolved alert must not linger just because it was retained (F-I2).
      removeDriftAlert(effectiveConnectionKey, id);
      genuineDriftAlerts = genuineDriftAlerts.filter((a) => a.memory_id !== id);
      await refresh();
    } catch (e) {
      statusMessage = `Resolve drift failed: ${String(e)}`;
    }
  }

  async function handleDeleteGolden(id: string) {
    try {
      await deleteGoldenQuery(id);
      await refresh();
    } catch (e) {
      statusMessage = `Delete golden query failed: ${String(e)}`;
    }
  }

  /** Two-step delete (F-I1): first click arms the row, second click deletes it. */
  async function requestDeleteGolden(id: string) {
    if (confirmDeleteId !== id) {
      confirmDeleteId = id;
      return;
    }
    confirmDeleteId = null;
    await handleDeleteGolden(id);
  }

  async function handleExportMarkdown() {
    try {
      const md = await exportMemoriesMarkdown(effectiveConnectionKey);
      await navigator.clipboard.writeText(md);
      statusMessage = 'Copied LUCENT.md to clipboard!';
      setTimeout(() => {
        statusMessage = null;
      }, 3000);
    } catch (e) {
      statusMessage = `Export failed: ${String(e)}`;
    }
  }

  async function handleImportMarkdown() {
    if (!importMarkdownContent.trim()) return;
    try {
      const count = await importMemoriesMarkdown(
        effectiveConnectionKey,
        importMarkdownContent,
      );
      statusMessage = `Successfully imported ${count} rules!`;
      showImportModal = false;
      importMarkdownContent = '';
      await refresh();
      setTimeout(() => {
        statusMessage = null;
      }, 3000);
    } catch (e) {
      importError = String(e);
    }
  }

  async function handleRunConsolidation() {
    loading = true;
    try {
      const report = await runConsolidation(effectiveConnectionKey);
      // Retain the real backend alerts so the following `refresh()` cannot
      // replace them with synthesized fallbacks (F-I2).
      genuineDriftAlerts = report.drift_alerts ?? [];
      genuineDriftKey = effectiveConnectionKey;
      statusMessage = `Maintenance complete: archived ${report.archived_memory_count} decayed memories, pruned ${report.pruned_session_json_count} messages.`;
      await refresh();
      setTimeout(() => {
        statusMessage = null;
      }, 4000);
    } catch (e) {
      statusMessage = `Consolidation error: ${String(e)}`;
    } finally {
      loading = false;
    }
  }

  const MEMORY_TABS = ['active', 'archived', 'drift', 'golden'] as const;

  const memoryTabs = $derived([
    { id: 'active' as const, label: `Active (${activeMemories.length})` },
    { id: 'archived' as const, label: `Archived (${archivedMemories.length})` },
    { id: 'drift' as const, label: `Drift Alerts (${driftAlerts.length})` },
    {
      id: 'golden' as const,
      label: `Golden Queries (${goldenQueries.length})`,
    },
  ]);

  function selectTab(tab: (typeof MEMORY_TABS)[number]) {
    activeTab = tab;
    confirmDeleteId = null;
  }

  /**
   * W3C APG Tabs keyboard support (F-I3): arrows move between tabs, Home/End
   * jump to the ends, and the selected tab carries the roving tabindex.
   */
  function handleTabKeydown(e: KeyboardEvent) {
    const idx = MEMORY_TABS.indexOf(activeTab);
    let next: number | null = null;
    if (e.key === 'ArrowRight') {
      next = (idx + 1) % MEMORY_TABS.length;
    } else if (e.key === 'ArrowLeft') {
      next = (idx - 1 + MEMORY_TABS.length) % MEMORY_TABS.length;
    } else if (e.key === 'Home') {
      next = 0;
    } else if (e.key === 'End') {
      next = MEMORY_TABS.length - 1;
    }
    if (next === null) return;
    e.preventDefault();
    const tab = MEMORY_TABS[next];
    selectTab(tab);
    tick().then(() => document.getElementById(`memory-tab-${tab}`)?.focus());
  }

  // Compute normalized search query once per query update.
  let searchQueryLower = $derived(searchQuery.trim().toLowerCase());

  // Filter activeMemories efficiently: when query is empty, return activeMemories directly
  // to avoid intermediate object/array allocations. When filtering, use searchQueryLower
  // so query normalization is performed only once instead of per item.
  let filteredActive = $derived(
    !searchQueryLower
      ? activeMemories
      : activeMemories.filter(
          (m) =>
            m.key_phrase.toLowerCase().includes(searchQueryLower) ||
            m.rule_text.toLowerCase().includes(searchQueryLower),
        ),
  );
</script>

<div class="memory-drawer-portal" data-memory-drawer-portal use:portal>
  {#if isOpen}
    <div
      class="drawer-overlay"
      onclick={() => {
        isOpen = false;
        onClose?.();
      }}
      role="presentation"
    >
      <div
        class="drawer"
        bind:this={drawerEl}
        onclick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="AI Memory Manager"
        tabindex="-1"
      >
        <header class="drawer-header">
          <div class="drawer-title-row">
            <div class="drawer-title">
              <svg
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
              >
                <path
                  d="M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm0 18a8 8 0 1 1 8-8 8 8 0 0 1-8 8z"
                />
                <path d="M12 6v6l4 2" />
              </svg>
              <h2>AI Memory Subsystem</h2>
            </div>
            <button
              class="icon-btn"
              onclick={() => {
                isOpen = false;
                onClose?.();
              }}
              title="Close (Esc)"
            >
              ✕
            </button>
          </div>

          {#if statusMessage}
            <div class="status-banner">{statusMessage}</div>
          {/if}

          <div class="drawer-nav" role="tablist" aria-label="Memory views">
            {#each memoryTabs as tab (tab.id)}
              <button
                class="nav-tab"
                class:active={activeTab === tab.id}
                id={`memory-tab-${tab.id}`}
                role="tab"
                aria-selected={activeTab === tab.id}
                aria-controls="memory-panel"
                tabindex={activeTab === tab.id ? 0 : -1}
                onclick={() => selectTab(tab.id)}
                onkeydown={handleTabKeydown}
              >
                {tab.label}
              </button>
            {/each}
          </div>

          <div class="action-bar">
            <input
              type="text"
              class="search-input"
              placeholder="Search rules..."
              bind:value={searchQuery}
              bind:this={searchInputEl}
            />
            <button
              class="action-btn primary"
              onclick={() => (showAddModal = true)}
            >
              + Add Rule
            </button>
            <button
              class="action-btn"
              onclick={handleExportMarkdown}
              title="Copy LUCENT.md to clipboard"
            >
              Export MD
            </button>
            <button class="action-btn" onclick={() => (showImportModal = true)}>
              Import MD
            </button>
            <button
              class="action-btn secondary"
              onclick={handleRunConsolidation}
              title="Prune decayed memories & audit schema drift"
            >
              Consolidate
            </button>
          </div>
        </header>

        <div
          class="drawer-body"
          role="tabpanel"
          id="memory-panel"
          aria-labelledby={`memory-tab-${activeTab}`}
        >
          {#if loading}
            <div class="loading-state">Loading AI memories...</div>
          {:else if activeTab === 'active'}
            {#if filteredActive.length === 0}
              <div class="empty-state">
                No active memories found. Use "+ Add Rule" or let the AI
                discover domain rules as you chat.
              </div>
            {:else}
              <div class="card-list">
                {#each filteredActive as m (m.id)}
                  <div class="memory-card">
                    <div class="card-header">
                      <span class="category-badge {m.category}"
                        >{m.category}</span
                      >
                      <span class="key-phrase">{m.key_phrase}</span>
                      <span class="trust-pill {m.source_trust}"
                        >{m.source_trust}</span
                      >
                    </div>
                    <div class="rule-text">{m.rule_text}</div>
                    {#if m.sql_snippet}
                      <pre class="sql-snippet"><code>{m.sql_snippet}</code
                        ></pre>
                    {/if}
                    <div class="card-footer">
                      <span class="meta"
                        >Access: {m.access_count}x · Stability: {m.stability_hours.toFixed(
                          0,
                        )}h</span
                      >
                      <div class="card-actions">
                        <button
                          class="text-btn"
                          onclick={() => handleToggleStatus(m.id, 'archived')}
                          >Archive</button
                        >
                        <button
                          class="text-btn danger"
                          onclick={() => requestDeleteMemory(m.id)}
                        >
                          {confirmDeleteId === m.id ? 'Confirm?' : 'Delete'}
                        </button>
                      </div>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          {:else if activeTab === 'archived'}
            {#if archivedMemories.length === 0}
              <div class="empty-state">
                No archived memories. Decayed memories are soft-archived here.
              </div>
            {:else}
              <div class="card-list">
                {#each archivedMemories as m (m.id)}
                  <div class="memory-card archived">
                    <div class="card-header">
                      <span class="category-badge {m.category}"
                        >{m.category}</span
                      >
                      <span class="key-phrase">{m.key_phrase}</span>
                      <span class="archived-tag">ARCHIVED</span>
                    </div>
                    <div class="rule-text">{m.rule_text}</div>
                    <div class="card-footer">
                      <span class="meta"
                        >Last accessed: {new Date(
                          m.last_accessed_at * 1000,
                        ).toLocaleDateString()}</span
                      >
                      <div class="card-actions">
                        <button
                          class="text-btn"
                          onclick={() => handleToggleStatus(m.id, 'active')}
                          >Restore</button
                        >
                        <button
                          class="text-btn danger"
                          onclick={() => requestDeleteMemory(m.id)}
                        >
                          {confirmDeleteId === m.id ? 'Confirm?' : 'Delete'}
                        </button>
                      </div>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          {:else if activeTab === 'drift'}
            {#if driftAlerts.length === 0}
              <div class="empty-state green">
                All memory entity links are valid and synced with the active
                schema catalog.
              </div>
            {:else}
              <div class="card-list">
                {#each driftAlerts as alert (alert.memory_id)}
                  <div class="memory-card drift">
                    <div class="card-header">
                      <span class="drift-badge">SCHEMA DRIFT</span>
                      <span class="key-phrase">{alert.table_name}</span>
                    </div>
                    <div class="drift-reason">{alert.reason}</div>
                    <div class="rule-text">{alert.rule_text}</div>
                    <div class="card-footer">
                      <div class="card-actions">
                        <button
                          class="action-btn primary small"
                          onclick={() =>
                            handleResolveDrift(alert.memory_id, 'revalidate')}
                        >
                          Revalidate
                        </button>
                        <button
                          class="action-btn small"
                          onclick={() =>
                            handleResolveDrift(alert.memory_id, 'dismiss')}
                        >
                          Dismiss
                        </button>
                      </div>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          {:else if activeTab === 'golden'}
            {#if goldenQueries.length === 0}
              <div class="empty-state">
                No golden queries saved yet. Click "⭐ Save as Golden Query" on
                query result cards.
              </div>
            {:else}
              <div class="card-list">
                {#each goldenQueries as q (q.id)}
                  <div class="memory-card golden">
                    <div class="card-header">
                      <span class="golden-badge">⭐ Golden Query</span>
                      <span class="key-phrase">{q.schema_name}</span>
                      {#if q.verified}
                        <span class="verified-tag">VERIFIED</span>
                      {/if}
                    </div>
                    <div class="prompt-text">
                      <strong>Prompt:</strong>
                      {q.natural_prompt}
                    </div>
                    <pre class="sql-snippet"><code>{q.sql_text}</code></pre>
                    <div class="card-footer">
                      <span class="meta"
                        >Runs: {q.run_count} · Tables: {q.tables_used.join(
                          ', ',
                        )}</span
                      >
                      <div class="card-actions">
                        <button
                          class="text-btn danger"
                          onclick={() => requestDeleteGolden(q.id)}
                        >
                          {confirmDeleteId === q.id ? 'Confirm?' : 'Delete'}
                        </button>
                      </div>
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          {/if}
        </div>
      </div>
    </div>
  {/if}

  {#if showAddModal}
    <div
      class="modal-overlay"
      onclick={() => (showAddModal = false)}
      role="presentation"
    >
      <div
        class="modal-content"
        bind:this={addModalEl}
        onclick={(e) => e.stopPropagation()}
        role="dialog"
      >
        <h3>Add Learned Rule</h3>
        {#if addError}
          <div class="error-banner">{addError}</div>
        {/if}
        <div class="form-group">
          <label for="newCategory">Category</label>
          <select id="newCategory" bind:value={newCategory}>
            <option value="metric">Metric (Calculations, formulas)</option>
            <option value="join"
              >Join (Idiosyncratic keys, multi-table paths)</option
            >
            <option value="quirk">Quirk (Schema oddities, soft-deletes)</option>
            <option value="preference"
              >Preference (User habits, dialect styles)</option
            >
          </select>
        </div>
        <div class="form-group">
          <label for="newKeyPhrase">Key Phrase</label>
          <input
            id="newKeyPhrase"
            type="text"
            placeholder="e.g. active_subscribers"
            bind:value={newKeyPhrase}
          />
        </div>
        <div class="form-group">
          <label for="newRuleText">Rule Explanation (Max 500 chars)</label>
          <textarea
            id="newRuleText"
            rows="3"
            placeholder="Explain the domain constraint or rule..."
            bind:value={newRuleText}></textarea>
        </div>
        <div class="form-group">
          <label for="newSqlSnippet"
            >Optional SQL Snippet (Max 1000 chars)</label
          >
          <textarea
            id="newSqlSnippet"
            rows="2"
            placeholder="e.g. status = 'active' AND canceled_at IS NULL"
            bind:value={newSqlSnippet}></textarea>
        </div>
        <div class="modal-actions">
          <button class="action-btn" onclick={() => (showAddModal = false)}
            >Cancel</button
          >
          <button class="action-btn primary" onclick={handleAddRule}
            >Save Rule</button
          >
        </div>
      </div>
    </div>
  {/if}

  {#if showImportModal}
    <div
      class="modal-overlay"
      onclick={() => (showImportModal = false)}
      role="presentation"
    >
      <div
        class="modal-content"
        bind:this={importModalEl}
        onclick={(e) => e.stopPropagation()}
        role="dialog"
      >
        <h3>Import Markdown Rules (LUCENT.md)</h3>
        {#if importError}
          <div class="error-banner">{importError}</div>
        {/if}
        <div class="form-group">
          <label for="importMarkdown">Paste Markdown Content</label>
          <textarea
            id="importMarkdown"
            rows="8"
            placeholder="# Lucent Database Rules..."
            bind:value={importMarkdownContent}></textarea>
        </div>
        <div class="modal-actions">
          <button class="action-btn" onclick={() => (showImportModal = false)}
            >Cancel</button
          >
          <button class="action-btn primary" onclick={handleImportMarkdown}
            >Import Rules</button
          >
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .drawer-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    z-index: 100;
    display: flex;
    justify-content: flex-end;
    backdrop-filter: blur(2px);
  }
  .drawer {
    width: 480px;
    max-width: 90vw;
    height: 100%;
    background: var(--bg-surface);
    border-left: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    box-shadow: -4px 0 20px rgba(0, 0, 0, 0.3);
    animation: slideIn 0.2s ease-out;
  }
  @keyframes slideIn {
    from {
      transform: translateX(100%);
    }
    to {
      transform: translateX(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .drawer {
      animation: none;
    }
  }
  .drawer-header {
    padding: 16px;
    border-bottom: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .drawer-title-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .drawer-title {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--accent);
  }
  .drawer-title h2 {
    font-size: var(--text-base, 15px);
    font-weight: 600;
    margin: 0;
    color: var(--text);
  }
  .icon-btn {
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 16px;
    padding: 4px 8px;
    border-radius: var(--radius-sm, 4px);
  }
  .icon-btn:hover {
    color: var(--text);
    background: var(--bg-subtle);
  }
  .status-banner {
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    color: var(--accent);
    padding: 6px 10px;
    border-radius: var(--radius-sm, 4px);
    font-size: var(--text-xs, 12px);
  }
  .drawer-nav {
    display: flex;
    border-bottom: 1px solid var(--border);
    gap: 4px;
  }
  .nav-tab {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
    font-size: var(--text-xs, 12px);
    font-weight: 500;
    padding: 6px 8px;
    cursor: pointer;
    transition: all 0.15s;
  }
  .nav-tab:hover {
    color: var(--text);
  }
  .nav-tab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .action-bar {
    display: flex;
    gap: 6px;
    align-items: center;
    /* Wrap rather than compress the search field below a usable width (F-I6). */
    flex-wrap: wrap;
  }
  .search-input {
    flex: 1 1 140px;
    min-width: 140px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm, 4px);
    color: var(--text);
    padding: 5px 8px;
    font-size: var(--text-xs, 12px);
  }
  .action-btn {
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    color: var(--text);
    font-size: var(--text-xs, 12px);
    padding: 5px 9px;
    border-radius: var(--radius-sm, 4px);
    cursor: pointer;
    white-space: nowrap;
  }
  .action-btn:hover {
    background: var(--bg-hover, rgba(255, 255, 255, 0.08));
  }
  .action-btn.primary {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .action-btn.small {
    padding: 3px 8px;
    font-size: 11px;
  }
  .drawer-body {
    flex: 1;
    overflow-y: auto;
    padding: 16px;
  }
  .loading-state,
  .empty-state {
    padding: 32px 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: var(--text-sm, 13px);
  }
  .empty-state.green {
    color: var(--success, #10b981);
  }
  .card-list {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .memory-card {
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-md, 6px);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .memory-card.archived {
    opacity: 0.75;
    border-style: dashed;
  }
  .memory-card.drift {
    border-color: #f59e0b;
    background: rgba(245, 158, 11, 0.05);
  }
  .card-header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .category-badge {
    text-transform: uppercase;
    font-size: 11px;
    font-weight: 700;
    padding: 2px 6px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.1);
  }
  .category-badge.metric {
    color: #38bdf8;
  }
  .category-badge.join {
    color: #a855f7;
  }
  .category-badge.quirk {
    color: #f59e0b;
  }
  .category-badge.preference {
    color: #10b981;
  }
  .key-phrase {
    font-weight: 600;
    font-size: var(--text-xs, 12px);
    color: var(--text);
  }
  .trust-pill {
    margin-left: auto;
    font-size: 11px;
    padding: 1px 5px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.05);
    color: var(--text-muted);
  }
  .trust-pill.user_explicit {
    background: rgba(16, 185, 129, 0.15);
    color: #10b981;
  }
  .archived-tag {
    margin-left: auto;
    font-size: 11px;
    color: #ef4444;
    font-weight: 600;
  }
  .drift-badge {
    background: #f59e0b;
    color: #000;
    font-weight: bold;
    font-size: 11px;
    padding: 1px 5px;
    border-radius: 3px;
  }
  .golden-badge {
    color: #fbbf24;
    font-weight: 600;
    font-size: 12px;
  }
  .verified-tag {
    margin-left: auto;
    font-size: 11px;
    padding: 1px 5px;
    border-radius: 3px;
    background: rgba(16, 185, 129, 0.15);
    color: #10b981;
  }
  .drift-reason {
    font-size: 11px;
    color: #f59e0b;
  }
  .rule-text {
    font-size: var(--text-sm, 13px);
    color: var(--text);
    line-height: 1.4;
  }
  .prompt-text {
    font-size: var(--text-xs, 12px);
    color: var(--text-muted);
  }
  .sql-snippet {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    padding: 6px 8px;
    border-radius: 4px;
    font-family: var(--font-mono);
    font-size: 11px;
    overflow-x: auto;
    color: var(--accent);
    margin: 0;
  }
  .card-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-top: 4px;
    border-top: 1px solid rgba(255, 255, 255, 0.05);
  }
  .meta {
    font-size: 11px;
    color: var(--text-muted);
  }
  .card-actions {
    display: flex;
    gap: 8px;
  }
  .text-btn {
    background: transparent;
    border: none;
    font-size: 11px;
    color: var(--accent);
    cursor: pointer;
    padding: 0;
  }
  .text-btn:hover {
    text-decoration: underline;
  }
  .text-btn.danger {
    color: #ef4444;
  }
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    z-index: 120;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .modal-content {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-md, 8px);
    width: 420px;
    max-width: 90vw;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .modal-content h3 {
    margin: 0;
    font-size: 15px;
    color: var(--text);
  }
  .form-group {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .form-group label {
    font-size: 11px;
    color: var(--text-muted);
    font-weight: 500;
  }
  .form-group input,
  .form-group select,
  .form-group textarea {
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    padding: 6px 8px;
    font-size: 12px;
    font-family: inherit;
  }
  .error-banner {
    background: rgba(239, 68, 68, 0.15);
    border: 1px solid #ef4444;
    color: #ef4444;
    padding: 6px 8px;
    border-radius: 4px;
    font-size: 11px;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 8px;
  }
</style>
