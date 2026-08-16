<script>
  import { onMount, onDestroy, untrack } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { initIndexingListeners } from './lib/stores/indexing.svelte';
  import Sidebar from './lib/components/sidebar/Sidebar.svelte';
  import QueryEditor from './lib/components/editor/QueryEditor.svelte';
  import ResultsGrid from './lib/components/grid/ResultsGrid.svelte';
  import Dashboard from './lib/components/dashboard/Dashboard.svelte';
  import SourceView from './lib/components/source/SourceView.svelte';
  import LandingPage from './lib/components/connection/LandingPage.svelte';
  import CommandPalette from './lib/components/palette/CommandPalette.svelte';
  import AppHeader from './lib/components/AppHeader.svelte';
  import ChatLanding from './lib/components/chat/ChatLanding.svelte';
  import ChatPanel from './lib/components/chat/ChatPanel.svelte';
  import AiSettings from './lib/components/chat/AiSettings.svelte';
  import Notebook from './lib/components/notebook/Notebook.svelte';
  import LogsDrawer from './lib/components/LogsDrawer.svelte';
  import { notebooks } from './lib/stores/notebooks.svelte.ts';
  import { resultSummary } from './lib/utils/resultSummary.js';
  import {
    saveNotebook,
    saveNotebookAs,
    pickNotebookToOpen,
    confirmDiscardIfDirty,
  } from './lib/stores/notebook-files.ts';
  import { saveQueryTab, saveQueryTabAs } from './lib/stores/query-files.ts';
  import {
    connect,
    executeQuery,
    cancelQuery,
    disconnect,
    getFunctionSource,
    getViewSource,
    getSequenceInfo,
    browseTable,
    countAllRows,
    describeFilters,
  } from './lib/ipc/client.js';
  import {
    fetchMoreOptions,
    refetchOptions,
    filterSpecFor,
  } from './lib/stores/tabQuery.js';
  import { getTheme } from './lib/stores/theme.svelte.js';
  import { schemaSummary } from './lib/stores/schema-summary.svelte.ts';
  import { editorSchema } from './lib/stores/editor-schema.svelte.ts';
  import { connections } from './lib/stores/connections.svelte.ts';
  import { addRecentConnection } from './lib/stores/recent.js';
  import {
    chat,
    createConversation,
    addMessage,
    getActive,
    updateLast,
    pauseForPermission,
    resumeFromPermission,
    createNewTab as createNewChatTab,
    closeTab as closeChatTab,
    switchTab as switchChatTab,
  } from './lib/stores/chat.svelte.ts';
  import {
    createAiSession,
    sendMessage,
    executeDml,
    respondAgentPermission,
    rejectPendingDml,
  } from './lib/ipc/ai.ts';

  let connected = $state(false);
  let config = $state(null);
  let view = $state('dashboard');
  let showPalette = $state(false);
  let showAiSettings = $state(false);
  let showLogs = $state(false);
  let showChatPanel = $state(true);
  let hasTabs = $derived(tabs.length > 0);
  // Shown on the AI landing's context strip. Sourced from the connections
  // store rather than `config` because the sidebar's switcher calls
  // connectToProfile() directly without telling App — so `config` can be
  // stale, and a context strip naming the wrong database is worse than none.
  // `activeProfile` is null for inline connections, where `config` is the
  // only record of what we're attached to.
  let connectionName = $derived(connections.activeProfile?.name ?? null);
  let databaseName = $derived(
    connections.activeProfile?.params['database'] ??
      connections.activeProfile?.params['path'] ??
      config?.database ??
      null,
  );

  // Reloads the schema summary whenever the live database changes, covering
  // both the initial connect and a switch from the sidebar. untrack() keeps
  // the store's own $state out of this effect's dependencies, so settling a
  // load can't retrigger it.
  $effect(() => {
    if (!connected || !databaseName) return;
    const db = databaseName;
    untrack(() => schemaSummary.load(db));
  });

  // Reloads the editor-schema store (table/column autocomplete lists) on the
  // same connect/switch trigger as the schema summary above — the spec's
  // fetch-on-connect behavior for schema-aware autocomplete. untrack() keeps
  // the store's own $state out of this effect's dependencies, mirroring the
  // sibling effect.
  $effect(() => {
    if (!connected || !databaseName) return;
    // Track the active profile id so switching profiles retriggers the
    // refresh even when the database name is unchanged (two profiles can
    // share a database name while pointing at different hosts). The read
    // stays outside untrack() so it registers as a real dependency.
    void connections.activeProfileId;
    untrack(() => void editorSchema.refresh());
  });
  let sidebarWidth = $state(252);
  let sidebarCollapsed = $state(false);
  const SIDEBAR_RAIL_WIDTH = 64;
  let chatWidth = $state(380);
  let editorHeight = $state(220);
  let resizeTarget = $state(null);
  // Clears document-level drag listeners if the app unmounts mid-drag. Each
  // resize start registers its closure-local onUp here; onDestroy invokes it.
  let resizeCleanup = null;

  function startResize(target, e) {
    e.preventDefault();
    resizeTarget = target;
    const startX = e.clientX;
    const startW = target === 'sidebar' ? sidebarWidth : chatWidth;

    const onMove = (e) => {
      const d = e.clientX - startX;
      if (target === 'sidebar') {
        sidebarWidth = Math.max(180, Math.min(400, startW + d));
      } else {
        chatWidth = Math.max(280, Math.min(600, startW - d));
      }
      document.body.style.cursor =
        target === 'sidebar' ? 'col-resize' : 'col-resize';
      document.body.style.userSelect = 'none';
    };

    const onUp = () => {
      resizeTarget = null;
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      resizeCleanup = null;
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    resizeCleanup = onUp;
  }

  function startVResize(e) {
    e.preventDefault();
    resizeTarget = 'vsplit';
    const startY = e.clientY;
    const startH = editorHeight;

    const onMove = (e) => {
      const d = e.clientY - startY;
      editorHeight = Math.max(100, Math.min(600, startH + d));
      document.body.style.cursor = 'row-resize';
      document.body.style.userSelect = 'none';
    };

    const onUp = () => {
      resizeTarget = null;
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      resizeCleanup = null;
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    resizeCleanup = onUp;
  }
  let tabs = $state([]);
  let activeTabId = $state(null);
  let activeConnectionId = $state('');
  let session = $state(null);

  // source view state
  let currentObject = $state(null);
  let sourceContent = $state('');
  let sourceLoading = $state(false);
  let sourceError = $state(null);

  // view sub-navigation state
  let viewSubView = $state('data'); // "data" | "source"

  const theme = getTheme();

  // Derive active tab's display state
  let activeTab = $derived(tabs.find((t) => t.id === activeTabId) || null);

  onMount(() => {
    theme.init();
    void (async () => {
      unlistenMenu = await listen('menu-action', (e) => {
        void handleMenuAction(e.payload);
      });
    })();
    void initIndexingListeners();
  });

  let unlistenMenu = null;

  async function handleMenuAction(id) {
    switch (id) {
      case 'new-notebook':
        goToNotebook();
        return;
      case 'new-query':
        goToQuery();
        return;
      case 'open-notebook': {
        const path = await pickNotebookToOpen();
        if (path) await openNotebookFile(path);
        return;
      }
      case 'save': {
        if (activeTab?.kind === 'notebook') {
          await saveNotebook(activeTab.id);
          updateNotebookTabName(activeTab.id);
        } else if (activeTab?.kind === 'query') {
          const saved = await saveQueryTab(activeTab);
          if (saved) updateQueryTabPath(activeTab.id, saved);
        }
        return;
      }
      case 'save-as': {
        if (activeTab?.kind === 'notebook') {
          await saveNotebookAs(activeTab.id);
          updateNotebookTabName(activeTab.id);
        } else if (activeTab?.kind === 'query') {
          const saved = await saveQueryTabAs(activeTab);
          if (saved) updateQueryTabPath(activeTab.id, saved);
        }
        return;
      }
    }
  }

  /** After a save, sync the tab's display name with the file that was written. */
  function updateNotebookTabName(tabId) {
    const model = notebooks.get(tabId);
    if (!model?.filePath) return;
    // Guard above ensures filePath is non-empty, so pop() always returns a string.
    const newName = model.filePath.split('/').pop();
    tabs = tabs.map((t) =>
      t.id === tabId ? { ...t, name: newName, filePath: model.filePath } : t,
    );
  }

  /** After a query save, sync the tab's display name and path with the file written. */
  function updateQueryTabPath(tabId, path) {
    tabs = tabs.map((t) =>
      t.id === tabId
        ? { ...t, name: path.split('/').pop(), filePath: path }
        : t,
    );
  }

  async function openNotebookFile(path) {
    // Reuse an already-open tab for this file rather than opening a duplicate.
    const existing = tabs.find(
      (t) => t.kind === 'notebook' && t.filePath === path,
    );
    if (existing) {
      switchTab(existing.id);
      return;
    }
    goToNotebook(path);
  }

  onDestroy(() => {
    resizeCleanup?.();
    if (session?.cleanup) session.cleanup();
    unlistenMenu?.();
  });

  async function handleAiSend(message) {
    if (!chat.activeConversationId) {
      const conv = createConversation(activeConnectionId);
      chat.conversations = [...chat.conversations, conv];
      chat.activeConversationId = conv.id;
    }
    const convId = chat.activeConversationId;

    addMessage(convId, {
      id: crypto.randomUUID(),
      role: 'user',
      content: message,
      createdAt: Date.now(),
    });
    addMessage(convId, {
      id: crypto.randomUUID(),
      role: 'assistant',
      content: '',
      createdAt: Date.now(),
    });

    const aiSession = createAiSession(convId);
    session = aiSession;
    await aiSession.setupListeners({
      onDmlApproval: (p) => {
        const conv = getActive();
        if (conv) {
          conv.isPaused = true;
          conv.dmlResult = null;
          conv.dmlError = null;
          conv.pausedDml = {
            sql: p.sql,
            description: p.description,
            estimatedRowsAffected: p.estimated_rows_affected,
          };
          // Stamp the card onto the last (assistant) message so it renders
          // in the thread with Execute/Cancel (C1).
          updateLast(conv.id, {
            dmlApproval: {
              sql: p.sql,
              description: p.description,
              estimatedRowsAffected: p.estimated_rows_affected,
            },
          });
        }
      },
      onAgentPermission: (p) => {
        // The agent asks permission to run one of ITS tools — distinct from
        // the DML gate above (Lucent's own tool). Pause the conversation and
        // stamp the permission card onto the last assistant message (E4).
        pauseForPermission(p.conversationId, p);
      },
      onError: (p) => {
        chat.error = p.message;
      },
    });

    await sendMessage(message, aiSession.channel, convId, activeConnectionId);
  }

  async function handleAllowPermission() {
    const conv = getActive();
    if (!conv?.isPaused || !conv.pendingPermission) return;
    try {
      await respondAgentPermission(conv.id, true);
    } catch (e) {
      chat.error = formatError(e);
    } finally {
      // The backend resolves the pending permission and the agent turn
      // resumes streaming on its own — only the pause state clears.
      resumeFromPermission(conv.id);
    }
  }

  async function handleRejectPermission() {
    const conv = getActive();
    if (!conv?.isPaused || !conv.pendingPermission) return;
    try {
      await respondAgentPermission(conv.id, false);
    } catch (e) {
      chat.error = formatError(e);
    } finally {
      resumeFromPermission(conv.id);
    }
  }

  async function handleRunDml() {
    const conv = getActive();
    if (!conv?.isPaused || !chat.activeConversationId) return;
    try {
      const res = await executeDml(chat.activeConversationId);
      conv.dmlResult = res?.rows_affected ?? null;
    } catch (e) {
      conv.dmlError = formatError(e);
    } finally {
      conv.isPaused = false;
      conv.pausedDml = null;
    }
  }

  async function handleCancelDml() {
    if (!chat.activeConversationId) return;
    // ACP mode rejects the bridge-held preview through the backend (the
    // `dml:rejected` event clears the card); the rig path cancels the turn.
    // The provider branch lives in `rejectPendingDml` (ipc/ai.ts) so it is
    // testable without mounting App.svelte.
    await rejectPendingDml(chat.activeConversationId);
  }

  function handleKeydown(e) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
      e.preventDefault();
      showPalette = !showPalette;
    }
    // Cmd+Shift+N — new notebook
    if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === 'n') {
      e.preventDefault();
      goToNotebook();
    }
    // Cmd+Shift+A — toggle AI chat panel
    if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === 'a') {
      e.preventDefault();
      if (hasTabs) showChatPanel = !showChatPanel;
      else showChatPanel = true;
    }
    // Esc — cancel the in-flight editor query
    if (e.key === 'Escape' && queryRunning) {
      e.preventDefault();
      cancelQuery().catch(() => {});
    }
    // Cmd/Ctrl+. — cancel the in-flight editor query
    if ((e.metaKey || e.ctrlKey) && e.key === '.') {
      e.preventDefault();
      cancelQuery().catch(() => {});
    }
  }

  async function handleConnect(cfg) {
    connectError = null;
    try {
      if (!cfg.connectionId) {
        // Inline connection — need to connect to the backend first
        await connect(cfg);
      }
      // For profile-based connections, connectToProfile() already
      // established the connection. In both cases, update UI state.
      addRecentConnection(cfg);
      config = cfg;
      connected = true;
      // Key AI conversations by the profile id when there is one (a DuckDB
      // profile has no host:port/database to form the legacy key from).
      activeConnectionId =
        connections.activeProfileId ??
        `${cfg.host}:${cfg.port}/${cfg.database}`;
      view = 'dashboard';
    } catch (e) {
      connectError = formatError(e);
    }
  }

  async function handleDisconnect() {
    await disconnect();
    connected = false;
    config = null;
    // Drop the cached schema so the next connection reads its own.
    schemaSummary.reset();
    view = 'dashboard';
    for (const t of tabs) {
      if (t.kind === 'notebook') void notebooks.release(t.id);
    }
    tabs = [];
    activeTabId = null;
    queryError = null;
    currentObject = null;
    showPalette = false;
    viewSubView = 'data';
  }

  let queryError = $state(null);
  let connectError = $state(null);
  // Editor queries in flight (primary runs + paginated fetches can overlap
  // across tabs). Derived boolean drives the Stop button and shortcuts.
  let queryRunCount = $state(0);
  const queryRunning = $derived(queryRunCount > 0);

  function formatError(e) {
    if (typeof e === 'object' && e !== null && 'message' in e) {
      return e.message;
    }
    if (typeof e === 'string') return e;
    return 'Unknown error';
  }

  const CHUNK_SIZE = 200;

  // A fetch is complete when the backend truncated it (non-wrappable queries
  // can't be paged further) or the chunk came back short of a full page.
  function pageComplete(result) {
    return result.truncated === true || result.rows.length < CHUNK_SIZE;
  }

  async function handleExecute(sql) {
    queryError = null;
    const tabId = activeTab?.id;
    if (!tabId) return;
    const tab = tabs.find((t) => t.id === tabId);
    if (!tab || tab.kind !== 'query') return;

    const start = performance.now();
    queryRunCount += 1;
    try {
      const result = await executeQuery(sql, { limit: CHUNK_SIZE, offset: 0 });
      const elapsed = ((performance.now() - start) / 1000).toFixed(2);
      updateTab(tabId, {
        baseSql: sql,
        columns: result.columns,
        rows: result.rows,
        fetchedCount: result.rows.length,
        totalCount: null,
        isEnd: pageComplete(result),
        truncated: result.truncated === true,
        duration: parseFloat(elapsed),
        summary: result.rows_affected != null ? resultSummary(result) : null,
        error: null,
      });
    } catch (e) {
      const msg = formatError(e);
      updateTab(tabId, {
        error: msg,
        columns: [],
        rows: [],
        fetchedCount: 0,
        totalCount: null,
        isEnd: false,
        duration: 0,
        summary: null,
      });
      queryError = msg;
    } finally {
      queryRunCount -= 1;
    }
  }

  async function handleNeedMore(tabId) {
    const tab = tabs.find((t) => t.id === tabId);
    if (!tab || tab.isFetchingMore) return;
    updateTab(tabId, { isFetchingMore: true });
    queryRunCount += 1;
    try {
      const opts = fetchMoreOptions(tab, CHUNK_SIZE);
      const result =
        tab.kind === 'view' || tab.kind === 'table'
          ? await browseTable(tab.path ?? [], tab.name, opts)
          : await executeQuery(tab.baseSql, opts);
      updateTab(tabId, {
        rows: [...tab.rows, ...result.rows],
        fetchedCount: tab.fetchedCount + result.rows.length,
        isFetchingMore: false,
        isEnd: pageComplete(result),
        truncated: result.truncated === true,
        summary: result.rows_affected != null ? resultSummary(result) : null,
      });
    } catch (e) {
      updateTab(tabId, { isFetchingMore: false, error: formatError(e) });
    } finally {
      queryRunCount -= 1;
    }
  }

  async function handleGridStateChange(tabId, updates) {
    const tab = tabs.find((t) => t.id === tabId);
    if (!tab) return;
    // Keep the current rows on screen and mark the tab as refetching. Blanking
    // rows here used to unmount the grid's filter UI mid-interaction.
    updateTab(tabId, { ...updates, refetching: true });
    const merged = { ...tab, ...updates };
    queryRunCount += 1;
    try {
      const opts = refetchOptions(merged, CHUNK_SIZE);
      const result =
        merged.kind === 'view' || merged.kind === 'table'
          ? await browseTable(merged.path ?? [], merged.name, opts)
          : await executeQuery(merged.baseSql, opts);
      updateTab(tabId, {
        columns: result.columns,
        rows: result.rows,
        fetchedCount: result.rows.length,
        totalCount: null,
        isEnd: pageComplete(result),
        truncated: result.truncated === true,
        summary: result.rows_affected != null ? resultSummary(result) : null,
        error: null,
        refetching: false,
      });
    } catch (e) {
      updateTab(tabId, {
        error: formatError(e),
        rows: [],
        fetchedCount: 0,
        totalCount: null,
        summary: null,
        refetching: false,
      });
    } finally {
      queryRunCount -= 1;
    }
  }

  async function handleCountAll(tabId) {
    const tab = tabs.find((t) => t.id === tabId);
    if (!tab || tab.totalCount !== null) return;
    try {
      const sqlForCount =
        tab.kind === 'view' || tab.kind === 'table'
          ? `SELECT * FROM ${tab.schema}.${tab.name}`
          : tab.baseSql;
      const total = await countAllRows(sqlForCount, filterSpecFor(tab));
      // Direct proxy mutation — changes only the totalCount signal, avoiding a
      // tab object replacement that would cascade into Effects watching other props.
      const current = tabs.find((t) => t.id === tabId);
      if (current) current.totalCount = total;
    } catch (e) {
      const current = tabs.find((t) => t.id === tabId);
      if (current) current.error = formatError(e);
    }
  }

  async function handleDashboardQuery(sql) {
    return executeQuery(sql, { limit: CHUNK_SIZE, offset: 0 });
  }

  async function handleViewSubView(schema, name, path, subView, kind = 'view') {
    if (subView === 'data') {
      const existingTab = tabs.find(
        (t) => t.kind === 'view' && t.schema === schema && t.name === name,
      );
      if (existingTab && existingTab.columns.length > 0) return; // already loaded
      try {
        const result = await browseTable(path, name, {
          limit: CHUNK_SIZE,
          offset: 0,
        });
        updateTab(existingTab?.id || '', {
          columns: result.columns,
          rows: result.rows,
          fetchedCount: result.rows.length,
          totalCount: null,
          duration: 0,
          error: null,
        });
      } catch (e) {
        updateTab(existingTab?.id || '', { error: formatError(e) });
      }
    } else {
      const existingTab = tabs.find(
        (t) => t.kind === 'view' && t.schema === schema && t.name === name,
      );
      if (existingTab?.sourceContent) return; // already loaded
      try {
        const src = await getViewSource(path, name, kind);
        updateTab(existingTab?.id || '', {
          sourceContent: src,
          sourceError: null,
        });
      } catch (e) {
        updateTab(existingTab?.id || '', { sourceError: formatError(e) });
      }
    }
  }

  function switchViewSubView(tabId, subView) {
    viewSubView = subView;
    const tab = tabs.find((t) => t.id === tabId);
    if (tab && tab.kind === 'view') {
      updateTab(tabId, { subView });
      handleViewSubView(
        tab.schema,
        tab.name,
        tab.path ?? [],
        subView,
        tab.sourceKind || 'view',
      );
    }
  }

  async function handleObjectClick({ schema, path = [], name, kind }) {
    if (kind === 'function' || kind === 'sequence') {
      const tabId = crypto.randomUUID();
      currentObject = { schema, name, kind };
      view = 'source';
      sourceLoading = true;
      sourceError = null;
      sourceContent = '';
      try {
        if (kind === 'function') {
          sourceContent = await getFunctionSource(path, name);
        } else {
          const props = await getSequenceInfo(path, name);
          sourceContent = [
            `-- Sequence: ${schema}.${name}`,
            `--`,
            ...props.map((p) => `-- ${p.key}: ${p.value}`),
          ].join('\n');
        }
      } catch (e) {
        sourceError = formatError(e);
      } finally {
        sourceLoading = false;
      }
      const newTab = {
        id: tabId,
        kind: 'source',
        schema,
        path,
        name,
        columns: [],
        rows: [],
        fetchedCount: 0,
        totalCount: null,
        duration: 0,
        filters: [],
        sortCol: null,
        sortDir: 'asc',
        error: null,
      };
      if (!tabs.find((t) => t.id === tabId)) {
        tabs = [...tabs, newTab];
        activeTabId = tabId;
      }
      showChatPanel = true;
    } else if (kind === 'view' || kind === 'matview') {
      const existingTab = tabs.find(
        (t) => t.kind === 'view' && t.schema === schema && t.name === name,
      );
      if (existingTab) {
        activeTabId = existingTab.id;
        view = 'view';
        viewSubView = existingTab.subView || 'data';
        return;
      }

      const tabId = crypto.randomUUID();
      viewSubView = 'data';
      const newTab = {
        id: tabId,
        kind: 'view',
        sourceKind: kind,
        subView: 'data',
        schema,
        path,
        name,
        columns: [],
        rows: [],
        fetchedCount: 0,
        totalCount: null,
        isFetchingMore: false,
        isEnd: false,
        baseSql: '',
        duration: 0,
        error: null,
        sourceContent: '',
        sourceError: null,
        filters: [],
        sortCol: null,
        sortDir: 'asc',
      };
      tabs = [...tabs, newTab];
      activeTabId = tabId;
      view = 'view';

      // Fetch data immediately
      handleViewSubView(schema, name, path, 'data', kind);
      // Also fetch source in background
      handleViewSubView(schema, name, path, 'source', kind);
    } else {
      const existingTab = tabs.find(
        (t) => t.kind === 'table' && t.schema === schema && t.name === name,
      );
      if (existingTab) {
        activeTabId = existingTab.id;
        view = 'table';
        queryError = null;
        return;
      }

      queryError = null;
      const start = performance.now();
      try {
        const result = await browseTable(path, name, {
          limit: CHUNK_SIZE,
          offset: 0,
        });
        const elapsed = ((performance.now() - start) / 1000).toFixed(2);
        const newTab = {
          id: crypto.randomUUID(),
          kind: 'table',
          schema,
          path,
          name,
          columns: result.columns,
          rows: result.rows,
          fetchedCount: result.rows.length,
          totalCount: null,
          isFetchingMore: false,
          isEnd: result.rows.length < CHUNK_SIZE,
          baseSql: '',
          duration: parseFloat(elapsed),
          filters: [],
          sortCol: null,
          sortDir: 'asc',
        };
        tabs = [...tabs, newTab];
        activeTabId = newTab.id;
        view = 'table';
      } catch (e) {
        queryError = formatError(e);
        view = 'dashboard';
      }
    }
  }

  function goToQuery() {
    const queryCount = tabs.filter((t) => t.kind === 'query').length;
    const newTab = {
      id: crypto.randomUUID(),
      kind: 'query',
      name: `query_${queryCount + 1}.sql`,
      filePath: null,
      columns: [],
      rows: [],
      fetchedCount: 0,
      totalCount: null,
      isFetchingMore: false,
      isEnd: false,
      baseSql: '',
      duration: 0,
      filters: [],
      sortCol: null,
      sortDir: 'asc',
      summary: null,
      error: null,
    };
    tabs = [...tabs, newTab];
    activeTabId = newTab.id;
    view = 'query';
    queryError = null;
    showPalette = false;
    showChatPanel = true;
  }

  function goToNotebook(filePath = null) {
    const tabId = crypto.randomUUID();
    const newTab = {
      id: tabId,
      kind: 'notebook',
      name: filePath ? filePath.split('/').pop() : 'Untitled.lucent',
      filePath: filePath,
    };
    tabs = [...tabs, newTab];
    activeTabId = tabId;
    view = 'notebook';
    showChatPanel = true;
    showPalette = false;
  }

  function handlePaletteSelect(item) {
    if (item.id === 'new-query') goToQuery();
    if (item.id === 'new-notebook') goToNotebook();
    if (item.id === 'toggle-theme') theme.toggle();
    if (item.id === 'disconnect') handleDisconnect();
    if (item.id === 'toggle-ai-chat') {
      if (hasTabs) showChatPanel = !showChatPanel;
      else showChatPanel = true;
    }
    showPalette = false;
  }

  function switchTab(tabId) {
    const tab = tabs.find((t) => t.id === tabId);
    if (!tab) return;
    activeTabId = tabId;
    if (tab.kind === 'query') view = 'query';
    else if (tab.kind === 'notebook') view = 'notebook';
    else if (tab.kind === 'source') view = 'source';
    else if (tab.kind === 'view') {
      view = 'view';
      viewSubView = tab.subView || 'data';
      return;
    } else view = 'table';
    queryError = tab.error || null;
  }

  async function closeTab(tabId) {
    const tab = tabs.find((t) => t.id === tabId);
    if (tab?.kind === 'notebook') {
      const intent = await confirmDiscardIfDirty(tabId);
      if (intent === 'cancel') return;
      if (intent === 'save') {
        const saved = await saveNotebook(tabId);
        if (!saved) return; // save cancelled or failed — abort the close
      }
      await notebooks.release(tabId);
    }
    tabs = tabs.filter((t) => t.id !== tabId);
    if (activeTabId === tabId) {
      if (tabs.length > 0) {
        switchTab(tabs[tabs.length - 1].id);
      } else {
        activeTabId = null;
      }
    }
  }

  async function closeMultipleTabs(tabIds) {
    for (const id of tabIds) {
      await closeTab(id);
    }
  }

  function updateTab(tabId, updates) {
    tabs = tabs.map((t) => (t.id === tabId ? { ...t, ...updates } : t));
  }

  const paletteCommands = $derived([
    {
      id: 'new-query',
      label: 'New Query',
      description: 'Open the SQL editor',
      icon: 'terminal',
      shortcut: 'Cmd+Enter',
    },
    {
      id: 'new-notebook',
      label: 'New Notebook',
      description: 'Create a new SQL notebook',
      icon: 'notebook',
      shortcut: '⌘⇧N',
    },
    {
      id: 'toggle-theme',
      label: 'Toggle Theme',
      description: `Switch to ${theme.current === 'light' ? 'dark' : 'light'} mode`,
      icon: 'theme',
    },
    ...(connected
      ? [
          {
            id: 'toggle-ai-chat',
            label: showChatPanel ? 'Close AI Chat' : 'Open AI Chat',
            description: 'Toggle the AI Copilot panel',
            icon: 'sparkles',
            shortcut: '⌘⇧A',
          },
          {
            id: 'disconnect',
            label: 'Disconnect',
            description: 'Close the current connection',
            icon: 'unplug',
          },
        ]
      : []),
  ]);
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="app">
  <AppHeader
    {config}
    {connected}
    {showAiSettings}
    {showChatPanel}
    {showLogs}
    {hasTabs}
    connectionId={activeConnectionId}
    leftWidth={sidebarCollapsed ? SIDEBAR_RAIL_WIDTH : sidebarWidth}
    {sidebarCollapsed}
    onToggleSidebar={() => (sidebarCollapsed = !sidebarCollapsed)}
    onToggleTheme={() => theme.toggle()}
    onToggleAi={() => (showAiSettings = !showAiSettings)}
    onToggleLogs={() => (showLogs = !showLogs)}
    onToggleChat={() => {
      if (hasTabs) showChatPanel = !showChatPanel;
    }}
    onTogglePalette={() => (showPalette = true)}
    onOpenChat={() => (showChatPanel = true)}
    {tabs}
    {activeTabId}
    {view}
    onSwitchTab={switchTab}
    onCloseTab={closeTab}
    onCloseTabs={closeMultipleTabs}
    onNewQuery={goToQuery}
    onNotebookSave={async (id) => {
      const tab = tabs.find((t) => t.id === id);
      if (tab?.kind === 'notebook') {
        await saveNotebook(id);
        updateNotebookTabName(id);
      } else if (tab?.kind === 'query') {
        const saved = await saveQueryTab(tab);
        if (saved) updateQueryTabPath(id, saved);
      }
    }}
    onNotebookSaveAs={async (id) => {
      const tab = tabs.find((t) => t.id === id);
      if (tab?.kind === 'notebook') {
        await saveNotebookAs(id);
        updateNotebookTabName(id);
      } else if (tab?.kind === 'query') {
        const saved = await saveQueryTabAs(tab);
        if (saved) updateQueryTabPath(id, saved);
      }
    }}
    onNotebookOpen={async () => {
      const path = await pickNotebookToOpen();
      if (path) await openNotebookFile(path);
    }}
    isTabDirty={(id) => notebooks.get(id)?.isDirty ?? false}
  />

  {#if connected}
    <div
      class="main-layout"
      class:resizing-sidebar={resizeTarget === 'sidebar'}
      class:resizing-chat={resizeTarget === 'chat'}
      class:has-tabs={hasTabs}
    >
      {#if !sidebarCollapsed}
        <div class="sidebar-wrap" style="width:{sidebarWidth}px">
          <Sidebar
            onObjectClick={handleObjectClick}
            onDisconnect={handleDisconnect}
            onOpenLogs={() => (showLogs = true)}
          />
          <div
            class="resize-handle sidebar-handle"
            onmousedown={(e) => startResize('sidebar', e)}
          />
        </div>
      {/if}

      {#if hasTabs}
        <!-- Content area -->
        <div class="content-area">
          {#if view === 'query' && activeTab}
            <div class="vsplit" class:resizing={resizeTarget === 'vsplit'}>
              <div class="vsplit-top" style="height:{editorHeight}px">
                <QueryEditor
                  onExecute={handleExecute}
                  tabId={activeTab.id}
                  content={activeTab.baseSql || ''}
                  onContentChange={(val) => {
                    if (activeTab) activeTab.baseSql = val;
                  }}
                  isRunning={queryRunning}
                  onCancel={() => cancelQuery().catch(() => {})}
                />
              </div>
              <div class="vsplit-handle" onmousedown={startVResize}></div>
              <div class="vsplit-bottom">
                <ResultsGrid
                  columns={activeTab.columns}
                  rows={activeTab.rows}
                  fetchedCount={activeTab.fetchedCount}
                  totalCount={activeTab.totalCount}
                  isEnd={activeTab.isEnd}
                  truncated={activeTab.truncated}
                  duration={activeTab.duration}
                  error={activeTab.error}
                  tabId={activeTab.id}
                  initFilters={activeTab.filters}
                  initSortCol={activeTab.sortCol}
                  initSortDir={activeTab.sortDir}
                  compact={showChatPanel}
                  loading={activeTab.refetching || false}
                  summary={activeTab.summary}
                  onDescribeFilters={describeFilters}
                  onStateChange={(updates) =>
                    handleGridStateChange(activeTab.id, updates)}
                  onNeedMore={() => handleNeedMore(activeTab.id)}
                  onCountAll={() => handleCountAll(activeTab.id)}
                />
              </div>
            </div>
          {:else if view === 'table' && activeTab}
            <ResultsGrid
              columns={activeTab.columns}
              rows={activeTab.rows}
              fetchedCount={activeTab.fetchedCount}
              totalCount={activeTab.totalCount}
              isEnd={activeTab.isEnd}
              truncated={activeTab.truncated}
              duration={activeTab.duration}
              error={activeTab.error}
              tabId={activeTab.id}
              initFilters={activeTab.filters}
              initSortCol={activeTab.sortCol}
              initSortDir={activeTab.sortDir}
              compact={showChatPanel}
              loading={activeTab.refetching || false}
              onDescribeFilters={describeFilters}
              onStateChange={(updates) =>
                handleGridStateChange(activeTab.id, updates)}
              onNeedMore={() => handleNeedMore(activeTab.id)}
              onCountAll={() => handleCountAll(activeTab.id)}
            />
          {:else if view === 'view' && activeTab}
            <div class="view-sub-bar">
              <button
                class="sub-tab"
                class:active={viewSubView === 'data'}
                onclick={() => switchViewSubView(activeTab.id, 'data')}
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
                >
                  <rect x="3" y="3" width="18" height="18" rx="2" /><path
                    d="M3 9h18"
                  /><path d="M3 15h18" /><path d="M9 3v18" />
                </svg>
                <span>Data</span>
              </button>
              <button
                class="sub-tab"
                class:active={viewSubView === 'source'}
                onclick={() => switchViewSubView(activeTab.id, 'source')}
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
                >
                  <path
                    d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"
                  /><polyline points="14 2 14 8 20 8" /><line
                    x1="16"
                    y1="13"
                    x2="8"
                    y2="13"
                  /><line x1="16" y1="17" x2="8" y2="17" /><polyline
                    points="10 9 9 9 8 9"
                  />
                </svg>
                <span>Source</span>
              </button>
            </div>

            {#if viewSubView === 'data'}
              <div class="table-header">
                <span class="table-title"
                  >{activeTab.schema}.{activeTab.name}</span
                >
                <span class="table-badge"
                  >View &middot; {activeTab.fetchedCount} rows</span
                >
              </div>
              <ResultsGrid
                columns={activeTab.columns}
                rows={activeTab.rows}
                fetchedCount={activeTab.fetchedCount}
                totalCount={activeTab.totalCount}
                isEnd={activeTab.isEnd}
                truncated={activeTab.truncated}
                duration={activeTab.duration}
                error={activeTab.error}
                tabId={activeTab.id}
                initFilters={activeTab.filters}
                initSortCol={activeTab.sortCol}
                initSortDir={activeTab.sortDir}
                compact={showChatPanel}
                loading={activeTab.refetching || false}
                onDescribeFilters={describeFilters}
                onStateChange={(updates) =>
                  handleGridStateChange(activeTab.id, updates)}
                onNeedMore={() => handleNeedMore(activeTab.id)}
                onCountAll={() => handleCountAll(activeTab.id)}
              />
            {:else}
              <SourceView
                title={`${activeTab.schema}.${activeTab.name} (${activeTab.sourceKind || 'view'})`}
                source={activeTab.sourceContent || ''}
                loading={false}
                error={activeTab.sourceError || null}
              />
            {/if}
          {:else if view === 'notebook' && activeTab}
            <Notebook
              tabId={activeTab.id}
              filePath={activeTab.filePath}
              connectionId={connections.activeProfileId ?? activeConnectionId}
              database={databaseName}
            />
          {:else if view === 'source' && activeTab}
            <SourceView
              title={currentObject
                ? `${currentObject.schema}.${currentObject.name} (${currentObject.kind})`
                : ''}
              source={sourceContent}
              loading={sourceLoading}
              error={sourceError}
            />
          {/if}
        </div>
        <!-- Chat panel (right side) -->
        {#if showChatPanel}
          <div
            class="resize-handle chat-handle"
            onmousedown={(e) => startResize('chat', e)}
          />
          <div class="chat-wrap" style="width:{chatWidth}px">
            <ChatPanel
              onSend={handleAiSend}
              onRunDml={handleRunDml}
              onCancelDml={handleCancelDml}
              onAllowPermission={handleAllowPermission}
              onRejectPermission={handleRejectPermission}
              {connected}
              database={databaseName}
              {connectionName}
              onOpenSettings={() => (showAiSettings = true)}
              onClose={() => (showChatPanel = false)}
              onNewChat={() => {
                createNewChatTab(activeConnectionId);
                showChatPanel = true;
              }}
              onSwitchConv={(id) => {
                switchChatTab(id);
                showChatPanel = true;
              }}
              onCloseConv={closeChatTab}
            />
          </div>
        {/if}
      {:else}
        <!-- Chat full width (no tabs open) -->
        <div class="chat-full">
          <ChatPanel
            onSend={handleAiSend}
            onRunDml={handleRunDml}
            onCancelDml={handleCancelDml}
            onAllowPermission={handleAllowPermission}
            onRejectPermission={handleRejectPermission}
            {connected}
            database={databaseName}
            {connectionName}
            onOpenSettings={() => (showAiSettings = true)}
            onNewChat={() => createNewChatTab(activeConnectionId)}
            onSwitchConv={switchChatTab}
            onCloseConv={closeChatTab}
          />
        </div>
      {/if}
    </div>
  {:else}
    <div class="landing-full">
      <LandingPage onConnect={handleConnect} {connectError} />
    </div>
  {/if}

  {#if showAiSettings}
    <div class="modal-overlay" onclick={() => (showAiSettings = false)}>
      <div class="modal-content" onclick={(e) => e.stopPropagation()}>
        <AiSettings onClose={() => (showAiSettings = false)} />
      </div>
    </div>
  {/if}
</div>

{#if queryError}
  <div class="toast toast-error" onclick={() => (queryError = null)}>
    {queryError}
  </div>
{/if}
{#if chat.error}
  <div class="toast toast-error" onclick={() => (chat.error = null)}>
    {chat.error}
  </div>
{/if}

{#if showPalette}
  <CommandPalette
    commands={paletteCommands}
    onSelect={handlePaletteSelect}
    onClose={() => (showPalette = false)}
  />
{/if}

{#if showLogs}
  <LogsDrawer onClose={() => (showLogs = false)} />
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }
  /* Header styles moved to AppHeader.svelte */
  .main-layout {
    display: flex;
    flex: 1;
    overflow: hidden;
  }
  .main-layout.resizing-sidebar,
  .main-layout.resizing-chat {
    cursor: col-resize;
  }
  .sidebar-wrap {
    display: flex;
    flex-shrink: 0;
    position: relative;
    overflow: hidden;
  }
  .sidebar-wrap :global(.sidebar) {
    width: 100%;
  }
  .chat-wrap {
    display: flex;
    flex-shrink: 0;
    position: relative;
  }
  .chat-wrap :global(.panel) {
    width: 100% !important;
    max-width: 100% !important;
    min-width: 0 !important;
  }
  .resize-handle {
    width: 4px;
    cursor: col-resize;
    flex-shrink: 0;
    position: relative;
    z-index: 10;
    background: transparent;
    transition: background var(--transition-fast);
  }
  .resize-handle:hover {
    background: var(--accent);
  }
  .resizing-sidebar .sidebar-handle {
    background: var(--accent);
  }
  .resizing-chat .chat-handle {
    background: var(--accent);
  }
  .content-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .view-sub-bar {
    display: flex;
    align-items: center;
    gap: 0;
    padding: 0 var(--space-2);
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    flex-shrink: 0;
  }
  .sub-tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    font-size: var(--text-sm);
    color: var(--text-secondary);
    cursor: pointer;
    border: none;
    background: transparent;
    border-bottom: 2px solid transparent;
    white-space: nowrap;
    transition: all var(--transition-fast);
    font-weight: 500;
  }
  .sub-tab:hover {
    color: var(--text);
    background: var(--bg-hover);
  }
  .sub-tab.active {
    color: var(--accent);
    background: var(--accent-soft);
    border-bottom-color: var(--accent);
  }
  .sub-tab svg {
    flex-shrink: 0;
  }
  .table-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border);
  }
  .table-title {
    font-size: var(--text-md);
    font-weight: var(--weight-semibold);
    color: var(--text);
    font-family: var(--font-mono);
  }
  .table-badge {
    font-size: var(--text-sm);
    color: var(--text-secondary);
    background: var(--bg-hover);
    padding: 2px 8px;
    border-radius: var(--radius-sm);
  }
  .vsplit {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .vsplit.resizing {
    cursor: row-resize;
  }
  .vsplit.resizing .vsplit-bottom,
  .vsplit.resizing .vsplit-top {
    pointer-events: none;
  }
  .vsplit-top {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .vsplit-top :global(.query-editor) {
    flex: 1;
    min-height: 0;
  }
  .vsplit-bottom {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .vsplit-handle {
    height: 4px;
    cursor: row-resize;
    flex-shrink: 0;
    position: relative;
    z-index: 10;
    background: transparent;
    transition: background var(--transition-fast);
  }
  .vsplit-handle:hover {
    background: var(--accent);
  }
  .vsplit.resizing .vsplit-handle {
    background: var(--accent);
  }

  .toast {
    position: fixed;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 1000;
    padding: 10px 20px;
    border-radius: var(--radius-md);
    font-size: 13px;
    cursor: pointer;
    max-width: 600px;
    word-break: break-word;
  }
  .toast-error {
    background: var(--danger-bg, rgba(239, 68, 68, 0.1));
    color: var(--danger, #ef4444);
    border: 1px solid var(--danger, #ef4444);
  }

  .chat-full {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--bg);
  }
  .chat-full :global(.panel) {
    width: 100% !important;
    max-width: 100% !important;
    min-width: 0 !important;
    border-left: none !important;
  }
  .chat-full :global(.messages) {
    max-width: 800px;
    margin: 0 auto;
    width: 100%;
  }
  .chat-full :global(.input-area) {
    max-width: 800px;
    margin: 0 auto;
    width: 100%;
  }
  .chat-full :global(header) {
    max-width: 800px;
    margin: 0 auto;
    width: 100%;
  }
  .landing-full {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: auto;
    background: var(--bg);
    width: 100%;
    height: 100%;
  }
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
  }
  .modal-content {
    background: var(--bg);
    border-radius: var(--radius-lg);
    padding: 0;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
  }
</style>
