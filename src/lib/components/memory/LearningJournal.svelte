<script lang="ts">
  import type { Observation } from '../../ipc/ai.ts';

  let {
    observations = [],
    loading = false,
  }: {
    observations?: Observation[];
    loading?: boolean;
  } = $props();

  /**
   * The observation's payload is raw JSON captured before distillation. Show a
   * short, human-readable single-line summary rather than the full blob; never
   * throw on malformed input (older rows may predate the schema).
   */
  function payloadSummary(payloadJson: string): string {
    if (!payloadJson) return '';
    try {
      const parsed: unknown = JSON.parse(payloadJson);
      if (parsed && typeof parsed === 'object') {
        const entries = Object.entries(parsed as Record<string, unknown>);
        if (entries.length === 0) return '';
        const [key, value] = entries[0];
        const rendered =
          typeof value === 'string' ? value : JSON.stringify(value);
        return `${key}: ${rendered}`.slice(0, 140);
      }
      return String(parsed).slice(0, 140);
    } catch {
      return payloadJson.slice(0, 140);
    }
  }

  function formatTimestamp(seconds: number): string {
    if (!seconds) return '';
    return new Date(seconds * 1000).toLocaleString();
  }
</script>

<div class="learning-journal">
  {#if loading}
    <div class="journal-empty">Loading observations...</div>
  {:else if observations.length === 0}
    <div class="journal-empty">
      No observations recorded yet. As you chat, Lucent journals corrections,
      preferences, and schema notes here before distilling them into memories.
    </div>
  {:else}
    <ul class="journal-list" role="list">
      {#each observations as obs (obs.id)}
        <li class="journal-item">
          <div class="journal-header">
            <span class="journal-signal">{obs.signal}</span>
            <span class="journal-kind">{obs.kind}</span>
            <span class="journal-origin">{obs.origin}</span>
          </div>
          <div class="journal-payload selectable">
            {payloadSummary(obs.payload_json)}
          </div>
          <div class="journal-meta">
            <span>{formatTimestamp(obs.created_at)}</span>
            {#if obs.occurrence_count > 1}
              <span class="journal-count">×{obs.occurrence_count}</span>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .learning-journal {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .journal-empty {
    padding: 32px 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: var(--text-sm, 13px);
  }
  .journal-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .journal-item {
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-md, 6px);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .journal-header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .journal-signal {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    padding: 1px 6px;
    border-radius: 4px;
    background: rgba(56, 189, 248, 0.15);
    color: #38bdf8;
  }
  .journal-kind {
    font-size: 11px;
    font-weight: 600;
    color: var(--text);
  }
  .journal-origin {
    margin-left: auto;
    font-size: 11px;
    padding: 1px 5px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.05);
    color: var(--text-muted);
  }
  .journal-payload {
    font-size: var(--text-xs, 12px);
    color: var(--text);
    line-height: 1.4;
    word-break: break-word;
  }
  .journal-meta {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--text-muted);
  }
  .journal-count {
    font-weight: 600;
  }
</style>
