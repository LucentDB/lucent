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
    installAcpAgent,
    uninstallAcpAgent,
    type AiModelSummary,
    type RegistryAgentSummary,
  } from '../../ipc/ai.ts';
  import ProviderPicker from './ProviderPicker.svelte';
  import ModelPicker from './ModelPicker.svelte';
  import AcpRegistryPanel from './AcpRegistryPanel.svelte';

  let { onClose }: { onClose: () => void } = $props();
  let apiKey = $state('');
  let saving = $state(false);
  let err = $state('');

  let fetchStatus = $state<'idle' | 'loading' | 'success' | 'error'>('idle');
  let fetchedModels: AiModelSummary[] = $state([]);
  let fetchError = $state('');

  let statusTitle = $derived(
    fetchStatus === 'idle'
      ? 'Not tested this session'
      : fetchStatus === 'loading'
        ? 'Fetching models…'
        : fetchStatus === 'success'
          ? 'Model list loaded'
          : 'Model fetch failed',
  );

  let showKey = $state(false);

  // Agents (ACP) registry section — browsable regardless of the selected
  // provider, so the user can install an agent before picking it.
  let agents: RegistryAgentSummary[] = $state([]);
  let acpLoading = $state(false);
  let acpError = $state('');
  let statusLabel = $derived(
    fetchStatus === 'idle'
      ? 'Not tested'
      : fetchStatus === 'loading'
        ? 'Fetching models…'
        : fetchStatus === 'success'
          ? 'Ready'
          : 'Failed',
  );

  const SHOWS_ENDPOINT = new Set(['ollama', 'custom']);

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
  });

  async function refreshAgents() {
    acpLoading = true;
    acpError = '';
    try {
      const list = await listRegistryAgents();
      agents = list ?? [];
    } catch (e) {
      acpError =
        typeof e === 'string' ? e : ((e as Error)?.message ?? String(e));
    } finally {
      acpLoading = false;
    }
  }

  async function handleInstall(agentId: string) {
    acpError = '';
    try {
      await installAcpAgent(agentId);
      await refreshAgents();
    } catch (e) {
      acpError =
        typeof e === 'string' ? e : ((e as Error)?.message ?? String(e));
    }
  }

  async function handleUninstall(agentId: string) {
    acpError = '';
    try {
      await uninstallAcpAgent(agentId);
      await refreshAgents();
    } catch (e) {
      acpError =
        typeof e === 'string' ? e : ((e as Error)?.message ?? String(e));
    }
  }

  function handleProviderChange(id: AiProviderId, agentId?: string) {
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

  // ACP env overrides are edited as key/value rows and written back into
  // `aiConfig.acp.env` (a plain record) so Save sends the merged block.
  let envRows = $state<{ key: string; value: string }[]>([]);

  function rebuildEnvRows() {
    const env = aiConfig.acp?.env ?? {};
    envRows = Object.entries(env).map(([key, value]) => ({ key, value }));
  }

  function addEnvRow() {
    envRows = [...envRows, { key: '', value: '' }];
  }

  function removeEnvRow(index: number) {
    envRows = envRows.filter((_, i) => i !== index);
  }

  function syncEnvToConfig() {
    if (!aiConfig.acp) return;
    aiConfig.acp.env = Object.fromEntries(
      envRows
        .filter((r) => r.key.trim() !== '')
        .map((r) => [r.key.trim(), r.value]),
    );
  }

  $effect(() => {
    // Rebuild the rows whenever the selected agent changes so the editor
    // never shows another agent's env. `aiConfig.acp` may be null before the
    // first ACP selection — read it defensively.
    aiConfig.acp?.agentId;
    if (aiConfig.provider === 'acp') rebuildEnvRows();
  });

  function handleModelChange(id: string) {
    aiConfig.model = id;
    aiConfig.providerModels = {
      ...aiConfig.providerModels,
      [aiConfig.provider]: id,
    };
  }

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
      fetchError =
        typeof e === 'string' ? e : ((e as Error)?.message ?? String(e));
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
</script>

<div class="settings">
  <div class="settings-header">
    <h2>AI Settings</h2>
    <span class="status" title={statusTitle} aria-label={statusTitle}>
      <span class="status-dot" data-status={fetchStatus}></span>
      {statusLabel}
    </span>
  </div>
  {#if err}<div class="error">{err}</div>{/if}

  <div class="settings-body">
    <section class="card">
      <h3 class="card-title">Provider &amp; Authentication</h3>
      <ProviderPicker
        value={aiConfig.provider}
        acpAgentId={aiConfig.acp?.agentId}
        onChange={handleProviderChange}
      />
      {#if aiConfig.provider === 'acp' && aiConfig.acp}
        <div class="acp-config">
          <div class="acp-agent-row">
            <span class="acp-agent-label">Selected agent</span>
            <span class="acp-agent-name">{aiConfig.acp.agentId}</span>
          </div>
          <label class="toggle-row">
            <input
              type="checkbox"
              bind:checked={aiConfig.acp.autoDenyPermissions}
            />
            <span class="toggle-track"><span class="toggle-thumb"></span></span>
            <span class="toggle-text"
              >Auto-deny the agent's tool-permission requests (no dialog)</span
            >
          </label>
          <details class="acp-advanced">
            <summary>Advanced</summary>
            <label class="field">
              Command override
              <input
                type="text"
                bind:value={aiConfig.acp.command}
                placeholder="e.g. npx @opencode/agent --headless"
              />
            </label>
            <div class="env-editor">
              <span class="env-caption">Environment variables</span>
              {#each envRows as row, i (i)}
                <div class="env-row">
                  <input
                    type="text"
                    placeholder="KEY"
                    bind:value={row.key}
                    aria-label="Environment key"
                    oninput={syncEnvToConfig}
                  />
                  <input
                    type="text"
                    placeholder="value"
                    bind:value={row.value}
                    aria-label="Environment value"
                    oninput={syncEnvToConfig}
                  />
                  <button
                    type="button"
                    class="env-remove"
                    aria-label="Remove environment variable"
                    onclick={() => removeEnvRow(i)}>×</button
                  >
                </div>
              {/each}
              <button type="button" class="env-add" onclick={addEnvRow}
                >Add variable</button
              >
            </div>
          </details>
        </div>
      {:else}
        <label class="field">
          API Key
          <span class="input-wrap">
            <input
              type={showKey ? 'text' : 'password'}
              bind:value={apiKey}
              placeholder={SHOWS_ENDPOINT.has(aiConfig.provider)
                ? 'Optional'
                : 'sk-…'}
            />
            <button
              type="button"
              class="eye-btn"
              onclick={() => (showKey = !showKey)}
              aria-label={showKey ? 'Hide API key' : 'Show API key'}
            >
              {#if showKey}
                <svg
                  viewBox="0 0 24 24"
                  width="16"
                  height="16"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path
                    d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"
                  />
                  <path d="M14.084 14.158a3 3 0 0 1-4.242-4.242" />
                  <path
                    d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"
                  />
                  <path d="m2 2 20 20" />
                </svg>
              {:else}
                <svg
                  viewBox="0 0 24 24"
                  width="16"
                  height="16"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path
                    d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"
                  />
                  <circle cx="12" cy="12" r="3" />
                </svg>
              {/if}
            </button>
          </span>
          <span class="hint"
            >Leave blank to keep existing. Stored in OS keychain.</span
          >
        </label>
        {#if SHOWS_ENDPOINT.has(aiConfig.provider)}
          <label class="field">
            Endpoint
            <input
              type="url"
              bind:value={aiConfig.endpoint}
              placeholder={aiConfig.provider === 'ollama'
                ? 'http://localhost:11434/v1'
                : 'https://your-endpoint/v1'}
            />
          </label>
        {/if}
      {/if}
    </section>

    {#if aiConfig.provider !== 'acp'}
      <section class="card">
        <h3 class="card-title">Model</h3>
        <button
          type="button"
          class="fetch-btn"
          onclick={fetchModels}
          disabled={fetchStatus === 'loading' || customEndpointMissing}
        >
          {fetchStatus === 'loading' ? 'Fetching…' : 'Fetch Models'}
        </button>
        <ModelPicker
          status={fetchStatus}
          models={fetchedModels}
          value={aiConfig.model}
          onChange={handleModelChange}
          errorMessage={fetchError}
          providerLabel={aiConfig.provider}
        />
      </section>
    {/if}
  </div>

  <section class="card">
    <h3 class="card-title">Agents (ACP)</h3>
    {#if acpError}<div class="error">{acpError}</div>{/if}
    <AcpRegistryPanel
      {agents}
      loading={acpLoading}
      onInstall={handleInstall}
      onUninstall={handleUninstall}
    />
  </section>

  <section class="card behavior-card">
    <label class="toggle-row">
      <input type="checkbox" bind:checked={aiConfig.sampleColumnValues} />
      <span class="toggle-track"><span class="toggle-thumb"></span></span>
      <span class="toggle-text"
        >Sample column values for the semantic index (reads up to 1,000 rows per
        column)</span
      >
    </label>
    <label class="toggle-row">
      <input type="checkbox" bind:checked={aiConfig.enableBlastRadiusCheck} />
      <span class="toggle-track"><span class="toggle-thumb"></span></span>
      <span class="toggle-text">Show estimated rows before DML</span>
    </label>
  </section>

  <div class="actions">
    <button
      type="button"
      class="btn btn-secondary"
      onclick={onClose}
      disabled={saving}
    >
      Cancel
    </button>
    <button
      type="button"
      class="btn btn-primary"
      onclick={save}
      disabled={saving || customEndpointMissing}
    >
      {saving ? 'Saving…' : 'Save'}
    </button>
  </div>
</div>

<style>
  .settings {
    width: min(740px, calc(100vw - 48px));
    padding: 24px 26px 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    max-height: calc(100vh - 64px);
    overflow-y: auto;
  }
  .settings-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-bottom: 14px;
    border-bottom: 1px solid var(--border-light);
  }
  .settings-header h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 650;
    letter-spacing: -0.02em;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-secondary);
    font-weight: 500;
    background: var(--bg-subtle);
    padding: 4px 10px 4px 8px;
    border-radius: 99px;
    border: 1px solid var(--border);
  }
  .settings-body {
    display: grid;
    grid-template-columns: minmax(0, 3fr) minmax(0, 2fr);
    gap: 14px;
    align-items: start;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px 18px;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
  }
  .behavior-card {
    display: flex;
    flex-wrap: wrap;
    gap: 10px 24px;
    padding: 14px 18px;
  }
  .card-title {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-muted);
  }
  .acp-config {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .acp-agent-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 13px;
  }
  .acp-agent-label {
    color: var(--text-muted);
    font-weight: 500;
  }
  .acp-agent-name {
    font-weight: 650;
    font-variant-numeric: tabular-nums;
  }
  .acp-advanced {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 8px 12px;
  }
  .acp-advanced summary {
    cursor: pointer;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text-secondary);
  }
  .acp-advanced[open] summary {
    margin-bottom: 10px;
  }
  .env-editor {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .env-caption {
    font-size: 11.5px;
    color: var(--text-muted);
    font-weight: 500;
  }
  .env-row {
    display: grid;
    grid-template-columns: 1fr 1fr auto;
    gap: 6px;
    align-items: center;
  }
  .env-row input {
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    font-size: 13px;
    font-family: var(--font-mono, monospace);
  }
  .env-remove {
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 15px;
    padding: 4px 6px;
    border-radius: var(--radius-sm);
  }
  .env-remove:hover {
    color: var(--error);
    background: var(--error-bg);
  }
  .env-add {
    align-self: flex-start;
    border: 1px dashed var(--border);
    background: transparent;
    color: var(--text-secondary);
    padding: 5px 12px;
    border-radius: var(--radius-sm);
    font-size: 12px;
    cursor: pointer;
  }
  .env-add:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 13px;
    font-weight: 500;
  }
  .field input {
    padding: 9px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    font-size: 13.5px;
    transition:
      border-color var(--transition-fast),
      box-shadow var(--transition-fast);
  }
  .field input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .input-wrap {
    position: relative;
    display: block;
  }
  .input-wrap input {
    width: 100%;
    padding: 9px 34px 9px 12px;
  }
  .eye-btn {
    position: absolute;
    right: 6px;
    top: 50%;
    transform: translateY(-50%);
    display: inline-flex;
    padding: 4px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: color var(--transition-fast);
  }
  .eye-btn:hover {
    color: var(--text);
  }
  .hint {
    font-size: 11.5px;
    color: var(--text-muted);
    font-weight: 400;
  }
  .fetch-btn {
    width: 100%;
    background: var(--accent);
    color: #fff;
    border: none;
    padding: 9px 16px;
    border-radius: var(--radius-md);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition:
      background var(--transition-fast),
      transform var(--transition-fast),
      box-shadow var(--transition-fast);
    box-shadow:
      0 1px 3px color-mix(in srgb, var(--accent) 40%, transparent),
      0 4px 10px color-mix(in srgb, var(--accent) 20%, transparent);
    letter-spacing: -0.01em;
  }
  .fetch-btn:hover:not(:disabled) {
    background: var(--accent-hover);
    transform: translateY(-1px);
    box-shadow:
      0 2px 6px color-mix(in srgb, var(--accent) 50%, transparent),
      0 6px 14px color-mix(in srgb, var(--accent) 25%, transparent);
  }
  .fetch-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
    box-shadow: none;
  }
  .toggle-row {
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: pointer;
    font-size: 13.5px;
    color: var(--text);
    user-select: none;
  }
  .toggle-row input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }
  .toggle-track {
    width: 36px;
    height: 20px;
    border-radius: 999px;
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    position: relative;
    transition:
      background var(--transition-normal),
      border-color var(--transition-normal);
    flex-shrink: 0;
  }
  .toggle-thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--text-muted);
    transition:
      transform var(--transition-normal),
      background var(--transition-normal);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
  }
  .toggle-row input:checked + .toggle-track {
    background: var(--accent);
    border-color: var(--accent);
  }
  .toggle-row input:checked + .toggle-track .toggle-thumb {
    transform: translateX(16px);
    background: #fff;
  }
  .toggle-row input:focus-visible + .toggle-track {
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 25%, transparent);
  }
  .error {
    background: var(--error-bg);
    color: var(--error);
    padding: 10px 14px;
    border-radius: var(--radius-md);
    font-size: 13px;
    border: 1px solid color-mix(in srgb, var(--error) 25%, transparent);
  }
  .actions {
    display: flex;
    gap: 10px;
    justify-content: flex-end;
    padding-top: 4px;
  }
  .btn {
    padding: 9px 22px;
    border-radius: var(--radius-md);
    font-size: 13.5px;
    cursor: pointer;
    font-weight: 500;
    letter-spacing: -0.01em;
    transition:
      background var(--transition-fast),
      transform var(--transition-fast),
      box-shadow var(--transition-fast);
  }
  .btn-primary {
    background: var(--accent);
    color: #fff;
    border: none;
    font-weight: 600;
    box-shadow:
      0 1px 3px color-mix(in srgb, var(--accent) 40%, transparent),
      0 4px 10px color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .btn-primary:hover:not(:disabled) {
    background: var(--accent-hover);
    transform: translateY(-1px);
  }
  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
    box-shadow: none;
  }
  .btn-secondary {
    background: var(--bg-surface);
    color: var(--text);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-sm);
  }
  .btn-secondary:hover:not(:disabled) {
    background: var(--bg-hover);
    border-color: color-mix(in srgb, var(--text) 30%, transparent);
  }
  .btn-secondary:disabled {
    opacity: 0.55;
  }
  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .status-dot[data-status='idle'] {
    background: #9ca3af;
  }
  .status-dot[data-status='loading'] {
    background: #f59e0b;
    animation: pulse 1.2s ease-in-out infinite;
  }
  .status-dot[data-status='success'] {
    background: #22c55e;
    box-shadow: 0 0 0 2px color-mix(in srgb, #22c55e 25%, transparent);
  }
  .status-dot[data-status='error'] {
    background: #ef4444;
    box-shadow: 0 0 0 2px color-mix(in srgb, #ef4444 25%, transparent);
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 0.4;
    }
    50% {
      opacity: 1;
    }
  }
</style>
