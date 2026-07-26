<script lang="ts">
  import ChatMessage from './ChatMessage.svelte';
  import ChatInput from './ChatInput.svelte';
  import TypingIndicator from './TypingIndicator.svelte';
  import Icon from '../icons/Icon.svelte';
  import { chat, getConversationTitle } from '../../stores/chat.svelte.ts';

  let {
    onSend,
    onRunDml,
    onCancelDml,
    onClose,
    onNewChat,
    onSwitchConv,
    onCloseConv,
  }: {
    onSend: (m: string) => void;
    onRunDml: () => void;
    onCancelDml: () => void;
    onClose?: () => void;
    onNewChat?: () => void;
    onSwitchConv?: (id: string) => void;
    onCloseConv?: (id: string) => void;
  } = $props();

  let msgsEl: HTMLDivElement;

  const conv = $derived(
    chat.conversations.find((c) => c.id === chat.activeConversationId),
  );
  const hasMessages = $derived(Boolean(conv && conv.messages.length > 0));

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
      <div class="landing">
        <div class="hero">
          <div class="hero-icon"><Icon name="star" size={20} /></div>
          <h1>AI Copilot</h1>
          <p>
            Ask questions, run queries, and manage your database with natural
            language.
          </p>
        </div>

        <div class="input-wrap">
          <ChatInput {onSend} />
        </div>

        <div class="suggestions">
          <span class="suggest-label">Try asking:</span>
          <div class="chips">
            <button
              class="chip"
              onclick={() => onSend('Show recent orders with user details')}
            >
              <span class="chip-icon"><Icon name="chart" size={14} /></span>
              <span>Show recent orders with user details</span>
            </button>
            <button
              class="chip"
              onclick={() => onSend('What tables track user activity?')}
            >
              <span class="chip-icon"><Icon name="search" size={14} /></span>
              <span>What tables track user activity?</span>
            </button>
            <button
              class="chip"
              onclick={() => onSend('Delete old cancelled orders')}
            >
              <span class="chip-icon"><Icon name="clean" size={14} /></span>
              <span>Delete old cancelled orders</span>
            </button>
            <button
              class="chip"
              onclick={() => onSend('Which tables have the most rows?')}
            >
              <span class="chip-icon"><Icon name="trending" size={14} /></span>
              <span>Which tables have the most rows?</span>
            </button>
          </div>
        </div>
      </div>
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

  .landing {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    padding: 32px 28px;
    gap: 20px;
    overflow-y: auto;
  }
  .hero {
    text-align: center;
    max-width: 480px;
  }
  .hero-icon {
    width: 40px;
    height: 40px;
    border-radius: var(--radius-lg);
    background: var(--accent-soft);
    color: var(--accent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 20px;
    margin: 0 auto 12px;
  }
  .hero h1 {
    font-size: var(--text-xl);
    font-weight: var(--weight-bold);
    margin: 0 0 6px;
    color: var(--text);
  }
  .hero p {
    color: var(--text-secondary);
    font-size: var(--text-sm);
    margin: 0;
    line-height: 1.5;
  }
  .input-wrap {
    width: 100%;
    max-width: 600px;
  }

  .suggestions {
    width: 100%;
    max-width: 600px;
  }
  .suggest-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: var(--weight-semibold);
    display: block;
    margin-bottom: 8px;
  }
  .chips {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .chip {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 8px 12px;
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--bg-surface);
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-secondary);
    text-align: left;
    transition: all var(--transition-fast);
  }
  .chip:hover {
    border-color: var(--accent);
    color: var(--text);
    background: var(--accent-soft);
  }
  .chip-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    flex-shrink: 0;
    color: var(--accent);
    opacity: 0.7;
  }

  .input-area {
    flex-shrink: 0;
  }
</style>
