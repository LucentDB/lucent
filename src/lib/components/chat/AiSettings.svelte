<script lang="ts">
  import { onMount } from 'svelte';
  import { aiConfig } from '../../stores/ai-config.svelte.ts';
  import { saveAiSettings, getAiSettings } from '../../ipc/ai.ts';
  let { onClose }: { onClose: () => void } = $props();
  let apiKey = $state('');
  let saving = $state(false);
  let err = $state('');

  // Hydrate the form from persisted config before the user edits it, so Save
  // doesn't overwrite real settings with store defaults. Backend AiConfig is
  // camelCase, matching the store field-for-field.
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
        aiConfig.sendResultsToAi =
          cfg.sendResultsToAi ?? aiConfig.sendResultsToAi;
        aiConfig.enableBlastRadiusCheck =
          cfg.enableBlastRadiusCheck ?? aiConfig.enableBlastRadiusCheck;
      }
    } catch (e) {
      err = `Could not load saved settings: ${e}`;
    }
  });

  async function save() {
    saving = true;
    err = '';
    try {
      await saveAiSettings(
        {
          provider: aiConfig.provider,
          endpoint: aiConfig.endpoint || undefined,
          model: aiConfig.model,
          maxTokens: aiConfig.maxTokens,
          maxTurns: aiConfig.maxTurns,
          rowLimit: aiConfig.rowLimit,
          sendResultsToAi: aiConfig.sendResultsToAi,
          enableBlastRadiusCheck: aiConfig.enableBlastRadiusCheck,
        },
        apiKey || undefined,
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
  <h2>AI Settings</h2>
  {#if err}<div class="error">{err}</div>{/if}

  <label
    >Provider
    <select bind:value={aiConfig.provider}>
      <option value="openai">OpenAI</option>
      <option value="anthropic">Anthropic</option>
      <option value="ollama">Ollama (local)</option>
    </select>
  </label>
  <label
    >API Key
    <input
      type="password"
      bind:value={apiKey}
      placeholder={aiConfig.provider === 'ollama' ? 'Not required' : 'sk-…'}
    />
    <span class="hint"
      >Leave blank to keep existing. Stored in OS keychain.</span
    >
  </label>
  {#if aiConfig.provider === 'ollama'}
    <label
      >Endpoint URL
      <input
        type="url"
        bind:value={aiConfig.endpoint}
        placeholder="http://localhost:11434"
      />
    </label>
  {/if}
  <label
    >Model
    <input type="text" bind:value={aiConfig.model} />
  </label>
  <label class="cb"
    ><input type="checkbox" bind:checked={aiConfig.sendResultsToAi} /> Send query
    results to AI for analysis</label
  >
  <label class="cb"
    ><input type="checkbox" bind:checked={aiConfig.enableBlastRadiusCheck} /> Show
    estimated rows before DML</label
  >

  <div class="actions">
    <button class="sec" onclick={onClose} disabled={saving}>Cancel</button>
    <button class="pri" onclick={save} disabled={saving}
      >{saving ? 'Saving…' : 'Save'}</button
    >
  </div>
</div>

<style>
  .settings {
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 400px;
  }
  h2 {
    margin: 0;
    font-size: 18px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 14px;
    font-weight: 500;
  }
  input,
  select {
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-size: 14px;
  }
  .hint {
    font-size: 12px;
    color: var(--text-secondary);
    font-weight: 400;
  }
  .cb {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    font-weight: 400;
  }
  .error {
    background: #fee2e2;
    color: #dc2626;
    padding: 8px 12px;
    border-radius: var(--radius-sm);
    font-size: 13px;
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 4px;
  }
  .pri {
    background: var(--accent);
    color: #fff;
    border: none;
    padding: 8px 20px;
    border-radius: var(--radius-sm);
    cursor: pointer;
    font-weight: 600;
  }
  .sec {
    background: var(--bg-hover);
    border: 1px solid var(--border);
    padding: 8px 20px;
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
</style>
