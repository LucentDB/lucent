<script lang="ts">
  import type { ConnectionProfile } from '../../stores/connections.svelte';

  let {
    profile,
    active = false,
    testing = false,
    onSelect,
    onTest,
    onDelete,
    onDuplicate,
  }: {
    profile: ConnectionProfile;
    active?: boolean;
    testing?: boolean;
    onSelect?: (id: string) => void;
    onTest?: (id: string) => void;
    onDelete?: (id: string) => void;
    onDuplicate?: (id: string) => void;
  } = $props();

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      onSelect?.(profile.id);
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      onDelete?.(profile.id);
    }
  }

  function formatLastUsed(iso: string | null): string {
    if (!iso) return 'Never used';
    const d = new Date(iso);
    const now = new Date();
    const diffMs = now.getTime() - d.getTime();
    const mins = Math.floor(diffMs / 60000);
    if (mins < 1) return 'Just now';
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    if (days < 7) return `${days}d ago`;
    return d.toLocaleDateString();
  }

  // Pick a display color
  const cardColor = $derived(profile.color ?? '#3b82f6');
</script>

<div
  class="connection-card"
  class:active
  class:testing
  tabindex="0"
  role="button"
  style="--card-color: {cardColor}"
  onclick={() => onSelect?.(profile.id)}
  onkeydown={handleKeydown}
>
  <div class="card-indicator" style="background: {cardColor}"></div>
  <div class="card-icon">
    {#if profile.icon}
      <span class="icon">{profile.icon}</span>
    {:else}
      <svg
        width="20"
        height="20"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
      >
        <ellipse cx="12" cy="5" rx="9" ry="3" />
        <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" />
        <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
      </svg>
    {/if}
  </div>
  <div class="card-info">
    <span class="card-name">{profile.name}</span>
    <span class="card-host"
      >{profile.user}@{profile.host}:{profile.port}/{profile.database}</span
    >
    <span class="card-time">{formatLastUsed(profile.lastUsed)}</span>
  </div>
  {#if testing}
    <div class="card-testing">
      <span class="spinner"></span>
    </div>
  {/if}
  {#if active}
    <div class="card-active-badge">Connected</div>
  {/if}
  <div class="card-actions">
    <button
      class="action-btn"
      title="Test connection"
      onclick={(e) => {
        e.stopPropagation();
        onTest?.(profile.id);
      }}
    >
      <svg
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
      >
        <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
        <polyline points="22 4 12 14.01 9 11.01" />
      </svg>
    </button>
    <button
      class="action-btn"
      title="Duplicate"
      onclick={(e) => {
        e.stopPropagation();
        onDuplicate?.(profile.id);
      }}
    >
      <svg
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
      >
        <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
        <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
      </svg>
    </button>
    <button
      class="action-btn danger"
      title="Delete"
      onclick={(e) => {
        e.stopPropagation();
        onDelete?.(profile.id);
      }}
    >
      <svg
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
      >
        <polyline points="3 6 5 6 21 6" />
        <path
          d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"
        />
      </svg>
    </button>
  </div>
</div>

<style>
  .connection-card {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    cursor: pointer;
    transition:
      border-color 0.12s,
      box-shadow 0.12s;
    position: relative;
    outline: none;
  }
  .connection-card:hover,
  .connection-card:focus-visible {
    border-color: var(--card-color, var(--accent));
    box-shadow: 0 0 0 2px
      color-mix(in srgb, var(--card-color, var(--accent)) 20%, transparent);
  }
  .connection-card.active {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, var(--bg-surface));
  }
  .connection-card.testing {
    opacity: 0.7;
    pointer-events: none;
  }
  .card-indicator {
    width: 4px;
    height: 32px;
    border-radius: 4px;
    flex-shrink: 0;
  }
  .card-icon {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--card-color, var(--text-secondary));
    flex-shrink: 0;
  }
  .card-icon .icon {
    font-size: 20px;
  }
  .card-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .card-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .card-host {
    font-size: 12px;
    color: var(--text-muted);
    font-family: var(--font-mono);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .card-time {
    font-size: 11px;
    color: var(--text-muted);
  }
  .card-active-badge {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 99px;
    background: var(--accent);
    color: #fff;
    font-weight: 500;
  }
  .card-testing {
    display: flex;
    align-items: center;
  }
  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .card-actions {
    display: flex;
    gap: 4px;
    opacity: 0;
    transition: opacity 0.12s;
  }
  .connection-card:hover .card-actions,
  .connection-card:focus-within .card-actions {
    opacity: 1;
  }
  .action-btn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    transition:
      background 0.1s,
      color 0.1s;
  }
  .action-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .action-btn.danger:hover {
    background: color-mix(in srgb, var(--error) 15%, transparent);
    color: var(--error);
  }
</style>
