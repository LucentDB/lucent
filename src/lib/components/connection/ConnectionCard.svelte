<script lang="ts">
  import type { ConnectionProfile } from '../../stores/connections.svelte';
  import { connectionSubtitle } from '../../connection-format';

  let {
    profile,
    active = false,
    testing = false,
    viewMode = 'list',
    onSelect,
    onTest,
    onEdit,
    onDelete,
    onDuplicate,
  }: {
    profile: ConnectionProfile;
    active?: boolean;
    testing?: boolean;
    viewMode?: 'list' | 'grid';
    onSelect?: (id: string) => void;
    onTest?: (id: string) => void;
    onEdit?: (profile: ConnectionProfile) => void;
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

  const cardColor = $derived(profile.color ?? '#3b82f6');
</script>

<div
  class="connection-card"
  class:active
  class:testing
  class:grid-mode={viewMode === 'grid'}
  tabindex="0"
  role="button"
  style="--card-color: {cardColor}"
  onclick={() => onSelect?.(profile.id)}
  onkeydown={handleKeydown}
>
  {#if active}
    <span class="card-active-badge">
      <span class="active-dot"></span>
      Connected
    </span>
  {/if}

  <div class="card-content">
    <div class="card-main-row">
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
        <div class="card-details">
          <span class="card-host">
            {connectionSubtitle(profile)}
          </span>
          <span class="card-separator">·</span>
          <span class="card-time">{formatLastUsed(profile.lastUsed)}</span>
        </div>
      </div>

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
            width="15"
            height="15"
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
          title="Edit profile"
          onclick={(e) => {
            e.stopPropagation();
            onEdit?.(profile);
          }}
        >
          <svg
            width="15"
            height="15"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <path
              d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"
            />
            <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
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
            width="15"
            height="15"
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
            width="15"
            height="15"
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
  </div>

  {#if testing}
    <div class="card-testing">
      <span class="spinner"></span>
    </div>
  {/if}
</div>

<style>
  .connection-card {
    display: flex;
    padding: 16px 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
    cursor: pointer;
    transition:
      transform 0.4s cubic-bezier(0.175, 0.885, 0.32, 1.275),
      border-color var(--transition-normal),
      box-shadow var(--transition-normal),
      background var(--transition-normal);
    position: relative;
    outline: none;
    box-sizing: border-box;
    box-shadow: var(--shadow-sm);
    overflow: hidden;
  }

  .connection-card:hover,
  .connection-card:focus-visible {
    box-shadow: 0 4px 12px -4px rgba(0, 0, 0, 0.1);
    transform: translateY(-2px);
  }

  .connection-card.active {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
    background: color-mix(in srgb, var(--accent) 3%, var(--bg-surface));
    box-shadow:
      0 4px 12px -2px color-mix(in srgb, var(--accent) 12%, transparent),
      0 0 0 1px color-mix(in srgb, var(--accent) 30%, transparent);
  }

  .connection-card.testing {
    opacity: 0.65;
    pointer-events: none;
  }

  .card-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    z-index: 2;
  }

  .card-main-row {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
  }

  .card-icon {
    width: 40px;
    height: 40px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--card-color, var(--accent));
    flex-shrink: 0;
    background: var(--bg-subtle);
    border-radius: var(--radius-md);
  }

  .card-icon .icon {
    font-size: 20px;
  }

  .card-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .card-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    letter-spacing: -0.015em;
  }

  .card-active-badge {
    position: absolute;
    top: 12px;
    right: 12px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 700;
    color: var(--success, #10b981);
    background: color-mix(in srgb, var(--success, #10b981) 10%, transparent);
    padding: 4px 10px 4px 8px;
    border-radius: 99px;
    line-height: 1.2;
    border: 1px solid
      color-mix(in srgb, var(--success, #10b981) 30%, transparent);
    z-index: 2;
  }

  .active-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--success, #10b981);
    box-shadow: 0 0 8px 1px var(--success, #10b981);
    animation: pulse-connected 1.5s ease-in-out infinite;
  }

  @keyframes pulse-connected {
    0%,
    100% {
      opacity: 1;
      transform: scale(1);
      box-shadow: 0 0 8px 1px var(--success, #10b981);
    }
    50% {
      opacity: 0.5;
      transform: scale(0.85);
      box-shadow: 0 0 3px 0 var(--success, #10b981);
    }
  }

  .card-details {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
  }

  .card-host {
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 11px;
  }

  .card-separator {
    color: var(--border);
    flex-shrink: 0;
    opacity: 0.6;
  }

  .card-time {
    flex-shrink: 0;
  }

  .card-testing {
    position: absolute;
    top: 13px;
    right: 13px;
    display: flex;
    align-items: center;
    z-index: 2;
  }

  .spinner {
    width: 15px;
    height: 15px;
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
    align-items: center;
    gap: 2px;
    opacity: 0;
    transform: translateX(10px);
    transition: all 0.3s cubic-bezier(0.175, 0.885, 0.32, 1.275);
    flex-shrink: 0;
    z-index: 2;
  }

  .connection-card:hover .card-actions,
  .connection-card:focus-within .card-actions {
    opacity: 1;
    transform: translateX(0);
  }

  .action-btn {
    width: 30px;
    height: 30px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition:
      background var(--transition-fast),
      color var(--transition-fast),
      transform var(--transition-fast);
  }

  .action-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
    transform: scale(1.1);
  }

  .action-btn.danger:hover {
    background: color-mix(in srgb, var(--error) 12%, transparent);
    color: var(--error);
  }

  /* Grid mode adjustments */
  .connection-card.grid-mode {
    flex-direction: column;
    padding: 20px;
  }

  .connection-card.grid-mode .card-active-badge {
    top: 16px;
    right: 16px;
  }

  .connection-card.grid-mode .card-main-row {
    flex-direction: column;
    align-items: flex-start;
    gap: 12px;
  }

  .connection-card.grid-mode .card-icon {
    width: 44px;
    height: 44px;
  }

  .connection-card.grid-mode .card-icon svg {
    width: 24px;
    height: 24px;
  }

  .connection-card.grid-mode .card-info {
    width: 100%;
  }

  .connection-card.grid-mode .card-details {
    flex-wrap: wrap;
    white-space: normal;
  }

  .connection-card.grid-mode .card-actions {
    width: 100%;
    justify-content: flex-end;
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
    opacity: 1;
    transform: none;
  }
</style>
