<script lang="ts">
  import ChatMessage from './ChatMessage.svelte';
  import ChatInput from './ChatInput.svelte';
  import ChatLanding from './ChatLanding.svelte';
  import TypingIndicator from './TypingIndicator.svelte';
  import MemoryDrawer from './MemoryDrawer.svelte';
  import PreviousChatsDrawer from './PreviousChatsDrawer.svelte';
  import {
    chat,
    getConversationTitle,
    formatUsageLine,
    openConversation,
    closeOtherTabs,
    closeTabsToRight,
    closeTabsToLeft,
    closeAllTabs,
  } from '../../stores/chat.svelte.ts';
  import TabContextMenu from '../TabContextMenu.svelte';
  import type { TabMenuItem } from '../tab-menu.ts';
  import { computeTurnLayout } from './chat-scroll.ts';

  let showMemoryDrawer = $state(false);
  let memoryTriggerEl = $state<HTMLButtonElement | null>(null);
  let showPreviousChats = $state(false);
  let previousChatsTriggerEl = $state<HTMLButtonElement | null>(null);
  let tabMenu = $state<{ x: number; y: number; convId: string } | null>(null);

  async function handleSelectPreviousConv(convId: string) {
    showPreviousChats = false;
    await openConversation(convId);
    onSwitchConv?.(convId);
  }

  function handleTabContextMenu(e: MouseEvent, convId: string) {
    e.preventDefault();
    e.stopPropagation();
    tabMenu = { x: e.clientX, y: e.clientY, convId };
  }

  // Same items and icons as the DB tab strip in AppHeader (shared
  // TabContextMenu), so tab management feels identical everywhere.
  const tabMenuItems = $derived.by<TabMenuItem[]>(() => {
    const menu = tabMenu;
    if (!menu) return [];
    const id = menu.convId;
    return [
      {
        label: 'Close Tab',
        icon: 'close',
        action: () => onCloseConv?.(id),
      },
      {
        label: 'Close Others',
        icon: 'close-others',
        action: () => closeOtherTabs(id),
      },
      {
        label: 'Close to the Right',
        icon: 'close-right',
        action: () => closeTabsToRight(id),
      },
      {
        label: 'Close to the Left',
        icon: 'close-left',
        action: () => closeTabsToLeft(id),
      },
      { separator: true },
      {
        label: 'Close All',
        icon: 'close-all',
        action: () => closeAllTabs(),
      },
    ];
  });

  let {
    onSend,
    onRunDml,
    onCancelDml,
    onAllowPermission,
    onRejectPermission,
    onClose,
    onNewChat,
    onSwitchConv,
    onCloseConv,
    connected = false,
    database = null,
    connectionName = null,
    onOpenSettings,
  }: {
    onSend: (m: string) => void;
    onRunDml: () => void;
    onCancelDml: () => void;
    onAllowPermission?: () => void;
    onRejectPermission?: () => void;
    onClose?: () => void;
    onNewChat?: () => void;
    onSwitchConv?: (id: string) => void;
    onCloseConv?: (id: string) => void;
    /** Forwarded to ChatLanding for its empty state. */
    connected?: boolean;
    database?: string | null;
    connectionName?: string | null;
    onOpenSettings?: () => void;
  } = $props();

  let msgsEl = $state<HTMLDivElement>();
  let tailSpacerEl = $state<HTMLDivElement>();
  // Bound so a panel resize re-runs the turn layout (the effect reads it).
  let viewportHeight = $state(0);

  // ── Turn scroll choreography ─────────────────────────────────────────
  // Sending pins the newest user message to the top of the viewport and a
  // spacer absorbs the height the turn has not used yet, so the thinking and
  // response stream into empty space below instead of pushing the message
  // off-screen. Once the response outgrows the viewport the view follows the
  // tail — unless the reader scrolled away, in which case their position is
  // left alone. See `chat-scroll.ts` for the geometry.
  let anchorMessageId: string | null = null;
  let anchorPending = false;
  let followTail = true;
  let lastScrollTop = 0;
  // The bottom-most scroll position as of the last layout. Comparing against
  // this instead of a fresh `scrollHeight` keeps "reader returned to the
  // bottom" from being missed when text streams in between their scroll and
  // the scroll event that reports it.
  let lastMaxScrollTop = 0;

  /** "At the bottom" tolerance in px. */
  const BOTTOM_SLACK = 32;

  function handleSend(m: string) {
    anchorPending = true;
    followTail = true;
    onSend(m);
  }

  /** Offset of `el`'s top within the scrollable content of `container`. */
  function offsetWithin(el: HTMLElement, container: HTMLElement): number {
    return (
      el.getBoundingClientRect().top -
      container.getBoundingClientRect().top +
      container.scrollTop
    );
  }

  // Scroll events fire for programmatic scrolls too. Landing at the bottom
  // re-arms following; an upward move is the reader taking over, and
  // following stops until they return to the bottom.
  function handleScroll() {
    if (!msgsEl) return;
    const top = msgsEl.scrollTop;
    if (top >= lastMaxScrollTop - BOTTOM_SLACK) followTail = true;
    else if (top < lastScrollTop - 2) followTail = false;
    lastScrollTop = top;
  }

  function recordScrollPosition() {
    if (!msgsEl) return;
    lastScrollTop = msgsEl.scrollTop;
    lastMaxScrollTop = msgsEl.scrollHeight - msgsEl.clientHeight;
  }

  function layoutTurn() {
    if (!msgsEl || !tailSpacerEl) return;

    const anchorEl = anchorMessageId
      ? msgsEl.querySelector<HTMLElement>(
          `[data-message-id="${anchorMessageId}"]`,
        )
      : null;

    if (!anchorEl) {
      tailSpacerEl.style.height = '0px';
      if (followTail) msgsEl.scrollTop = msgsEl.scrollHeight;
      recordScrollPosition();
      return;
    }

    const layout = computeTurnLayout(
      {
        viewportHeight: msgsEl.clientHeight,
        scrollHeight: msgsEl.scrollHeight,
        spacerHeight: tailSpacerEl.offsetHeight,
        anchorTop: offsetWithin(anchorEl, msgsEl),
      },
      followTail,
    );

    tailSpacerEl.style.height = `${layout.spacerHeight}px`;
    if (layout.scrollTop !== null) msgsEl.scrollTop = layout.scrollTop;
    recordScrollPosition();
  }

  const conv = $derived(
    chat.conversations.find((c) => c.id === chat.activeConversationId),
  );
  const hasMessages = $derived(Boolean(conv && conv.messages.length > 0));

  const usageTitle = $derived(
    conv?.usage
      ? `${conv.usage.promptTokens} prompt tokens (${conv.usage.cachedPromptTokens} cached) · ${conv.usage.completionTokens} completion tokens`
      : '',
  );

  $effect(() => {
    if (!hasMessages || !msgsEl || !conv) return;
    // A send pins the newest user message; the rest of the turn keeps the
    // same anchor. Only a fresh send re-anchors.
    if (anchorPending) {
      const anchor = [...conv.messages]
        .reverse()
        .find((m) => m.role === 'user');
      if (anchor) {
        anchorMessageId = anchor.id;
        anchorPending = false;
      }
    }
    // Read content and segment state to establish reactive dependency —
    // without this, streaming text appends and thinking segments don't
    // trigger a scroll because $effect only watches `hasMessages`
    // (a boolean that never changes once true).
    const last = conv.messages[conv.messages.length - 1];
    if (last) {
      void last.content;
      void last.session?.segments.length;
      void last.session?.segments.at(-1)?.type;
      // Approval and permission cards appear mid-turn without new text;
      // they must still scroll into view.
      void last.dmlApproval;
      void last.permissionRequest;
    }
    // A resized panel changes how much space the turn has.
    void viewportHeight;
    requestAnimationFrame(layoutTurn);
  });

  // Switching conversations starts a fresh scroll state — no stale anchor or
  // spacer from the previous thread. A conversation created by a send is not
  // a switch: its anchor must survive (the id goes null -> new, and the send
  // that caused it sets `anchorPending`).
  let lastConversationId: string | null = null;
  $effect(() => {
    const id = chat.activeConversationId;
    if (lastConversationId !== null && id !== lastConversationId) {
      anchorMessageId = null;
      anchorPending = false;
      followTail = true;
      if (tailSpacerEl) tailSpacerEl.style.height = '0px';
    }
    lastConversationId = id;
  });
