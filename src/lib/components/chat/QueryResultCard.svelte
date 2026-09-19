<script lang="ts">
  import { saveGoldenQuery } from '../../ipc/ai.ts';
  import { chat } from '../../stores/chat.svelte.ts';

  let {
    qr,
  }: { qr: { sql: string; rowCount: number; executionTimeMs: number } } =
    $props();

  let saved = $state(false);
  let saving = $state(false);
  let saveError = $state<string | null>(null);

  const activeConv = $derived(
    chat.conversations.find((c) => c.id === chat.activeConversationId),
  );

  async function handleSaveGolden() {
    if (saved || saving) return;
    saving = true;
    saveError = null;
    try {
      const connId = activeConv?.connectionId || 'global';
      const lastUserMsg =
        activeConv?.messages.filter((m) => m.role === 'user').at(-1)?.content ||
        'User query';
      await saveGoldenQuery(connId, lastUserMsg, qr.sql);
      saved = true;
    } catch (e) {
      // Surface the failure instead of silently reverting the button: the
      // user must not believe the query was saved when it was lost.
      saveError = e instanceof Error && e.message ? e.message : String(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="qr-card">
  <div class="qr-sql selectable"><code>{qr.sql}</code></div>
  <div class="qr-meta">
    <span class="qr-badge">{qr.rowCount} rows</span>
    <span class="qr-time">{qr.executionTimeMs}ms</span>
    {#if saveError}
      <span class="qr-save-error selectable" role="alert" title={saveError}
        >Save failed: {saveError}</span
      >
    {/if}
    <button
      class="qr-golden-btn"
      class:saved
      onclick={handleSaveGolden}
      disabled={saving || saved}
      title="Save as verified Golden Query for future retrieval"
    >
      {#if saved}
        ✓ Saved
      {:else if saving}
        Saving...
      {:else}
        ⭐ Save as Golden Query
      {/if}
    </button>
  </div>
</div>

<style>
  .qr-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
    margin: 8px 0;
  }
  .qr-sql {
    padding: 8px 12px;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }
  .qr-sql code {
    font-family: inherit;
    color: var(--accent);
  }
  .qr-meta {
    display: flex;
    gap: 8px;
    padding: 4px 12px;
    background: var(--bg-subtle);
    align-items: center;
  }
  .qr-badge {
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    color: var(--success);
  }
  .qr-time {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .qr-save-error {
    font-size: var(--text-xs);
    color: var(--error);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .qr-golden-btn {
    margin-left: auto;
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-muted);
    font-size: 11px;
    padding: 2px 7px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s;
  }
  .qr-golden-btn:hover:not(:disabled) {
    color: #fbbf24;
    border-color: #fbbf24;
    background: rgba(251, 191, 36, 0.1);
  }
  .qr-golden-btn.saved {
    color: var(--success);
    border-color: var(--success);
    background: rgba(16, 185, 129, 0.1);
  }
</style>
