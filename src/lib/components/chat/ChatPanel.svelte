<script lang="ts">
  import ChatMessage from './ChatMessage.svelte';
  import ChatInput from './ChatInput.svelte';
  import ChatLanding from './ChatLanding.svelte';
  import TypingIndicator from './TypingIndicator.svelte';
  import {
    chat,
    getConversationTitle,
    formatUsageLine,
  } from '../../stores/chat.svelte.ts';

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

  let msgsEl: HTMLDivElement;

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
    if (hasMessages && msgsEl && conv) {
      // Read content and segment state to establish reactive dependency —
      // without this, streaming text appends and thinking segments don't
      // trigger a scroll because $effect only watches `hasMessages`
      // (a boolean that never changes once true).
      const last = conv.messages[conv.messages.length - 1];
      if (last) {
        void last.content;
        void last.session?.segments.length;
        void last.session?.segments.at(-1)?.type;
      }
      requestAnimationFrame(() => {
        if (msgsEl) msgsEl.scrollTop = msgsEl.scrollHeight;
      });
    }
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
          title={getConversationTitle(c)}
        >
          <span class="conv-tab-label">{getConversationTitle(c)}</span>
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <span
            class="conv-tab-close"
            role="button"
            tabindex="-1"
            onclick={(e) => {
              e.stopPropagation();
              onCloseConv?.(c.id);
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
        onclick={onNewChat}
        title="New conversation"
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
      <div class="messages" bind:this={msgsEl}>
        {#each conv!.messages as m, i (m.id)}
          <ChatMessage
            message={m}
            {onRunDml}
            {onCancelDml}
            {onAllowPermission}
            {onRejectPermission}
            grouped={i > 0 && conv!.messages[i - 1].role === m.role}
            conversationId={conv!.id}
          />
        {/each}

        <TypingIndicator
          visible={chat.isStreaming && conv?.messages.at(-1)?.role === 'user'}
        />
      </div>

      <div class="input-area">
        <ChatInput {onSend} />
      </div>
    {:else}
      <ChatLanding
        {onSend}
        {connected}
        {database}
        {connectionName}
        {onOpenSettings}
      />
    {/if}
  </div>
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
    scroll-behavior: smooth;
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
</style>
