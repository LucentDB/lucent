<script lang="ts">
  import { onMount } from 'svelte';
  import {
    aiConfig,
    type AiProviderId,
  } from '../../stores/ai-config.svelte.ts';
  import {
    saveAiSettings,
    getAiSettings,
    listAiModels,
    listRegistryAgents,
    listInstalledAcpAgents,
    installAcpAgent,
    uninstallAcpAgent,
    type AiModelSummary,
    type RegistryAgentSummary,
    type InstalledAcpAgent,
  } from '../../ipc/ai.ts';
  import ProviderPane from './settings/ProviderPane.svelte';
  import ModelPane from './settings/ModelPane.svelte';
  import AgentsPane from './settings/AgentsPane.svelte';
  import DataPane from './settings/DataPane.svelte';

  let { onClose }: { onClose: () => void } = $props();

  let apiKey = $state('');
  let saving = $state(false);
  let err = $state('');

  let fetchStatus = $state<'idle' | 'loading' | 'success' | 'error'>('idle');
  let fetchedModels: AiModelSummary[] = $state([]);
  let fetchError = $state('');

  let agents: RegistryAgentSummary[] = $state([]);
  let installedAgents = $state<InstalledAcpAgent[]>([]);
  let acpLoading = $state(false);
  let acpError = $state('');

  // ── Sections ───────────────────────────────────────────────────────────
  // The dialog was one 740px scroll of five stacked cards. A rail plus one
  // visible pane is how the platform's own settings cluster this much, and it
  // means a user changing a row limit never scrolls past an agent registry.
  type SectionId = 'provider' | 'model' | 'agents' | 'data';

  let isAcp = $derived(aiConfig.provider === 'acp');

  // Relevance follows the provider: a key-based provider has no agent to
  // configure, and an agent brings its own model. Hiding the pane is more
  // honest than showing one that explains it does not apply.
  let sections = $derived([
    { id: 'provider' as const, label: 'Provider' },
    ...(isAcp ? [] : [{ id: 'model' as const, label: 'Model' }]),
    { id: 'agents' as const, label: 'Agents' },
    { id: 'data' as const, label: 'Data & Safety' },
  ]);

  let active = $state<SectionId>('provider');

  // A section can disappear under the user when the provider changes.
  $effect(() => {
    if (!sections.some((s) => s.id === active)) active = 'provider';
  });

  let statusLabel = $derived(
    fetchStatus === 'idle'
      ? 'Not tested'
      : fetchStatus === 'loading'
        ? 'Fetching models…'
        : fetchStatus === 'success'
          ? 'Ready'
          : 'Failed',
  );
  let statusTitle = $derived(
    fetchStatus === 'idle'
      ? 'Not tested this session'
      : fetchStatus === 'loading'
        ? 'Fetching models…'
        : fetchStatus === 'success'
          ? 'Model list loaded'
          : 'Model fetch failed',
  );

  onMount(async () => {
    try {
      const cfg = await getAiSettings();
      if (cfg) {
        aiConfig.provider = cfg.provider ?? aiConfig.provider;
        aiConfig.endpoint = cfg.endpoint ?? aiConfig.endpoint;
        aiConfig.model = cfg.model ?? aiConfig.model;
        aiConfig.maxTokens = cfg.maxTokens ?? aiConfig.maxTokens;
        aiConfig.maxTurns = cfg.maxTurns ?? aiConfig.maxTurns;
        aiConfig.rowLimit = cfg.rowLimit ?? aiConfig.rowLimit;
        aiConfig.sampleColumnValues =
          cfg.sampleColumnValues ?? aiConfig.sampleColumnValues;
        aiConfig.enableBlastRadiusCheck =
          cfg.enableBlastRadiusCheck ?? aiConfig.enableBlastRadiusCheck;
        aiConfig.providerModels =
          cfg.providerModels && Object.keys(cfg.providerModels).length > 0
            ? cfg.providerModels
            : aiConfig.providerModels;
        aiConfig.acp = cfg.acp ?? null;
      }
    } catch (e) {
      err = `Could not load saved settings: ${e}`;
    }
    await refreshAgents();
    await refreshInstalledAgents();
  });

  async function refreshInstalledAgents() {
    try {
      installedAgents = (await listInstalledAcpAgents()) ?? [];
    } catch {
      installedAgents = [];
    }
  }

  async function refreshAgents() {
    acpLoading = true;
    acpError = '';
    try {
      agents = (await listRegistryAgents()) ?? [];
    } catch (e) {
      acpError = messageOf(e);
    } finally {
      acpLoading = false;
    }
  }

  function messageOf(e: unknown): string {
    return typeof e === 'string' ? e : ((e as Error)?.message ?? String(e));
  }

  async function handleInstall(agentId: string) {
    acpError = '';
    try {
      await installAcpAgent(agentId);
      await refreshAgents();
      await refreshInstalledAgents();
    } catch (e) {
      acpError = messageOf(e);
    }
  }

  async function handleUninstall(agentId: string) {
    acpError = '';
    try {
      await uninstallAcpAgent(agentId);
      await refreshAgents();
      await refreshInstalledAgents();
    } catch (e) {
      acpError = messageOf(e);
    }
  }

  // ProviderPicker's ids come from its own literal list, so the cast is the
  // one place that boundary is asserted rather than sprinkled through the pane.
  function handleProviderChange(rawId: string, agentId?: string) {
    const id = rawId as AiProviderId;
    aiConfig.provider = id;
    aiConfig.model = aiConfig.providerModels[id] ?? '';
    if (id === 'acp' && agentId) {
      aiConfig.acp = aiConfig.acp ?? {
        agentId,
        command: null,
        env: {},
        autoDenyPermissions: false,
      };
      aiConfig.acp.agentId = agentId;
    }
    if (id === 'ollama' && !aiConfig.endpoint) {
      aiConfig.endpoint = 'http://localhost:11434/v1';
    }
    fetchStatus = 'idle';
    fetchedModels = [];
  }

  function handleModelChange(id: string) {
    aiConfig.model = id;
    aiConfig.providerModels = {
      ...aiConfig.providerModels,
      [aiConfig.provider]: id,
    };
  }

  // ACP env overrides are edited as key/value rows and written back into
  // `aiConfig.acp.env` (a plain record) so Save sends the merged block.
  let envRows = $state<{ id: number; key: string; value: string }[]>([]);

  function syncEnvToConfig() {
    if (!aiConfig.acp) return;
    aiConfig.acp.env = Object.fromEntries(
      envRows
        .filter((r) => r.key.trim() !== '')
        .map((r) => [r.key.trim(), r.value]),
    );
  }

  $effect(() => {
    // Rebuild the rows whenever the selected agent changes so the editor never
    // shows another agent's env. `aiConfig.acp` may be null before the first
    // ACP selection — read it defensively.
    aiConfig.acp?.agentId;
    if (aiConfig.provider !== 'acp') return;
    const env = aiConfig.acp?.env ?? {};
    envRows = Object.entries(env).map(([key, value], i) => ({
      id: i,
      key,
      value,
    }));
  });

  async function fetchModels() {
    const requestedProvider = aiConfig.provider;
    fetchStatus = 'loading';
    try {
      const models = await listAiModels(
        aiConfig.provider,
        apiKey || undefined,
        aiConfig.endpoint || undefined,
      );
      if (aiConfig.provider !== requestedProvider) return;
      fetchedModels = models;
      fetchStatus = 'success';
    } catch (e) {
      if (aiConfig.provider !== requestedProvider) return;
      fetchError = messageOf(e);
      fetchStatus = 'error';
    }
  }

  let customEndpointMissing = $derived(
    aiConfig.provider === 'custom' && !aiConfig.endpoint?.trim(),
  );

  async function save() {
    saving = true;
    err = '';
    syncEnvToConfig();
    try {
      await saveAiSettings(
        {
          provider: aiConfig.provider,
          endpoint: aiConfig.endpoint || undefined,
          model: aiConfig.model,
          maxTokens: aiConfig.maxTokens,
          maxTurns: aiConfig.maxTurns,
          rowLimit: aiConfig.rowLimit,
          sampleColumnValues: aiConfig.sampleColumnValues,
          enableBlastRadiusCheck: aiConfig.enableBlastRadiusCheck,
          providerModels: aiConfig.providerModels,
        },
        apiKey || undefined,
        aiConfig.acp,
      );
      apiKey = '';
      onClose();
    } catch (e) {
      err = String(e);
    } finally {
      saving = false;
    }
  }

  /** A dialog has to close on Escape; the overlay's click-out is not enough. */
  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && !saving) {
      e.stopPropagation();
      onClose();
    }
  }

  /** Up/Down move between sections when the rail has focus, as a list does. */
  function onRailKeydown(e: KeyboardEvent) {
    const delta = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : 0;
    if (delta === 0) return;
    e.preventDefault();
    const i = sections.findIndex((s) => s.id === active);
    const next = Math.max(0, Math.min(sections.length - 1, i + delta));
    active = sections[next].id;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="settings"
  role="dialog"
  tabindex="-1"
  aria-modal="true"
  aria-labelledby="ai-settings-title"
  onkeydown={onKeydown}
>
  <header class="titlebar">
    <h2 id="ai-settings-title">AI Settings</h2>
  </header>

  <div class="split">
    <nav class="rail" aria-label="Settings sections" onkeydown={onRailKeydown}>
      {#each sections as section (section.id)}
        <button
          type="button"
          class="rail-item"
          class:active={active === section.id}
          aria-current={active === section.id ? 'page' : undefined}
          onclick={() => (active = section.id)}
        >
          {section.label}
        </button>
      {/each}
    </nav>

    <div class="pane" role="region" aria-label={statusTitle} tabindex="-1">
      {#if err}
        <p class="error" role="alert">{err}</p>
      {/if}

      {#if active === 'provider'}
        <ProviderPane
          bind:apiKey
          {installedAgents}
          onProviderChange={handleProviderChange}
        />
      {:else if active === 'model'}
        <ModelPane
          {fetchStatus}
          {fetchedModels}
          {fetchError}
          canFetch={!customEndpointMissing}
          onFetch={fetchModels}
          onModelChange={handleModelChange}
        />
      {:else if active === 'agents'}
        <AgentsPane
          {agents}
          loading={acpLoading}
          error={acpError}
          bind:envRows
          onInstall={handleInstall}
          onUninstall={handleUninstall}
          onEnvChange={syncEnvToConfig}
        />
      {:else}
        <DataPane />
      {/if}
    </div>
  </div>

  <footer class="footer">
    <!-- The connection status belongs beside Save, which is what commits it,
         not in the title bar where it read as the dialog's own state. -->
    <span class="status" title={statusTitle}>
      <span class="dot" data-status={fetchStatus}></span>
      {statusLabel}
    </span>
    <div class="spacer"></div>
    <button type="button" class="btn" onclick={onClose} disabled={saving}>
      Cancel
    </button>
    <button
      type="button"
      class="btn primary"
      onclick={save}
      disabled={saving || customEndpointMissing}
    >
      {saving ? 'Saving…' : 'Save'}
    </button>
  </footer>
</div>

<style>
  /* A fixed frame, not a growing scroll: the dialog is the same size whichever
     section is open, so switching sections never resizes the window. */
  .settings {
    display: flex;
    flex-direction: column;
    width: min(820px, calc(100vw - 64px));
    height: min(580px, calc(100vh - 80px));
    overflow: hidden;
  }

  .titlebar {
    display: flex;
    align-items: center;
    height: 38px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border-light);
    background: var(--bg-subtle);
    flex-shrink: 0;
  }
  .titlebar h2 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: var(--weight-semibold);
  }

  .split {
    display: grid;
    grid-template-columns: 176px minmax(0, 1fr);
    flex: 1;
    min-height: 0;
  }

  /* The sidebar surface, a step cooler than the content it indexes. */
  .rail {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 8px 8px;
    border-right: 1px solid var(--border-light);
    background: var(--bg-subtle);
    overflow-y: auto;
  }
  .rail-item {
    padding: 5px 8px;
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-secondary);
    font-size: var(--text-base);
    text-align: left;
    transition:
      background var(--transition-fast),
      color var(--transition-fast);
  }
  .rail-item:hover:not(.active) {
    background: var(--bg-hover);
    color: var(--text);
  }
  /* Solid accent for the current section, as a source-list selection. */
  .rail-item.active {
    background: var(--accent);
    color: var(--accent-foreground);
  }
  .rail-item:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .pane {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: 16px 18px;
    overflow-y: auto;
    background: var(--bg-surface);
    outline: none;
  }

  .footer {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 48px;
    padding: 0 14px;
    border-top: 1px solid var(--border-light);
    background: var(--bg-subtle);
    flex-shrink: 0;
  }
  .spacer {
    flex: 1;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--text-muted);
  }
  .dot[data-status='success'] {
    background: var(--success);
  }
  .dot[data-status='error'] {
    background: var(--danger);
  }
  .dot[data-status='loading'] {
    background: var(--warning);
  }

  .btn {
    min-width: 68px;
    padding: 4px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text);
    font-size: var(--text-base);
    box-shadow: var(--shadow-sm);
    transition:
      background var(--transition-fast),
      border-color var(--transition-fast);
  }
  .btn:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .btn.primary {
    border-color: transparent;
    background: var(--accent);
    color: var(--accent-foreground);
    font-weight: var(--weight-medium);
  }
  .btn.primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .error {
    margin: 0;
    padding: 7px 10px;
    border: 1px solid color-mix(in oklch, var(--danger) 35%, transparent);
    border-radius: var(--radius-sm);
    background: var(--danger-bg);
    color: var(--danger);
    font-size: var(--text-xs);
  }
</style>