</script>

<aside class="panel">
  <!-- Panel header: conversation tabs + actions -->
  <div class="panel-header">
    <div class="conv-tabs">
      {#each chat.conversations as c (c.id)}
        <button
          class="conv-tab"
          class:active={c.id === chat.activeConversationId}
          onclick={() => onSwitchConv?.(c.id)}
          oncontextmenu={(e) => handleTabContextMenu(e, c.id)}
          title={getConversationTitle(c)}
        >
          <span class="conv-tab-label">{getConversationTitle(c)}</span>
          <span
            class="conv-tab-close"
            role="button"
            tabindex="0"
            aria-label={`Close ${getConversationTitle(c)}`}
            onclick={(e) => {
              e.stopPropagation();
              onCloseConv?.(c.id);
            }}
            onkeydown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                e.stopPropagation();
                onCloseConv?.(c.id);
              }
            }}>×</span
          >
        </button>
      {/each}
      {#if chat.isStreaming}
        <span class="live-dot"></span>
      {/if}
    </div>
    {#if conv && conv.usage && conv.usage.promptTokens + conv.usage.completionTokens > 0}
      <span class="usage-line" title={usageTitle}>
        {formatUsageLine(conv.usage)}
      </span>
    {/if}
    <div class="panel-actions">
      <button
        class="panel-icon-btn"
        class:active={showMemoryDrawer}
        bind:this={memoryTriggerEl}
        onclick={() => {
          showMemoryDrawer = !showMemoryDrawer;
          if (showMemoryDrawer) showPreviousChats = false;
        }}
        title="AI Memory & Rules (Cmd+Shift+M)"
        aria-label="AI Memory & Rules"
      >
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
          <path
            d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"
          />
          <path
            d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"
          />
          <path d="M12 5v13" />
          <path d="M15.5 13a3.5 3.5 0 0 0-3.5 3.5" />
          <path d="M8.5 13a3.5 3.5 0 0 1 3.5 3.5" />
        </svg>
      </button>
      <button
        class="panel-icon-btn"
        class:active={showPreviousChats}
        bind:this={previousChatsTriggerEl}
        onclick={() => {
          showPreviousChats = !showPreviousChats;
          if (showPreviousChats) showMemoryDrawer = false;
        }}
        title="Previous chats"
        aria-label="Previous chats"
      >
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
          <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
          <path d="M3 3v5h5" />
          <path d="M12 7v5l4 2" />
        </svg>
      </button>
      <button
        class="panel-icon-btn"
        onclick={onNewChat}
        title="New conversation"
        aria-label="New conversation"
      >
        <svg
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <line x1="12" y1="5" x2="12" y2="19" /><line
            x1="5"
            y1="12"
            x2="19"
            y2="12"
          />
        </svg>
      </button>
      <button
        class="panel-icon-btn close-btn"
        onclick={onClose}
        title="Close AI panel"
        aria-label="Close AI panel"
      >
        <svg
          width="13"
          height="13"
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
    </div>
  </div>
  <div class="body">
    {#if hasMessages}
      {#if conv?.error}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="conv-error"
          role="button"
          tabindex="0"
          onclick={() => (conv.error = null)}
        >
          <span class="conv-error-text selectable">{conv.error}</span>
          <span class="conv-error-dismiss">×</span>
        </div>
      {/if}
      <div
        class="messages"
        bind:this={msgsEl}
        bind:clientHeight={viewportHeight}
        onscroll={handleScroll}
      >
        {#each conv!.messages as m, i (m.id)}
          <ChatMessage
            message={m}
            {onRunDml}
            {onCancelDml}
            {onAllowPermission}
            {onRejectPermission}
            onOpenMemoryDrawer={() => (showMemoryDrawer = true)}
            grouped={i > 0 && conv!.messages[i - 1].role === m.role}
            conversationId={conv!.id}
          />
        {/each}

        <TypingIndicator
          visible={chat.isStreaming && conv?.messages.at(-1)?.role === 'user'}
        />

        <!-- Absorbs the viewport height a streaming turn has not used yet, so
             the sent message can sit at the top with the response growing
             into the space below it (see `chat-scroll.ts`). -->
        <div
          class="tail-spacer"
          bind:this={tailSpacerEl}
          aria-hidden="true"
        ></div>
      </div>

      <div class="input-area">
        <ChatInput onSend={handleSend} />
      </div>
    {:else}
      <ChatLanding
        onSend={handleSend}
        {connected}
        {database}
        {connectionName}
        {onOpenSettings}
      />
    {/if}
  </div>

  <MemoryDrawer
    bind:isOpen={showMemoryDrawer}
    connectionId={conv?.connectionId}
    triggerEl={memoryTriggerEl}
  />

  <PreviousChatsDrawer
    bind:isOpen={showPreviousChats}
    connectionId={conv?.connectionId}
    triggerEl={previousChatsTriggerEl}
    onSelectConv={handleSelectPreviousConv}
  />

  {#if tabMenu}
    <TabContextMenu
      x={tabMenu.x}
      y={tabMenu.y}
      items={tabMenuItems}
      onClose={() => (tabMenu = null)}
    />
  {/if}
</aside>

<style>
  .panel {
    width: 380px;
    min-width: 280px;
    max-width: 50vw;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--border);
    background: var(--bg-surface);
    height: 100%;
    position: relative;
  }

  /* ── Panel header ── */
  .panel-header {
    display: flex;
    align-items: center;
    height: 38px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    gap: 0;
    padding: 0 4px 0 0;
    background: var(--bg-elevated);
    overflow: hidden;
  }
  .conv-tabs {
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 0;
    height: 100%;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    gap: 2px;
    padding: 4px 4px;
  }
  .conv-tabs::-webkit-scrollbar {
    display: none;
  }
  .conv-tab {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding: 0 8px;
    font-size: 11px;
    color: var(--text-secondary);
    background: transparent;
    border: none;
    border-radius: var(--radius-md);
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    max-width: 140px;
    transition:
      background var(--transition-fast),
      color var(--transition-fast);
  }
  .conv-tab:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .conv-tab.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .conv-tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .conv-tab-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    font-size: 12px;
    line-height: 1;
    border-radius: 3px;
    color: var(--text-muted);
    opacity: 0;
    flex-shrink: 0;
    transition:
      opacity var(--transition-fast),
      background var(--transition-fast),
      color var(--transition-fast);
  }
  .conv-tab:hover .conv-tab-close,
  .conv-tab.active .conv-tab-close {
    opacity: 1;
  }
  .conv-tab-close:hover {
    color: var(--danger);
    background: var(--danger-bg);
  }

  .panel-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
    padding: 0 2px;
  }
  .panel-icon-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition: all var(--transition-fast);
    flex-shrink: 0;
  }
  .panel-icon-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .panel-icon-btn.close-btn:hover {
    background: var(--danger-bg);
    color: var(--danger);
  }

  .live-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    animation: live-pulse 1.2s infinite;
    flex-shrink: 0;
    margin: 0 4px;
  }
  .usage-line {
    font-size: 10px;
    color: var(--text-muted);
    white-space: nowrap;
    flex-shrink: 0;
    padding: 0 6px;
    font-variant-numeric: tabular-nums;
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

  .body {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    /* Deliberately no `scroll-behavior: smooth`: programmatic scrolling here
       tracks streaming text frame by frame, and a smooth animation never
       catches up — the newest thinking/response ended up below the fold. */
  }

  /* Sized in px by the turn layout; 0 when nothing is streaming or the
     response already fills the viewport. */
  .tail-spacer {
    flex: 0 0 auto;
    min-height: 0;
  }
  .messages::-webkit-scrollbar {
    width: 4px;
  }
  .messages::-webkit-scrollbar-thumb {
    background: transparent;
    border-radius: var(--radius-full);
  }
  .messages:hover::-webkit-scrollbar-thumb {
    background: var(--border);
  }

  .input-area {
    flex-shrink: 0;
  }

  .conv-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 8px 16px 0;
    padding: 8px 12px;
    font-size: var(--text-sm);
    color: var(--danger);
    background: var(--danger-bg);
    border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent);
    border-radius: var(--radius-md);
    cursor: pointer;
  }
  .conv-error-text {
    line-height: 1.35;
  }
  .conv-error-dismiss {
    font-size: 14px;
    flex-shrink: 0;
  }
</style>
