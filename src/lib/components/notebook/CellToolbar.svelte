<script lang="ts">
  import type { CellStatus } from '../../stores/notebook.svelte.ts';

  let {
    cellId,
    cellStatus,
    referencable = true,
    onCancel,
    onDelete,
    onMoveUp,
    onMoveDown,
  }: {
    cellId: string;
    cellStatus?: CellStatus;
    /** Markdown cells cannot be referenced, so they get no reference chip. */
    referencable?: boolean;
    onCancel?: () => void;
    onDelete?: () => void;
    onMoveUp?: () => void;
    onMoveDown?: () => void;
  } = $props();

  let isRunning = $derived(cellStatus === 'running');

  // The chip shows the literal paste-ready syntax rather than a bare hex id, so
  // the reference feature teaches itself instead of relying on recall.
  let ref = $derived('${' + cellId + '}');
  let copied = $state(false);
  let copyTimer: ReturnType<typeof setTimeout> | undefined;

  async function copyRef() {
    try {
      await navigator.clipboard?.writeText(ref);
    } catch (e) {
      // Clipboard can be denied (permissions, insecure context). Stay silent
      // rather than flashing a "copied" confirmation that did not happen.
      console.error('[notebook] failed to copy cell reference', e);
      return;
    }
    copied = true;
    clearTimeout(copyTimer);
    copyTimer = setTimeout(() => (copied = false), 1400);
  }

  $effect(() => () => clearTimeout(copyTimer));
</script>

<div class="cell-toolbar" role="toolbar" aria-label="Cell actions">
  {#if referencable}
    <button
      class="ref-chip"
      class:copied
      onclick={copyRef}
      title="Copy reference — paste into a later cell to use this cell's result"
      aria-label="Copy cell reference {ref}"
    >
      <code>{copied ? 'copied' : ref}</code>
    </button>
  {/if}

  {#if isRunning}
    <button
      class="toolbar-btn stop"
      onclick={onCancel}
      title="Stop cell"
      aria-label="Stop cell"
    >
      <svg
        width="9"
        height="9"
        viewBox="0 0 24 24"
        fill="currentColor"
        aria-hidden="true"
      >
        <rect x="6" y="6" width="12" height="12" rx="2" />
      </svg>
      Stop
    </button>
  {/if}

  <button
    class="toolbar-btn"
    onclick={onMoveUp}
    title="Move up (Alt+↑)"
    aria-label="Move cell up"
  >
    <svg
      width="11"
      height="11"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"><path d="M12 19V5M5 12l7-7 7 7" /></svg
    >
  </button>
  <button
    class="toolbar-btn"
    onclick={onMoveDown}
    title="Move down (Alt+↓)"
    aria-label="Move cell down"
  >
    <svg
      width="11"
      height="11"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"><path d="M12 5v14M19 12l-7 7-7-7" /></svg
    >
  </button>
  <button
    class="toolbar-btn danger"
    onclick={onDelete}
    title="Delete cell (dd)"
    aria-label="Delete cell"
  >
    <svg
      width="11"
      height="11"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"><path d="M3 6h18M8 6V4h8v2M6 6l1 14h10l1-14" /></svg
    >
  </button>
</div>

<style>
  .cell-toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  /* ─── Reference chip ───────────────────────────────────────────────
     Always present, but recessive: it is an identifier, not an action the user
     goes hunting for. The surface background keeps it legible when a long first
     line of SQL runs underneath it. */
  .ref-chip {
    display: inline-flex;
    align-items: center;
    height: 20px;
    margin-right: 4px;
    padding: 0 6px;
    border: 1px solid var(--border-light, var(--border));
    border-radius: var(--radius-sm);
    background: var(--bg-surface);
    color: var(--text-muted);
    cursor: pointer;
    opacity: 0;
    visibility: hidden;
    transition:
      opacity 0.15s,
      visibility 0.15s,
      background 0.15s,
      border-color 0.15s,
      color 0.15s;
  }
  .ref-chip code {
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: -0.01em;
    line-height: 1;
  }
  :global(.cell:hover) .ref-chip,
  :global(.cell.selected) .ref-chip,
  .ref-chip:focus-visible {
    opacity: 1;
    visibility: visible;
  }
  .ref-chip:hover {
    border-color: var(--accent-muted);
    background: var(--accent-soft);
    color: var(--accent);
  }
  .ref-chip.copied {
    opacity: 1;
    border-color: var(--accent-muted);
    background: var(--accent-soft);
    color: var(--accent);
  }
  .ref-chip:focus-visible {
    opacity: 1;
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  /* ─── Secondary actions ───────────────────────────────────────────
     visibility, not opacity alone, so a hidden button is also unclickable. */
  .toolbar-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 5px;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: var(--bg-surface);
    color: var(--text-muted);
    font-size: 11px;
    cursor: pointer;
    visibility: hidden;
    opacity: 0;
    transition:
      opacity 0.15s,
      background 0.15s,
      border-color 0.15s,
      color 0.15s;
  }
  :global(.cell:hover) .toolbar-btn,
  :global(.cell.selected) .toolbar-btn,
  .toolbar-btn:focus-visible {
    visibility: visible;
    opacity: 1;
  }
  .toolbar-btn:hover {
    border-color: var(--border);
    background: var(--bg-hover);
    color: var(--text);
  }
  /* A running cell needs its stop button unconditionally, not on hover. */
  .toolbar-btn.stop {
    visibility: visible;
    opacity: 1;
    border-color: color-mix(in srgb, var(--danger) 30%, transparent);
    color: var(--danger);
    font-weight: var(--weight-medium);
  }
  .toolbar-btn.stop:hover {
    background: var(--danger-bg);
    color: var(--danger);
  }
  .toolbar-btn.danger:hover {
    border-color: color-mix(in srgb, var(--danger) 30%, transparent);
    background: var(--danger-bg);
    color: var(--danger);
  }
  .toolbar-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
