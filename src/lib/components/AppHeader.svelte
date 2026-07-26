<script lang="ts">
  import {
    chat,
    getConversationTitle,
    createNewTab,
    closeTab as closeChatTab,
    switchTab as switchChatTab,
  } from '../stores/chat.svelte.ts';

  let {
    config,
    connected,
    showAiSettings,
    showChatPanel,
    hasTabs,
    connectionId = '',
    leftWidth = 0,
    sidebarCollapsed = false,
    onToggleSidebar,
    onToggleTheme,
    onToggleAi,
    onToggleChat,
    onTogglePalette,
    onOpenChat,
    // unified tab bar props
    tabs = [],
    activeTabId = '',
    view = 'query',
    onSwitchTab,
    onCloseTab,
    onNewQuery,
    // batch close callbacks
    onCloseTabs,
  } = $props();

  let tabsEl: HTMLDivElement;
  let contextMenu = $state<{
    x: number;
    y: number;
    tabId: string;
    isChat: boolean;
  } | null>(null);

  const chatTabsVisible = $derived(connected && chat.conversations.length > 0);

  function tabIconSvg(tab: any): string {
    if (tab.kind === 'query') return 'query';
    if (tab.kind === 'table') return 'table';
    return 'source';
  }
  function tabLabel(tab: any) {
    if (tab.kind === 'query') return tab.name;
    if (tab.kind === 'source') return `${tab.schema}.${tab.name}`;
    return tab.name;
  }

  function handleSelectChatTab(id: string) {
    switchChatTab(id);
    onOpenChat?.();
    closeContextMenu();
  }

  function handleCloseChatTab(e: Event, id: string) {
    e.stopPropagation();
    closeChatTab(id);
    closeContextMenu();
  }

  function handleNewChatTab() {
    createNewTab(connectionId);
    onOpenChat?.();
  }

  function handleNewDbTab() {
    onNewQuery?.();
  }

  function isActive(tabId: string) {
    return activeTabId === tabId;
  }

  function isChatActive(convId: string) {
    return convId === chat.activeConversationId;
  }

  function closeContextMenu() {
    contextMenu = null;
  }

  function handleContextMenu(e: MouseEvent, tabId: string, isChat: boolean) {
    e.preventDefault();
    e.stopPropagation();
    contextMenu = { x: e.clientX, y: e.clientY, tabId, isChat };
  }

  // Close context menu on any click outside
  function handleGlobalClick() {
    closeContextMenu();
  }

  // ── Context menu actions ──────────────────────────────

  function closeTab(tabId: string, isChat: boolean) {
    if (isChat) {
      closeChatTab(tabId);
    } else {
      onCloseTab?.(tabId);
    }
    closeContextMenu();
  }

  function closeOtherTabs(tabId: string, isChat: boolean) {
    if (isChat) {
      for (const c of chat.conversations) {
        if (c.id !== tabId) closeChatTab(c.id);
      }
    } else {
      const toClose = tabs
        .filter((t: any) => t.id !== tabId)
        .map((t: any) => t.id);
      onCloseTabs?.(toClose);
    }
    closeContextMenu();
  }

  function closeTabsToRight(tabId: string, isChat: boolean) {
    if (isChat) {
      const ids = chat.conversations.map((c) => c.id);
      const idx = ids.indexOf(tabId);
      for (let i = idx + 1; i < ids.length; i++) {
        closeChatTab(ids[i]);
      }
    } else {
      const ids = tabs.map((t: any) => t.id);
      const idx = ids.indexOf(tabId);
      const toClose = ids.slice(idx + 1);
      onCloseTabs?.(toClose);
    }
    closeContextMenu();
  }

  function closeTabsToLeft(tabId: string, isChat: boolean) {
    if (isChat) {
      const ids = chat.conversations.map((c) => c.id);
      const idx = ids.indexOf(tabId);
      for (let i = idx - 1; i >= 0; i--) {
        closeChatTab(ids[i]);
      }
    } else {
      const ids = tabs.map((t: any) => t.id);
      const idx = ids.indexOf(tabId);
      const toClose = ids.slice(0, idx);
      onCloseTabs?.(toClose);
    }
    closeContextMenu();
  }

  function closeAllTabs() {
    for (const c of chat.conversations) {
      closeChatTab(c.id);
    }
    onCloseTabs?.(tabs.map((t: any) => t.id));
    closeContextMenu();
  }

  $effect(() => {
    if (tabsEl && (chat.activeConversationId || activeTabId)) {
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
        {/if}
        <span class="brand-spacer"></span>
        <button
          class="icon-btn sidebar-toggle"
          onclick={onToggleSidebar}
          title={sidebarCollapsed ? 'Show sidebar' : 'Hide sidebar'}
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
              oncontextmenu={(e) => handleContextMenu(e, tab.id, false)}
              title={tabLabel(tab)}
            >
              <span class="tab-icon {tabIconSvg(tab)}">
                {#if tab.kind === 'query'}
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
                    <polyline points="16 3 21 3 21 8" /><line
                      x1="4"
                      y1="20"
                      x2="21"
                      y2="3"
                    /><polyline points="21 16 21 21 16 21" /><line
                      x1="15"
                      y1="15"
                      x2="21"
                      y2="21"
                    /><line x1="4" y1="4" x2="9" y2="9" />
                  </svg>
                {:else if tab.kind === 'table'}
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
                {:else}
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
                    /><line x1="16" y1="17" x2="8" y2="17" />
                  </svg>
                {/if}
              </span>
              <span class="tab-label">{tabLabel(tab)}</span>
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
                tabindex="-1">×</span
              >
            </button>
          {/each}
        </div>

        <div class="new-tab-group">
          <button class="new-tab-btn" onclick={handleNewDbTab} title="New Query"
            >+</button
          >
        </div>
      {/if}
    </div>

    <div class="actions">
      <button class="icon-btn ai-btn" onclick={onToggleAi} title="AI Settings">
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

      {#if connected}
        <button class="search-btn" onclick={onTogglePalette}>
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

        <button class="icon-btn" onclick={onToggleTheme} title="Toggle theme">
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
    </div>
  </header>

  {#if contextMenu}
    {@const cm = contextMenu}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="context-menu"
      style="left:{cm.x}px; top:{cm.y}px"
      onclick={(e) => e.stopPropagation()}
    >
      <button class="menu-item" onclick={() => closeTab(cm.tabId, cm.isChat)}>
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
          <line x1="18" y1="6" x2="6" y2="18" /><line
            x1="6"
            y1="6"
            x2="18"
            y2="18"
          />
        </svg>
        Close Tab
      </button>
      <button
        class="menu-item"
        onclick={() => closeOtherTabs(cm.tabId, cm.isChat)}
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
          <rect x="3" y="3" width="18" height="18" rx="2" /><line
            x1="9"
            y1="3"
            x2="9"
            y2="21"
          /><line x1="15" y1="3" x2="15" y2="21" />
        </svg>
        Close Others
      </button>
      <button
        class="menu-item"
        onclick={() => closeTabsToRight(cm.tabId, cm.isChat)}
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
          <line x1="3" y1="12" x2="21" y2="12" /><polyline
            points="15 6 21 12 15 18"
          />
        </svg>
        Close to the Right
      </button>
      <button
        class="menu-item"
        onclick={() => closeTabsToLeft(cm.tabId, cm.isChat)}
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
          <line x1="3" y1="12" x2="21" y2="12" /><polyline
            points="9 6 3 12 9 18"
          />
        </svg>
        Close to the Left
      </button>
      <div class="menu-separator"></div>
      <button class="menu-item" onclick={() => closeAllTabs()}>
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
            d="M9 3v18"
          /><path d="M15 3v18" />
        </svg>
        Close All
      </button>
    </div>
  {/if}
</div>

<style>
  .app-header {
    position: relative;
    user-select: none;
  }
  header {
    height: 44px;
    padding: 0 12px 0 0;
    display: flex;
    align-items: center;
    gap: 12px;
    background: rgba(255, 255, 255, 0.72);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    border-bottom: 1px solid rgba(0, 0, 0, 0.08);
    user-select: none;
    flex-shrink: 0;
    z-index: 100;
    transition: background var(--transition-normal);
  }
  :global(.dark) header {
    background: rgba(15, 15, 23, 0.78);
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
    gap: 5px;
    height: 28px;
    padding: 0 8px;
    font-size: var(--text-xs);
    color: var(--text-secondary);
    background: transparent;
    border: none;
    border-radius: var(--radius-md);
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    min-width: 0;
    max-width: 180px;
    user-select: none;
    -webkit-user-select: none;
    transition:
      background var(--transition-fast),
      color var(--transition-fast);
  }
  .tab:hover {
    color: var(--text);
    background: var(--bg-hover);
  }
  .tab.active {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .tab-icon {
    flex-shrink: 0;
    font-size: var(--text-xs);
    line-height: 1;
  }
  .tab-icon.query {
    color: var(--accent);
  }
  .tab-icon.table {
    color: var(--success);
  }
  .tab-icon.source {
    color: var(--text-muted);
  }
  .tab-icon.chat {
    color: var(--accent);
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
    gap: 6px;
    background: var(--bg-hover);
    border: 1px solid var(--border);
    color: var(--text-secondary);
    padding: 5px 10px;
    border-radius: 20px;
    font-size: 12px;
    cursor: pointer;
    transition: all var(--transition-fast);
    white-space: nowrap;
  }
  .search-btn:hover {
    background: var(--bg-subtle);
    color: var(--text);
    border-color: var(--accent);
    transform: scale(1.02);
  }
  .search-btn svg {
    flex-shrink: 0;
    opacity: 0.7;
  }
  .kbd {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-muted);
    background: var(--bg);
    padding: 1px 5px;
    border-radius: 4px;
    border: 1px solid var(--border);
    font-family: var(--font-sans);
  }

  .icon-btn {
    background: none;
    border: 1px solid transparent;
    color: var(--text-secondary);
    width: 32px;
    height: 32px;
    border-radius: var(--radius-md);
    cursor: pointer;
    font-size: 15px;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all var(--transition-fast);
    flex-shrink: 0;
  }
  .icon-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
    transform: scale(1.05);
  }
  .icon-btn.ai-btn:hover {
    color: var(--accent);
    background: var(--accent-soft);
  }
  .icon-btn.chat-toggle.active {
    background: var(--accent-soft);
    color: var(--accent);
  }

  /* ── Context menu ───────────────────────────── */
  .context-menu {
    position: fixed;
    z-index: 9999;
    min-width: 180px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.15);
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  :global(.dark) .context-menu {
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  }
  .menu-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px;
    font-size: 13px;
    color: var(--text);
    background: transparent;
    border: none;
    border-radius: var(--radius-md);
    cursor: pointer;
    text-align: left;
    white-space: nowrap;
    user-select: none;
    transition: background var(--transition-fast);
  }
  .menu-item:hover {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .menu-item svg {
    flex-shrink: 0;
    color: var(--text-muted);
  }
  .menu-item:hover svg {
    color: var(--accent);
  }
  .menu-separator {
    height: 1px;
    background: var(--border);
    margin: 4px 8px;
  }
</style>
