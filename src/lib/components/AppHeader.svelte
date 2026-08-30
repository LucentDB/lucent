<script lang="ts">
  import { connections } from '../stores/connections.svelte';
  import { indexing } from '../stores/indexing.svelte';
  import ReadOnlyBadge from './connection/ReadOnlyBadge.svelte';
  import TabContextMenu from './TabContextMenu.svelte';
  import type { TabMenuItem } from './tab-menu.ts';
  import DbIcon from './icons/DbIcon.svelte';
  import UpdateBanner from './UpdateBanner.svelte';

  let {
    config = null,
    connected = false,
    showAiSettings = false,
    showChatPanel = false,
    showLogs = false,
    hasTabs = false,
    leftWidth = 0,
    sidebarCollapsed = false,
    onToggleSidebar = undefined,
    onToggleTheme = undefined,
    onToggleAi = undefined,
    onToggleLogs = undefined,
    onToggleChat = undefined,
    onTogglePalette = undefined,
    // unified tab bar props
    tabs = [],
    activeTabId = '',
    view = 'query',
    onSwitchTab = undefined,
    onCloseTab = undefined,
    onNewQuery = undefined,
    // notebook file actions
    onNotebookSave = undefined,
    onNotebookSaveAs = undefined,
    onNotebookOpen = undefined,
    isTabDirty = (_id: string) => false,
    // batch close callbacks
    onCloseTabs = undefined,
  } = $props();

  let tabsEl: HTMLDivElement;
  let contextMenu = $state<{
    x: number;
    y: number;
    tabId: string;
    kind: string | null;
  } | null>(null);

  function tabIconSvg(tab: any): string {
    if (tab.kind === 'query') return 'query';
    if (tab.kind === 'table') return 'table';
    if (tab.kind === 'view') return 'view';
    if (tab.kind === 'notebook') return 'notebook';
    if (tab.kind === 'source' && tab.sourceObjectKind)
      return tab.sourceObjectKind;
    return 'source';
  }
  function tabLabel(tab: any) {
    if (tab.kind === 'query') return tab.name;
    if (tab.kind === 'source') return `${tab.schema}.${tab.name}`;
    return tab.name;
  }

  function handleNewDbTab() {
    onNewQuery?.();
  }

  function isActive(tabId: string) {
    return activeTabId === tabId;
  }

  function closeContextMenu() {
    contextMenu = null;
  }

  function handleContextMenu(
    e: MouseEvent,
    tabId: string,
    kind: string | null = null,
  ) {
    e.preventDefault();
    e.stopPropagation();
    contextMenu = { x: e.clientX, y: e.clientY, tabId, kind };
  }

  // Close context menu on any click outside
  function handleGlobalClick() {
    closeContextMenu();
  }

  // ── Context menu actions ──────────────────────────────
  // Chat tabs run the same items through ChatPanel; both tab strips build
  // a TabMenuItem[] and render it through the shared TabContextMenu.

  function closeOtherTabs(tabId: string) {
    onCloseTabs?.(
      tabs.filter((t: any) => t.id !== tabId).map((t: any) => t.id),
    );
  }

  function closeTabsToRight(tabId: string) {
    const ids = tabs.map((t: any) => t.id);
    const idx = ids.indexOf(tabId);
    if (idx === -1) return;
    onCloseTabs?.(ids.slice(idx + 1));
  }

  function closeTabsToLeft(tabId: string) {
    const ids = tabs.map((t: any) => t.id);
    const idx = ids.indexOf(tabId);
    if (idx === -1) return;
    onCloseTabs?.(ids.slice(0, idx));
  }

  function closeAllTabs() {
    onCloseTabs?.(tabs.map((t: any) => t.id));
  }

  const contextMenuItems = $derived.by<TabMenuItem[]>(() => {
    const cm = contextMenu;
    if (!cm) return [];
    const items: TabMenuItem[] = [];
    if (cm.kind === 'notebook' || cm.kind === 'query') {
      items.push(
        {
          label: 'Save',
          icon: 'save',
          shortcut: '⌘S',
          action: () => onNotebookSave?.(cm.tabId),
        },
        {
          label: 'Save As…',
          icon: 'save-as',
          shortcut: '⇧⌘S',
          action: () => onNotebookSaveAs?.(cm.tabId),
        },
      );
      if (cm.kind === 'notebook') {
        items.push(
          { separator: true },
          {
            label: 'Open Notebook…',
            icon: 'open-notebook',
            shortcut: '⌘O',
            action: () => onNotebookOpen?.(),
          },
        );
      }
    }
    items.push(
      {
        label: 'Close Tab',
        icon: 'close',
        action: () => onCloseTab?.(cm.tabId),
      },
      {
        label: 'Close Others',
        icon: 'close-others',
        action: () => closeOtherTabs(cm.tabId),
      },
      {
        label: 'Close to the Right',
        icon: 'close-right',
        action: () => closeTabsToRight(cm.tabId),
      },
      {
        label: 'Close to the Left',
        icon: 'close-left',
        action: () => closeTabsToLeft(cm.tabId),
      },
      { separator: true },
      {
        label: 'Close All',
        icon: 'close-all',
        action: () => closeAllTabs(),
      },
    );
    return items;
  });

  $effect(() => {
    if (tabsEl && activeTabId) {
      const activeEl = tabsEl.querySelector(
        '.tab.active',
      ) as HTMLElement | null;
      activeEl?.scrollIntoView({
        behavior: 'smooth',
        block: 'nearest',
        inline: 'nearest',
      });
    }
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="app-header"
  onclick={handleGlobalClick}
  oncontextmenu={() => closeContextMenu()}
>
  <header>
    <div class="brand" style={connected ? `width:${leftWidth}px` : ''}>
      {#if connected}
        {#if !sidebarCollapsed}
          <span class="db-icon">⌬</span>
          <span class="db-name">{config?.database || 'database'}</span>
          <ReadOnlyBadge capabilities={connections.capabilities} />
        {/if}
        <span class="brand-spacer"></span>
        <button
          class="icon-btn sidebar-toggle"
          onclick={onToggleSidebar}
          title={sidebarCollapsed ? 'Show sidebar' : 'Hide sidebar'}
          aria-label={sidebarCollapsed ? 'Show sidebar' : 'Hide sidebar'}
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <rect x="3" y="3" width="18" height="18" rx="2" />
            <line x1="9" y1="3" x2="9" y2="21" />
          </svg>
        </button>
      {:else}
        <span class="app-name">Lucent</span>
      {/if}
    </div>

    <div class="spacer">
      {#if connected && tabs.length > 0}
        <div class="unified-tabs" bind:this={tabsEl}>
          <!-- DB tabs (query / table / source) -->
          {#each tabs as tab (tab.id)}
            <button
              class="tab"
              class:active={isActive(tab.id)}
              onclick={() => {
                onSwitchTab?.(tab.id);
                closeContextMenu();
              }}
              oncontextmenu={(e) => handleContextMenu(e, tab.id, tab.kind)}
              title={tabLabel(tab)}
            >
              <span class="tab-icon {tabIconSvg(tab)}">
                <DbIcon kind={tabIconSvg(tab)} size={12.5} strokeWidth={1.6} />
              </span>
              <span class="tab-label">{tabLabel(tab)}</span>
              {#if isTabDirty(tab.id)}
                <span
                  class="dirty-dot"
                  title="Unsaved changes"
                  aria-label="Unsaved changes"
                ></span>
              {/if}
              <span
                class="tab-close"
                onclick={(e) => {
                  e.stopPropagation();
                  onCloseTab?.(tab.id);
                }}
                onkeydown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.stopPropagation();
                    onCloseTab?.(tab.id);
                  }
                }}
                role="button"
                tabindex="-1"
                aria-label="Close tab">×</span
              >
            </button>
          {/each}
        </div>

        <div class="new-tab-group">
          <button
            class="new-tab-btn"
            onclick={handleNewDbTab}
            title="New Query"
            aria-label="New Query">+</button
          >
        </div>
      {/if}
    </div>

    {#if indexing.visible}
      <div
        class="indexing-indicator"
        title="Semantic schema index (background)"
      >
        <span class="spinner" aria-hidden="true"></span>
        <span class="indexing-text">
          {#if indexing.connections > 1}
            Indexing {indexing.connections} connections…{indexing.text}
          {:else}
            {indexing.text}
          {/if}
        </span>
        <span class="indexing-bar"
          ><span style="width: {indexing.percent}%"></span></span
        >
      </div>
    {/if}

    <div class="actions">
      <button
        class="icon-btn ai-btn"
        onclick={onToggleAi}
        title="AI Settings"
        aria-label="AI Settings"
      >
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M12 3l1.5 5.5L19 10l-5.5 1.5L12 17l-1.5-5.5L5 10l5.5-1.5z" />
          <path d="M18 14l.6 2.4L21 17l-2.4.6L18 20l-.6-2.4L15 17l2.4-.6z" />
        </svg>
      </button>

      <button
        class="icon-btn logs-toggle"
        class:active={showLogs}
        onclick={onToggleLogs}
        title="Worker logs"
        aria-label="Worker logs"
      >
        <svg
          width="16"
          height="16"
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
      </button>

      {#if connected}
        <button
          class="search-btn"
          onclick={onTogglePalette}
          aria-label="Search command palette"
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
            <circle cx="11" cy="11" r="8" /><path d="M21 21l-4.35-4.35" />
          </svg>
          <span>Search</span>
          <span class="kbd">⌘K</span>
        </button>

        <button
          class="icon-btn"
          onclick={onToggleTheme}
          title="Toggle theme"
          aria-label="Toggle theme"
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <circle cx="12" cy="12" r="5" />
            <line x1="12" y1="1" x2="12" y2="3" /><line
              x1="12"
              y1="21"
              x2="12"
              y2="23"
            />
            <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" /><line
              x1="18.36"
              y1="18.36"
              x2="19.78"
              y2="19.78"
            />
            <line x1="1" y1="12" x2="3" y2="12" /><line
              x1="21"
              y1="12"
              x2="23"
              y2="12"
            />
            <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" /><line
              x1="18.36"
              y1="5.64"
              x2="19.78"
              y2="4.22"
            />
          </svg>
        </button>

        <button
          class="icon-btn chat-toggle"
          class:active={showChatPanel}
          class:full={!hasTabs}
          onclick={onToggleChat}
          title={hasTabs ? 'Toggle AI Chat' : 'AI Chat'}
          aria-label={hasTabs ? 'Toggle AI Chat' : 'AI Chat'}
        >
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path
              d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"
            />
          </svg>
        </button>
      {/if}

      <UpdateBanner />
    </div>
  </header>

  {#if contextMenu}
    <TabContextMenu
      x={contextMenu.x}
      y={contextMenu.y}
      items={contextMenuItems}
      onClose={closeContextMenu}
    />
  {/if}
</div>

<style>
  .app-header {
    position: relative;
    user-select: none;
  }
  header {
    height: 46px;
    padding: 0 14px 0 0;
    display: flex;
    align-items: center;
    gap: 12px;
    background: rgba(255, 255, 255, 0.82);
    backdrop-filter: blur(24px) saturate(180%);
    -webkit-backdrop-filter: blur(24px) saturate(180%);
    border-bottom: 1px solid rgba(0, 0, 0, 0.08);
    user-select: none;
    flex-shrink: 0;
    z-index: 100;
    transition: background var(--transition-normal);
  }
  :global(.dark) header {
    background: rgba(13, 13, 18, 0.85);
    border-bottom-color: rgba(255, 255, 255, 0.06);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
    height: 100%;
    padding: 0 6px 0 12px;
    box-sizing: border-box;
    overflow: hidden;
  }
  .app-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .db-icon {
    font-size: 16px;
    color: var(--accent);
    flex-shrink: 0;
  }
  .db-name {
    font-size: 12px;
    font-weight: 500;
    color: var(--text);
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex-shrink: 1;
  }
  .brand-spacer {
    flex: 1;
    min-width: 4px;
  }

  .spacer {
    flex: 1;
    min-width: 28px;
    height: 100%;
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .unified-tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: 1 1 auto;
    height: 100%;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }
  .unified-tabs::-webkit-scrollbar {
    display: none;
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 10px;
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    min-width: 0;
    max-width: 180px;
    user-select: none;
    -webkit-user-select: none;
    transition: all var(--transition-fast);
  }
  .tab:hover {
    color: var(--text);
    background: var(--bg-hover);
  }
  .tab.active {
    color: var(--text);
    background: var(--bg-elevated);
    border-color: var(--border);
    box-shadow:
      var(--shadow-sm),
      0 0 0 1px color-mix(in srgb, var(--accent) 10%, transparent);
    font-weight: 550;
  }
  .tab-icon {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 13px;
    height: 13px;
    line-height: 1;
  }
  .tab-icon :global(svg) {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
  }
  .tab-icon.query {
    color: var(--accent);
  }
  .tab-icon.table {
    color: #10b981;
  }
  .tab-icon.view {
    color: #6366f1;
  }
  .tab-icon.matview {
    color: #0ea5e9;
  }
  .tab-icon.function {
    color: #a855f7;
  }
  .tab-icon.sequence {
    color: #f59e0b;
  }
  .tab-icon.source {
    color: var(--text-muted);
  }
  .tab-icon.chat {
    color: var(--accent);
  }
  .tab-icon.notebook {
    color: var(--accent);
  }
  .tab-icon-text {
    font-size: 13px;
    line-height: 1;
    flex-shrink: 0;
  }
  .tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    user-select: none;
  }
  .tab-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    font-size: 12px;
    line-height: 1;
    color: var(--text-muted);
    border-radius: 3px;
    cursor: pointer;
    flex-shrink: 0;
    opacity: 0;
    user-select: none;
    transition:
      opacity var(--transition-fast),
      background var(--transition-fast),
      color var(--transition-fast);
  }
  .tab:hover .tab-close,
  .tab.active .tab-close {
    opacity: 1;
  }
  .tab-close:hover {
    color: var(--danger);
    background: var(--danger-bg);
  }

  .new-tab-group {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
    margin-left: 2px;
  }
  .new-tab-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    flex-shrink: 0;
    background: none;
    border: none;
    border-radius: var(--radius-md);
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
    transition: all var(--transition-fast);
  }
  .new-tab-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }

  .live-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    animation: live-pulse 1.2s infinite;
    flex-shrink: 0;
    margin: 0 2px;
  }
  @keyframes live-pulse {
    0%,
    100% {
      opacity: 1;
      transform: scale(1);
    }
    50% {
      opacity: 0.5;
      transform: scale(0.8);
    }
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .search-btn {
    display: flex;
    align-items: center;
    gap: 7px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    color: var(--text-secondary);
    padding: 6px 14px;
    border-radius: var(--radius-sm);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all var(--transition-normal);
    white-space: nowrap;
    box-shadow: var(--shadow-sm);
  }
  .search-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
    box-shadow:
      var(--shadow-md),
      0 0 0 2px color-mix(in srgb, var(--accent) 15%, transparent);
    transform: scale(1.03);
  }
  .search-btn svg {
    flex-shrink: 0;
    opacity: 0.6;
  }
  .search-btn:hover svg {
    opacity: 1;
    color: var(--accent);
  }
  .kbd {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-muted);
    background: var(--bg-subtle);
    padding: 2px 6px;
    border-radius: 4px;
    border: 1px solid var(--border);
    font-family: var(--font-mono);
    box-shadow: 0 1px 0 var(--border);
  }

  .icon-btn {
    background: none;
    border: 1px solid transparent;
    color: var(--text-secondary);
    width: 34px;
    height: 34px;
    border-radius: var(--radius-md);
    cursor: pointer;
    font-size: 15px;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all var(--transition-normal);
    flex-shrink: 0;
  }
  .icon-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
    transform: scale(1.1);
  }
  .icon-btn.ai-btn:hover {
    color: var(--accent);
    background: var(--accent-soft);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .icon-btn.logs-toggle.active {
    background: var(--accent-soft);
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .icon-btn.chat-toggle.active {
    background: var(--accent-soft);
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }

  .dirty-dot {
    width: 6px;
    height: 6px;
    margin-left: 4px;
    border-radius: 50%;
    background: var(--accent);
    flex-shrink: 0;
  }

  .indexing-indicator {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.25rem 0.6rem;
    border-radius: 999px;
    background: var(--accent-soft, rgba(127, 127, 255, 0.12));
    color: var(--text-secondary, inherit);
    font-size: 0.75rem;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .spinner {
    width: 0.75rem;
    height: 0.75rem;
    border-radius: 50%;
    border: 2px solid currentColor;
    border-top-color: transparent;
    animation: indexing-spin 0.8s linear infinite;
  }
  @keyframes indexing-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .indexing-bar {
    width: 4rem;
    height: 4px;
    border-radius: 2px;
    background: rgba(127, 127, 127, 0.25);
    overflow: hidden;
  }
  .indexing-bar span {
    display: block;
    height: 100%;
    background: currentColor;
    transition: width 0.2s ease;
  }
</style>
