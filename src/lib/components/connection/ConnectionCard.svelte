<script lang="ts">
  import type { ConnectionProfile } from '../../stores/connections.svelte';

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

  const puser = $derived(profile.params['user'] ?? 'postgres');
  const phost = $derived(profile.params['host'] ?? '');
  const pport = $derived(profile.params['port'] ?? '5432');
  const pdb = $derived(profile.params['database'] ?? '');

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
  <div class="card-indicator" style="background: {cardColor}"></div>

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
            width="18"
            height="18"
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
            {puser}@{phost}:{pport}/{pdb}
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
          title="Edit profile"
          onclick={(e) => {
            e.stopPropagation();
            onEdit?.(profile);
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
    gap: 12px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
    cursor: pointer;
    transition:
      border-color 0.15s ease,
      box-shadow 0.15s ease;
    position: relative;
    outline: none;
    box-sizing: border-box;
  }
  .connection-card:hover,
  .connection-card:focus-visible {
    border-color: var(--card-color, var(--accent));
    box-shadow: 0 0 0 2px
      color-mix(in srgb, var(--card-color, var(--accent)) 20%, transparent);
  }
  .connection-card.active {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 6%, var(--bg-surface));
  }
  .connection-card.testing {
    opacity: 0.7;
    pointer-events: none;
  }
  .card-indicator {
    width: 4px;
    border-radius: 4px;
    flex-shrink: 0;
    align-self: stretch;
  }
  .card-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .card-main-row {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
  }
  .card-icon {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--card-color, var(--accent));
    flex-shrink: 0;
    background: color-mix(
      in srgb,
      var(--card-color, var(--accent)) 10%,
      transparent
    );
    border-radius: var(--radius-md);
  }
  .card-icon .icon {
    font-size: 18px;
  }
  .card-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .card-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .card-active-badge {
    position: absolute;
    top: 10px;
    right: 12px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    font-weight: 600;
    color: var(--success, #22c55e);
    background: color-mix(in srgb, var(--success, #22c55e) 10%, transparent);
    padding: 3px 8px;
    border-radius: 99px;
    line-height: 1.2;
  }
  .active-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--success, #22c55e);
    box-shadow: 0 0 6px var(--success, #22c55e);
  }
  .card-details {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
  }
  .card-host {
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .card-separator {
    color: var(--border);
    flex-shrink: 0;
  }
  .card-time {
    flex-shrink: 0;
  }
  .card-testing {
    position: absolute;
    top: 12px;
    right: 12px;
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
    align-items: center;
    gap: 2px;
    opacity: 0.65;
    transition: opacity 0.12s;
    flex-shrink: 0;
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

  /* Grid mode adjustments */
  .connection-card.grid-mode {
    flex-direction: column;
    padding: 16px;
  }
  .connection-card.grid-mode .card-indicator {
    width: 100%;
    height: 4px;
    align-self: auto;
    border-radius: 4px 4px 0 0;
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
  }
  .connection-card.grid-mode .card-active-badge {
    top: 14px;
    right: 14px;
  }
  .connection-card.grid-mode .card-main-row {
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
    margin-top: 8px;
  }
  .connection-card.grid-mode .card-icon {
    width: 40px;
    height: 40px;
  }
  .connection-card.grid-mode .card-icon svg {
    width: 22px;
    height: 22px;
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
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
  }
</style>
