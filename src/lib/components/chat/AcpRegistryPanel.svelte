<script lang="ts">
  import type { RegistryAgentSummary } from '../../ipc/ai.ts';

  let {
    agents = [] as RegistryAgentSummary[],
    loading = false,
    onInstall,
    onUninstall,
  } = $props<{
    agents: RegistryAgentSummary[];
    loading: boolean;
    onInstall: (id: string) => void;
    onUninstall: (id: string) => void;
  }>();
</script>

{#if loading}
  <div class="acp-panel-state">Loading agents…</div>
{:else if agents.length === 0}
  <div class="acp-panel-state">No agents available — check your connection</div>
{:else}
  <ul class="acp-list">
    {#each agents as agent (agent.id)}
      <li class="acp-row">
        <span class="acp-icon">
          <span class="acp-glyph" aria-hidden="true">▦</span>
          {#if agent.icon}
            <img
              src={agent.icon}
              alt={agent.name}
              loading="lazy"
              onerror={(e) => {
                (e.currentTarget as HTMLImageElement).style.display = 'none';
              }}
            />
          {/if}
        </span>
        <div class="acp-meta">
          <div class="acp-title-line">
            <span class="acp-name">{agent.name}</span>
            <span class="acp-version">v{agent.version}</span>
            {#if agent.installedVersion && agent.updateAvailable}
              <span class="acp-badge acp-badge-update">Update available</span>
            {/if}
            {#if agent.dbTools === 'supported'}
              <span
                class="acp-badge acp-badge-tools"
                title="Verified: this agent connects to Lucent's database tools (search_schema, get_objects_info, run_readonly_query, preview_dml) via ACP session MCP servers."
              >
                DB tools ✓
              </span>
            {:else if agent.dbTools === 'unsupported'}
              <span
                class="acp-badge acp-badge-tools-unsupported"
                title="Known limitation: this agent silently ignores the MCP servers Lucent passes in session/new, so Lucent's database tools can't reach its model. Chat still works; you can run SQL yourself in the query editor."
              >
                No DB tools
              </span>
            {/if}
            <span class="acp-license">{agent.license}</span>
          </div>
          <p class="acp-desc">{agent.description}</p>
        </div>
        <div class="acp-actions">
          {#if agent.installedVersion}
            {#if agent.updateAvailable}
              <button
                type="button"
                class="btn-sm btn-primary"
                onclick={() => onInstall(agent.id)}
              >
                Update
              </button>
            {/if}
            <button
              type="button"
              class="btn-sm"
              onclick={() => onUninstall(agent.id)}
            >
              Uninstall
            </button>
          {:else}
            <button
              type="button"
              class="btn-sm btn-primary"
              onclick={() => onInstall(agent.id)}
            >
              Install
            </button>
          {/if}
        </div>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .acp-panel-state {
    padding: 14px 4px;
    font-size: 13px;
    color: var(--text-secondary);
  }
  .acp-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: 320px;
    overflow-y: auto;
  }
  .acp-row {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-subtle);
  }
  .acp-icon {
    position: relative;
    flex-shrink: 0;
    width: 36px;
    height: 36px;
    border-radius: 9px;
    overflow: hidden;
    background: var(--bg-surface);
    border: 1px solid var(--border);
  }
  .acp-glyph {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    font-size: 16px;
    color: var(--text-muted);
  }
  .acp-icon img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .acp-meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .acp-title-line {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .acp-name {
    font-size: 14px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .acp-version {
    font-size: 12px;
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }
  .acp-license {
    font-size: 11px;
    color: var(--text-muted);
    padding: 2px 8px;
    border-radius: 99px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    font-weight: 500;
  }
  .acp-badge {
    font-size: 11px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 99px;
    color: var(--text-secondary);
    background: var(--bg-surface);
    border: 1px solid var(--border);
  }
  .acp-badge-update {
    color: #b45309;
    background: color-mix(in srgb, #f59e0b 12%, transparent);
    border-color: color-mix(in srgb, #f59e0b 35%, transparent);
  }
  .acp-badge-tools {
    color: #15803d;
    background: color-mix(in srgb, #22c55e 12%, transparent);
    border-color: color-mix(in srgb, #22c55e 35%, transparent);
  }
  .acp-badge-tools-unsupported {
    color: #b45309;
    background: color-mix(in srgb, #f59e0b 12%, transparent);
    border-color: color-mix(in srgb, #f59e0b 35%, transparent);
  }
  .acp-desc {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-secondary);
    line-height: 1.45;
  }
  .acp-actions {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex-shrink: 0;
  }
  .btn-sm {
    padding: 6px 14px;
    border-radius: var(--radius-md);
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    background: var(--bg-surface);
    color: var(--text);
    border: 1px solid var(--border);
    transition:
      background var(--transition-fast),
      transform var(--transition-fast);
  }
  .btn-sm:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .btn-sm.btn-primary {
    background: var(--accent);
    color: #fff;
    border-color: transparent;
  }
  .btn-sm.btn-primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }
</style>
