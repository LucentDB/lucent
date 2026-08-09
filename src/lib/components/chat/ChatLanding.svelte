<script lang="ts">
  // The AI copilot's empty state. Rendered in two very different contexts:
  // the right half of the disconnected split screen (App.svelte) and the
  // connected empty state inside the resizable chat panel (ChatPanel.svelte).
  // Both use this one component, so the two can no longer drift apart.
  import ChatInput from './ChatInput.svelte';
  import Icon from '../icons/Icon.svelte';
  import { aiConfig } from '../../stores/ai-config.svelte.ts';
  import { history } from '../../stores/history.svelte.ts';
  import { schemaSummary } from '../../stores/schema-summary.svelte.ts';
  import { buildSuggestions, CAPABILITIES } from './suggestions.ts';
  import {
    dedupeBySql,
    describeEntry,
    excerptSql,
    explainPrompt,
    splitExcerpt,
    RECENT_LIMIT,
  } from './recent-queries.ts';

  let {
    onSend,
    connected = false,
    database = null,
    connectionName = null,
    onOpenSettings,
  }: {
    onSend: (msg: string) => void;
    /**
     * Whether a database connection is live. Passed in rather than read from
     * the connections store because inline connections leave `activeProfileId`
     * null there — App.svelte holds the real answer.
     */
    connected?: boolean;
    database?: string | null;
    connectionName?: string | null;
    onOpenSettings?: () => void;
  } = $props();

  const suggestions = $derived(
    buildSuggestions(
      schemaSummary.loaded
        ? { schema: schemaSummary.schema, tables: schemaSummary.tables }
        : null,
    ),
  );

  // Browsing a table re-runs the same statement per page, so raw history is
  // often one query repeated — deduped before slicing, or the list would be
  // three identical rows.
  const recents = $derived(
    connected && !history.error
      ? dedupeBySql(history.entries).slice(0, RECENT_LIMIT)
      : [],
  );

  const contextParts = $derived.by(() => {
    if (!connected) return [];
    const parts: string[] = [];
    if (connectionName) parts.push(connectionName);
    if (database && database !== connectionName) parts.push(database);
    return parts;
  });

  // HistoryPanel loads history in every other path, so fetch here only when
  // the landing is the first thing a user sees. The guard is a plain boolean,
  // not $state: a successful load that returns zero rows would otherwise
  // re-satisfy this condition and loop forever.
  let historyRequested = false;
  $effect(() => {
    if (!connected || historyRequested) return;
    if (!history.loading && history.entries.length === 0) {
      historyRequested = true;
      history.loadHistory();
    }
  });
</script>

