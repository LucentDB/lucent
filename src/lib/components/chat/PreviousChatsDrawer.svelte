<script lang="ts">
  import { onMount, tick } from 'svelte';
  import {
    listChatConversations,
    type ChatConversation,
  } from '../../ipc/ai.ts';
  import {
    chat,
    switchTab,
    openConversation,
    deleteConversationHistory,
  } from '../../stores/chat.svelte.ts';

  let {
    isOpen = $bindable(false),
    connectionId = '',
    onClose,
    triggerEl = null,
    onSelectConv,
  }: {
    isOpen?: boolean;
    connectionId?: string;
    onClose?: () => void;
    triggerEl?: HTMLElement | null;
    onSelectConv?: (convId: string) => void;
  } = $props();

  let conversations = $state<ChatConversation[]>([]);
  let searchQuery = $state('');
  let loading = $state(false);
  let statusMessage = $state<string | null>(null);
  let confirmDeleteId = $state<string | null>(null);
  let drawerEl = $state<HTMLDivElement>();
  let searchInputEl = $state<HTMLInputElement>();

  let portalHost = $state<HTMLElement | null>(null);

  function portal(node: HTMLElement) {
    portalHost = node;
    document.body.appendChild(node);
    return {
      destroy() {
        node.remove();
        portalHost = null;
      },
    };
  }

  export async function refresh() {
    loading = true;
    try {
      const all = await listChatConversations();
      conversations = all ?? [];
    } catch (e) {
      statusMessage = `Failed to load chats: ${String(e)}`;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (isOpen) {
      void refresh();
    } else {
      confirmDeleteId = null;
      searchQuery = '';
    }
  });

  // Inert background while the drawer is open and manage focus.
  $effect(() => {
    const host = portalHost;
    if (!isOpen || !host) return;

    const background = Array.from(document.body.children).filter(
      (el) => el !== host,
    );
    for (const el of background) el.setAttribute('inert', '');

    void tick().then(() => searchInputEl?.focus());

    return () => {
      for (const el of background) el.removeAttribute('inert');
      const target = triggerEl;
      if (target?.isConnected) {
        void tick().then(() => {
          if (target.isConnected) target.focus();
        });
      }
    };
  });

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      close();
    }
  }

  function close() {
    isOpen = false;
    onClose?.();
  }

  async function handleSelect(convId: string) {
    if (onSelectConv) {
      onSelectConv(convId);
    } else {
      await openConversation(convId);
    }
    close();
  }

  async function handleDelete(convId: string, e: MouseEvent) {
    e.stopPropagation();
    if (confirmDeleteId !== convId) {
      confirmDeleteId = convId;
      return;
    }
    confirmDeleteId = null;
    try {
      await deleteConversationHistory(convId);
      conversations = conversations.filter((c) => c.id !== convId);
    } catch (e) {
      statusMessage = `Delete failed: ${String(e)}`;
    }
  }

  function formatDate(timestampSeconds: number): string {
    const d = new Date(timestampSeconds * 1000);
    const now = new Date();
    const isToday =
      d.getDate() === now.getDate() &&
      d.getMonth() === now.getMonth() &&
      d.getFullYear() === now.getFullYear();

    if (isToday) {
      return d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    }
    return d.toLocaleDateString([], {
      month: 'short',
      day: 'numeric',
      hour: 'numeric',
      minute: '2-digit',
    });
  }

  const filteredConversations = $derived.by(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return conversations;
    return conversations.filter((c) => (c.title || '').toLowerCase().includes(q));
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="previous-chats-portal" data-previous-chats-portal use:portal>
  {#if isOpen}
    <div
      class="drawer-overlay"
      onclick={close}
      role="presentation"
    >
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="drawer"
        bind:this={drawerEl}
        onclick={(e) => e.stopPropagation()}
        onkeydown={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="Previous Chats"
        tabindex="-1"
      >
        <header class="drawer-header">
          <div class="drawer-title-row">
            <div class="drawer-title">
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
                <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
                <path d="M3 3v5h5" />
                <path d="M12 7v5l4 2" />
              </svg>
              <h2>Previous Chats</h2>
            </div>
            <button
              class="icon-btn"
              onclick={close}
              title="Close drawer (Esc)"
              aria-label="Close drawer"
            >
              ✕
            </button>
          </div>

          <div class="action-bar">
            <input
              type="text"
              class="search-input"
              placeholder="Search previous chats..."
              bind:value={searchQuery}
              bind:this={searchInputEl}
            />
          </div>
        </header>

        {#if statusMessage}
          <div class="status-banner selectable" role="status">
            {statusMessage}
          </div>
        {/if}

        <div class="drawer-body">
          {#if loading}
            <div class="loading-state">Loading previous chats...</div>
          {:else if filteredConversations.length === 0}
            <div class="empty-state">
              {#if searchQuery.trim()}
                No chats match "{searchQuery}".
              {:else}
                No previous chats found.
              {/if}
            </div>
          {:else}
            <div class="conv-list">
              {#each filteredConversations as conv (conv.id)}
                {@const isOpenTab = chat.conversations.some((c) => c.id === conv.id)}
                {@const isActive = chat.activeConversationId === conv.id}
                <div
                  class="conv-card"
                  class:active={isActive}
                  class:open-tab={isOpenTab}
                  role="button"
                  tabindex="0"
                  onclick={() => handleSelect(conv.id)}
                  onkeydown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      handleSelect(conv.id);
                    }
                  }}
                >
                  <div class="conv-card-top">
                    <span class="conv-title">{conv.title || 'Conversation'}</span>
                    <div class="conv-badges">
                      {#if isOpenTab}
                        <span class="badge open">Open</span>
                      {/if}
                      <button
                        class="delete-btn"
                        class:armed={confirmDeleteId === conv.id}
                        title={confirmDeleteId === conv.id ? 'Click again to confirm delete' : 'Delete chat'}
                        onclick={(e) => handleDelete(conv.id, e)}
                        onkeydown={(e) => {
                          e.stopPropagation();
                          if (e.key === 'Enter' || e.key === ' ') {
                            e.preventDefault();
                            handleDelete(conv.id, e as unknown as MouseEvent);
                          }
                        }}
                        aria-label={confirmDeleteId === conv.id ? 'Confirm delete chat' : 'Delete chat'}
                      >
                        {#if confirmDeleteId === conv.id}
                          Confirm?
                        {:else}
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
                            <polyline points="3 6 5 6 21 6" />
                            <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                          </svg>
                        {/if}
                      </button>
                    </div>
                  </div>
                  <div class="conv-card-bottom">
                    <span class="conv-date">{formatDate(conv.updated_at || conv.created_at)}</span>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
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
    width: 400px;
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
  .action-bar {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .search-input {
    flex: 1;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm, 4px);
    color: var(--text);
    padding: 6px 10px;
    font-size: var(--text-xs, 12px);
  }
  .search-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .status-banner {
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border);
    color: var(--accent);
    padding: 6px 16px;
    font-size: var(--text-xs, 12px);
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
  .conv-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .conv-card {
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-md, 6px);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .conv-card:hover {
    border-color: var(--accent);
    background: color-mix(in oklch, var(--accent) 5%, var(--bg-subtle));
  }
  .conv-card.active {
    border-color: var(--accent);
    background: color-mix(in oklch, var(--accent) 10%, var(--bg-subtle));
  }
  .conv-card-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
  }
  .conv-title {
    font-size: var(--text-sm, 13px);
    font-weight: 500;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .conv-badges {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .badge {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    padding: 1px 5px;
    border-radius: 3px;
  }
  .badge.open {
    background: rgba(16, 185, 129, 0.15);
    color: #10b981;
  }
  .delete-btn {
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 2px 4px;
    border-radius: var(--radius-sm, 3px);
    display: flex;
    align-items: center;
    font-size: 11px;
    transition: all 0.15s ease;
  }
  .delete-btn:hover {
    color: var(--danger, #ef4444);
    background: rgba(239, 68, 68, 0.1);
  }
  .delete-btn.armed {
    color: var(--danger, #ef4444);
    background: rgba(239, 68, 68, 0.15);
    font-weight: 600;
  }
  .conv-card-bottom {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .conv-date {
    font-size: 11px;
    color: var(--text-muted);
  }
</style>
