<script>
  import { untrack } from 'svelte';
  import { getLogs } from '../ipc/client.js';

  let { onClose } = $props();

  // Mirrors supervisor::LOG_BUFFER_CAP — the backend drops the oldest lines
  // past this, so the frontend stops growing its own list at the same bound.
  const CACHE_MAX = 1000;
  const POLL_MS = 2000;

  let lines = $state([]);
  let error = $state(null);
  let listEl = $state(null);
  let followTail = $state(true);
  let polling = $state(false);

  function formatError(e) {
    return typeof e === 'object' && e !== null && 'message' in e
      ? e.message
      : String(e);
  }

  function nearBottom() {
    if (!listEl) return true;
    return listEl.scrollTop + listEl.clientHeight >= listEl.scrollHeight - 24;
  }

  async function poll() {
    if (polling) return;
    polling = true;
    try {
      const after = lines.length;
      let fresh = await getLogs(after);
      if (fresh.length === 0 && after >= CACHE_MAX) {
        // Ring buffer hit its cap and dropped old lines, so an incremental
        // tail would stall at CACHE_MAX forever. Refetch the whole tail.
        fresh = await getLogs(0);
        lines = fresh.slice(-CACHE_MAX);
      } else if (fresh.length > 0) {
        lines = [...lines, ...fresh].slice(-CACHE_MAX);
      }
      error = null;
    } catch (e) {
      error = formatError(e);
    } finally {
      polling = false;
    }
  }

  // Poll while the drawer is mounted (open): immediately, then every POLL_MS.
  // Cleanup on close clears the timer. poll() mutates `lines`/`polling`, so
  // every call must be untracked — otherwise those state reads would re-trigger
  // this effect on each poll, restarting the interval forever (OOM loop).
  $effect(() => {
    untrack(() => void poll());
    const timer = setInterval(() => void poll(), POLL_MS);
    return () => clearInterval(timer);
  });

  // Keep the tail in view while the user is at the bottom; leave the scroll
  // position alone if they've scrolled up to read history.
  $effect(() => {
    if (listEl && followTail && lines.length > 0) {
      listEl.scrollTop = listEl.scrollHeight;
    }
  });

  function handleScroll() {
    followTail = nearBottom();
  }
</script>

<div class="logs-drawer">
  <header class="drawer-header">
    <span class="drawer-title">
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
        <polyline points="4 17 10 11 4 5" /><line
          x1="12"
          y1="19"
          x2="20"
          y2="19"
        />
      </svg>
      Logs
    </span>
    <span class="drawer-hint">worker stderr</span>
    <button class="close-btn" onclick={onClose} title="Close logs">
      <svg
        width="14"
        height="14"
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
  </header>

  <div class="logs-body" bind:this={listEl} onscroll={handleScroll}>
    {#if lines.length === 0 && !error}
      <div class="empty">
        No log lines yet — worker stderr will appear here.
      </div>
    {:else}
      {#each lines as line, i (i)}
        <div class="log-line">{line}</div>
      {/each}
      {#if error}
        <div class="log-line error">{error}</div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .logs-drawer {
    position: fixed;
    right: 16px;
    bottom: 16px;
    width: 640px;
    max-width: calc(100vw - 32px);
    height: 360px;
    display: flex;
    flex-direction: column;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
    z-index: 1500;
    overflow: hidden;
  }
  .drawer-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px 8px 14px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-subtle);
    flex-shrink: 0;
    user-select: none;
  }
  .drawer-title {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: var(--text-md);
    font-weight: var(--weight-semibold);
    color: var(--text);
  }
  .drawer-title svg {
    color: var(--accent);
  }
  .drawer-hint {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-family: var(--font-mono);
  }
  .close-btn {
    margin-left: auto;
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
  }
  .close-btn:hover {
    background: var(--danger-bg);
    color: var(--danger);
  }
  .logs-body {
    flex: 1;
    overflow-y: auto;
    padding: 6px 0;
    font-family: var(--font-mono);
    font-size: 11.5px;
    line-height: 1.55;
    background: var(--bg);
  }
  .log-line {
    padding: 0 14px;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text-secondary);
  }
  .log-line.error {
    color: var(--danger);
  }
  .empty {
    padding: 24px 14px;
    text-align: center;
    color: var(--text-muted);
    font-family: var(--font-sans);
    font-size: var(--text-sm);
  }
</style>
