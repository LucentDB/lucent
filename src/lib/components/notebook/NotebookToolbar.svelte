<script lang="ts">
  let {
    onRunAll,
    onClearOutputs,
    onRestartSession,
    mode = 'command',
    isRunning = false,
    runAllProgress = null as { current: number; total: number } | null,
    connectionName = '',
    databaseName = '',
  }: {
    onRunAll?: () => void;
    onClearOutputs?: () => void | Promise<void>;
    onRestartSession?: () => void;
    mode?: 'command' | 'edit';
    isRunning?: boolean;
    runAllProgress?: { current: number; total: number } | null;
    connectionName?: string;
    databaseName?: string;
  } = $props();
</script>

<div class="notebook-toolbar" role="toolbar" aria-label="Notebook actions">
  <div class="toolbar-left">
    <button
      class="toolbar-btn primary"
      onclick={onRunAll}
      disabled={isRunning}
      type="button"
    >
      {#if isRunning && runAllProgress}
        <span class="btn-spinner" aria-hidden="true"></span>
        <span class="btn-label">
          {runAllProgress.current}/{runAllProgress.total}
        </span>
      {:else}
        <svg
          class="btn-icon"
          width="11"
          height="11"
          viewBox="0 0 24 24"
          fill="currentColor"
          aria-hidden="true"
        >
          <polygon points="5 3 19 12 5 21" />
        </svg>
        <span class="btn-label">Run All</span>
      {/if}
    </button>
    <div class="divider"></div>
    <button
      class="toolbar-btn ghost"
      onclick={onClearOutputs}
      disabled={isRunning}
      type="button"
      title="Clear all outputs"
    >
      <svg
        class="btn-icon"
        width="11"
        height="11"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M3 6h18M8 6V4h8v2M6 6l1 14h10l1-14" />
      </svg>
      <span class="btn-label">Clear</span>
    </button>
    <button
      class="toolbar-btn ghost"
      onclick={onRestartSession}
      disabled={isRunning}
      type="button"
      title="Restart session"
    >
      <svg
        class="btn-icon"
        width="12"
        height="12"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
        <path d="M21 3v5h-5" />
        <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16" />
        <path d="M8 16H3v5" />
      </svg>
      <span class="btn-label">Restart</span>
    </button>
  </div>
  <div class="toolbar-right">
    <div
      class="mode-indicator"
      class:edit-mode={mode === 'edit'}
      role="status"
      aria-live="polite"
      title={mode === 'command'
        ? 'Command mode: Arrow keys navigate · Enter edits · Y SQL · I AI · M Markdown'
        : 'Edit mode: Esc returns to command mode · Shift+Enter runs and advances'}
    >
      <kbd>{mode === 'command' ? 'Esc' : 'Enter'}</kbd>
      <span>{mode === 'command' ? 'Command' : 'Edit'}</span>
    </div>
    {#if connectionName || databaseName}
      <div class="connection-badge">
        <span class="badge-dot" class:running={isRunning}></span>
        <span class="badge-text">
          {#if connectionName}<span class="conn-name">{connectionName}</span
            >{/if}
          {#if databaseName}<span class="db-separator">/</span><span
              class="db-name">{databaseName}</span
            >{/if}
        </span>
      </div>
    {/if}
  </div>
</div>

<style>
  .notebook-toolbar {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-surface);
    gap: 12px;
    flex-shrink: 0;
    backdrop-filter: blur(8px);
  }

  .toolbar-left {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .toolbar-right {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .mode-indicator {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-subtle);
    color: var(--text-secondary);
    font-size: var(--text-xs);
    white-space: nowrap;
  }
  .mode-indicator.edit-mode {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    background: var(--accent-soft);
    color: var(--accent);
  }
  .mode-indicator kbd {
    padding: 1px 4px;
    border: 1px solid currentColor;
    border-radius: 3px;
    font-family: var(--font-mono);
    font-size: 10px;
    line-height: 1.2;
  }

  .divider {
    width: 1px;
    height: 16px;
    background: var(--border);
    margin: 0 4px;
    border-radius: 1px;
  }

  .toolbar-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 5px 11px;
    border-radius: var(--radius-md);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    cursor: pointer;
    transition:
      background 0.12s ease,
      color 0.12s ease,
      border-color 0.12s ease,
      box-shadow 0.12s ease,
      transform 0.1s ease;
  }

  .toolbar-btn.primary {
    border: none;
    background: var(--accent);
    color: #fff;
    box-shadow: 0 1px 3px color-mix(in srgb, var(--accent) 35%, transparent);
    min-width: 80px;
    justify-content: center;
  }
  .toolbar-btn.primary:hover:not(:disabled) {
    background: var(--accent-hover);
    box-shadow: 0 2px 8px color-mix(in srgb, var(--accent) 45%, transparent);
    transform: translateY(-1px);
  }
  .toolbar-btn.primary:active:not(:disabled) {
    transform: translateY(0);
  }

  .toolbar-btn.ghost {
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-secondary);
  }
  .toolbar-btn.ghost:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text);
    border-color: var(--border);
  }

  .toolbar-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .toolbar-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .btn-label {
    flex-shrink: 0;
    white-space: nowrap;
    line-height: 1;
  }

  .btn-icon {
    flex-shrink: 0;
  }

  .btn-spinner {
    width: 10px;
    height: 10px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    flex-shrink: 0;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .connection-badge {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 4px 10px;
    border-radius: var(--radius-full);
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    font-size: var(--text-xs);
    color: var(--text-secondary);
    max-width: 260px;
    overflow: hidden;
  }

  .badge-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--success);
    flex-shrink: 0;
    transition: background 0.2s;
  }
  .badge-dot.running {
    background: var(--accent);
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }

  .badge-text {
    display: flex;
    align-items: center;
    gap: 5px;
    overflow: hidden;
    white-space: nowrap;
  }
  .conn-name {
    font-weight: var(--weight-medium);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .db-separator {
    color: var(--text-muted);
    font-weight: 300;
  }
  .db-name {
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-secondary);
  }
</style>