<div class="landing">
  <div class="inner">
    <!-- Provenance: what am I attached to, and which model answers -->
    <div class="context" style="--i: 0">
      <span
        class="status-dot"
        class:live={connected}
        aria-label={connected ? 'Connected' : 'Not connected'}
      ></span>
      {#if connected}
        <span class="context-parts">
          {#each contextParts as part, i}
            {#if i > 0}<span class="sep" aria-hidden="true">/</span>{/if}
            <span class="part">{part}</span>
          {/each}
        </span>
        <span class="context-right">
          {#if aiConfig.model}
            <span class="model">{aiConfig.model}</span>
          {/if}
          {#if onOpenSettings}
            <button
              class="settings-btn"
              onclick={onOpenSettings}
              title="AI settings"
              aria-label="AI settings"
            >
              <Icon name="settings" size={13} />
            </button>
          {/if}
        </span>
      {:else}
        <span class="context-parts">
          <span class="part">No database connected</span>
        </span>
      {/if}
    </div>

    <div class="hero" style="--i: 1">
      <div class="mark-wrap">
        <div class="mark"><Icon name="sparkle" size={22} /></div>
      </div>
      <h1>AI Copilot</h1>
      {#if connected}
        <p>
          Ask about {#if database}<strong>{database}</strong>{:else}your
            database{/if} in plain English — it reads your schema and writes the SQL.
        </p>
      {:else}
        <p>
          Connect a database and the copilot will read your schema, write the
          SQL, and run it for you.
        </p>
      {/if}
    </div>

    <div class="composer" style="--i: 2">
      <ChatInput
        {onSend}
        docked={false}
        disabled={!connected}
        placeholder={connected
          ? `Ask anything about ${database ?? 'your database'}…`
          : 'Connect a database to start asking…'}
        hint={connected
          ? 'Enter to send · Shift+Enter for newline'
          : 'The copilot needs a live connection to read your schema.'}
      />
    </div>

    <div class="columns" style="--i: 3">
      <section class="col">
        <h2 class="col-label">
          <span>{connected ? 'Try asking' : 'What it can do'}</span>
          <span class="rule" aria-hidden="true"></span>
        </h2>
        <div class="items">
          {#if connected}
            {#each suggestions as s (s.prompt)}
              <button class="item chip" onclick={() => onSend(s.prompt)}>
                <span class="tile"><Icon name={s.icon} size={13} /></span>
                <span class="item-label">{s.label}</span>
                <span class="go" aria-hidden="true">
                  <Icon name="arrow" size={13} />
                </span>
              </button>
            {/each}
          {:else}
            {#each CAPABILITIES as c (c.text)}
              <div class="item capability">
                <span class="tile"><Icon name={c.icon} size={13} /></span>
                <span class="item-label wrap">{c.text}</span>
              </div>
            {/each}
          {/if}
        </div>
      </section>

      {#if recents.length > 0}
        <section class="col recents">
          <h2 class="col-label">
            <span>Recent queries</span>
            <span class="rule" aria-hidden="true"></span>
          </h2>
          <div class="items">
            {#each recents as entry (entry.id)}
              {@const parts = splitExcerpt(excerptSql(entry.sql))}
              <button
                class="item recent"
                class:failed={entry.status === 'error'}
                onclick={() => onSend(explainPrompt(entry))}
                title={entry.sql}
              >
                <span class="tile"><Icon name="replay" size={13} /></span>
                <span class="recent-text">
                  <span class="sql">
                    {#if parts.verb}<span class="verb">{parts.verb}</span>{/if}
                    <span class="sql-rest">{parts.rest}</span>
                  </span>
                  <span class="recent-meta">{describeEntry(entry)}</span>
                </span>
                <span class="go" aria-hidden="true">
                  <Icon name="arrow" size={13} />
                </span>
              </button>
            {/each}
          </div>
        </section>
      {/if}
    </div>
  </div>
</div>

<style>
  /*
   * Container queries, not media queries: this panel is user-resizable
   * between 280px and 50vw, so the viewport width says nothing useful about
   * how much room the content actually has.
   */
  .landing {
    container-type: inline-size;
    height: 100%;
    /* Deterministic scrolling when the content is taller than the panel:
       as a flex item, height alone can be treated as a hint. */
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 28px 20px;
  }
  .inner {
    width: 100%;
    max-width: 560px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  /* ── Provenance strip ── */
  .context {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }
  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--text-muted);
    flex-shrink: 0;
    /* A soft halo reads as "live" without an attention-seeking animation. */
    box-shadow: 0 0 0 3px var(--bg-hover);
  }
  .status-dot.live {
    background: var(--success);
    box-shadow: 0 0 0 3px var(--success-bg);
  }
  .context-parts {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
  }
  .part {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: var(--weight-medium);
  }
  .sep {
    color: var(--text-muted);
    opacity: 0.6;
    flex-shrink: 0;
  }
  .context-right {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-left: auto;
    min-width: 0;
    flex-shrink: 1;
  }
  /* The model is reference information, so it yields space before the
     connection name does. */
  .model {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-muted);
    padding: 2px 6px;
    border-radius: var(--radius-full);
    background: var(--bg-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .settings-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    transition: all var(--transition-fast);
  }
  .settings-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }

  /* ── Hero ── */
  .hero {
    text-align: center;
  }
  /* A faint accent bloom gives the mark presence without a heavy container. */
  .mark-wrap {
    display: flex;
    justify-content: center;
    margin-bottom: 12px;
    background: radial-gradient(
      circle at center,
      var(--accent-soft) 0%,
      transparent 68%
    );
    padding: 10px 0;
  }
  .mark {
    width: 42px;
    height: 42px;
    border-radius: 13px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #fff;
    background: linear-gradient(
      145deg,
      var(--accent) 0%,
      var(--accent-hover) 100%
    );
    box-shadow:
      var(--shadow-md),
      inset 0 1px 0 rgba(255, 255, 255, 0.3);
  }
  .hero h1 {
    font-size: var(--text-xl);
    font-weight: var(--weight-bold);
    letter-spacing: -0.02em;
    margin: 0 0 5px;
    color: var(--text);
  }
  .hero p {
    color: var(--text-secondary);
    font-size: var(--text-sm);
    line-height: 1.55;
    margin: 0 auto;
    max-width: 44ch;
    text-wrap: pretty;
  }
  .hero strong {
    color: var(--text);
    font-weight: var(--weight-semibold);
  }

  /* ── Columns ── */
  .columns {
    display: grid;
    grid-template-columns: 1fr;
    gap: 18px;
    align-items: start;
  }
  .col {
    min-width: 0;
  }
  /* Label plus hairline: an editorial divider that separates sections without
     boxing them in. */
  .col-label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 10px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.07em;
    font-weight: var(--weight-semibold);
    margin: 0 0 9px;
  }
  .col-label .rule {
    flex: 1;
    height: 1px;
    background: var(--border);
  }
  .items {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 9px 10px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--bg-surface);
    box-shadow: var(--shadow-sm);
    font-size: var(--text-sm);
    color: var(--text-secondary);
    text-align: left;
    min-width: 0;
    transition:
      border-color var(--transition-fast),
      background var(--transition-fast),
      color var(--transition-fast),
      box-shadow var(--transition-fast),
      transform var(--transition-fast);
  }
  .item.chip:hover,
  .item.recent:hover {
    border-color: var(--accent);
    color: var(--text);
    box-shadow: var(--shadow-md);
    transform: translateY(-1px);
  }
  .item.chip:active,
  .item.recent:active {
    transform: none;
    box-shadow: var(--shadow-sm);
  }
  /* Capabilities are informational, not actionable — no shadow, no lift. */
  .item.capability {
    background: transparent;
    border-color: var(--border-light);
    box-shadow: none;
    cursor: default;
    align-items: flex-start;
  }

  .tile {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    border-radius: 7px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .item.recent.failed .tile {
    background: var(--danger-bg);
    color: var(--danger);
  }

  .item-label {
    min-width: 0;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item-label.wrap {
    white-space: normal;
    line-height: 1.45;
  }

  /* The arrow only resolves on hover, so the resting state stays quiet. */
  .go {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    color: var(--accent);
    opacity: 0;
    transform: translateX(-3px);
    transition:
      opacity var(--transition-fast),
      transform var(--transition-fast);
  }
  .item.chip:hover .go,
  .item.recent:hover .go {
    opacity: 1;
    transform: none;
  }

  /* ── Recents ── */
  .recent {
    align-items: center;
  }
  .recent-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .sql {
    display: flex;
    gap: 5px;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text);
  }
  .verb {
    color: var(--accent);
    font-weight: var(--weight-semibold);
    flex-shrink: 0;
  }
  .sql-rest {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .recent-meta {
    font-size: 10px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .item.recent.failed .recent-meta {
    color: var(--danger);
  }

  /*
   * Wide enough for two columns — the full-width chat view. The disconnected
   * split half is pinned to 420px and has no recents column, so it correctly
   * never crosses this threshold.
   */
  @container (min-width: 560px) {
    .inner {
      max-width: 760px;
      gap: 22px;
    }
    .columns {
      grid-template-columns: 1fr 1fr;
      gap: 28px;
    }
    .mark {
      width: 46px;
      height: 46px;
      border-radius: 14px;
    }
    .hero h1 {
      font-size: var(--text-2xl);
    }
    .hero p {
      font-size: var(--text-md);
    }
  }

  /* ── Entry motion ── */
  .context,
  .hero,
  .composer,
  .columns {
    animation: rise 200ms ease-out backwards;
    animation-delay: calc(var(--i, 0) * 55ms);
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(5px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .context,
    .hero,
    .composer,
    .columns {
      animation: none;
    }
  }
</style>
