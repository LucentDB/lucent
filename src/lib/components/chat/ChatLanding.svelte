<script lang="ts">
  // The AI copilot's empty state. Rendered in two very different contexts:
  // the right half of the disconnected split screen (App.svelte) and the
  // connected empty state inside the resizable chat panel (ChatPanel.svelte).
  // Both use this one component, so the two can no longer drift apart.
  import ChatInput from './ChatInput.svelte';
  import Icon from '../icons/Icon.svelte';
  import {
    aiConfig,
    getActiveAiModelDisplay,
  } from '../../stores/ai-config.svelte.ts';
  import { history } from '../../stores/history.svelte.ts';
  import { historyQuery } from '../../queries/history.ts';
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

  // The query mounts with the landing and shares the cache with HistoryPanel,
  // so no manual lazy-fetch is needed here any more.
  const entries = historyQuery(() => history.filter);

  // Browsing a table re-runs the same statement per page, so raw history is
  // often one query repeated — deduped before slicing, or the list would be
  // three identical rows.
  const recents = $derived(
    connected && !entries.error
      ? dedupeBySql(entries.data ?? []).slice(0, RECENT_LIMIT)
      : [],
  );

  const contextParts = $derived.by(() => {
    if (!connected) return [];
    const parts: string[] = [];
    if (connectionName) parts.push(connectionName);
    if (database && database !== connectionName) parts.push(database);
    return parts;
  });

  const activeModel = $derived(getActiveAiModelDisplay());
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
      {:else}
        <span class="context-parts">
          <span class="part">No database connected</span>
        </span>
      {/if}
      <span class="context-right">
        {#if activeModel}
          <span class="model">{activeModel}</span>
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
    </div>

    <!-- No mark and no title. This panel is resizable down to 280px, where a
         42px badge, an h1 and a centred two-line subtitle pushed the input
         and every suggestion below the fold. The input is the thing you came
         for, so it goes first, and one line says what it will do. -->
    <p class="intro selectable" style="--i: 1">
      {#if connected}
        Ask about {#if database}<strong>{database}</strong>{:else}your database{/if}.
        Lucent reads the schema and writes the SQL.
      {:else}
        Connect a database and Lucent will read your schema, write the SQL, and
        run it for you.
      {/if}
    </p>

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

    <div class="columns" class:single={recents.length === 0} style="--i: 3">
      <section class="col">
        <h2 class="col-label">
          {connected ? 'Try asking' : 'What it can do'}
        </h2>
        <div class="items">
          {#if connected}
            {#each suggestions as s (s.prompt)}
              <button class="item chip" onclick={() => onSend(s.prompt)}>
                <span class="item-label">{s.label}</span>
                <span class="go" aria-hidden="true">
                  <Icon name="arrow" size={13} />
                </span>
              </button>
            {/each}
          {:else}
            {#each CAPABILITIES as c (c.text)}
              <div class="item capability">
                <span class="item-label wrap selectable">{c.text}</span>
              </div>
            {/each}
          {/if}
        </div>
      </section>

      {#if recents.length > 0}
        <section class="col recents">
          <h2 class="col-label">Recent queries</h2>
          <div class="items">
            {#each recents as entry (entry.id)}
              {@const parts = splitExcerpt(excerptSql(entry.sql))}
              <button
                class="item recent"
                class:failed={entry.status === 'error'}
                onclick={() => onSend(explainPrompt(entry))}
                title={entry.sql}
              >
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
    padding: 1px 5px;
    border-radius: var(--radius-sm);
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

  /* ── Intro ── */
  /* One line, left aligned, at body size. It orients without competing with
     the input directly beneath it. */
  .intro {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-base);
    line-height: 1.5;
    max-width: 52ch;
    text-wrap: pretty;
  }
  .intro strong {
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
  .col-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-weight: var(--weight-medium);
    margin: 0 0 6px;
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
    padding: 7px 10px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--bg-elevated);
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
    background: var(--bg-hover);
  }
  .item.chip:active,
  .item.recent:active {
    background: var(--bg-subtle);
  }
  /* Capabilities are informational, not actionable. */
  .item.capability {
    background: transparent;
    border-color: var(--border-light);
    cursor: default;
    align-items: flex-start;
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
    /* Nothing to sit beside: the suggestions take the whole width rather
       than leaving a column of dead space. */
    .columns.single {
      grid-template-columns: 1fr;
    }
  }

  /* ── Entry motion ── */
  .context,
  .intro,
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
    .intro,
    .composer,
    .columns {
      animation: none;
    }
  }
</style>
