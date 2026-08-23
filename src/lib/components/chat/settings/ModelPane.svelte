<script lang="ts">
  import ModelPicker from '../ModelPicker.svelte';
  import SettingsGroup from './SettingsGroup.svelte';
  import SettingRow from './SettingRow.svelte';
  import PaneSection from './PaneSection.svelte';
  import { aiConfig } from '../../../stores/ai-config.svelte.ts';
  import type { AiModelSummary } from '../../../ipc/ai.ts';

  let {
    fetchStatus,
    fetchedModels,
    fetchError,
    canFetch,
    onFetch,
    onModelChange,
  }: {
    fetchStatus: 'idle' | 'loading' | 'success' | 'error';
    fetchedModels: AiModelSummary[];
    fetchError: string;
    canFetch: boolean;
    onFetch: () => void;
    onModelChange: (id: string) => void;
  } = $props();
</script>

<!-- No hint here: ModelPicker's idle state already says to press Fetch, and
     two lines of guidance for one button is one too many. -->
<PaneSection title="Model">
  {#snippet action()}
    <button
      type="button"
      class="fetch"
      onclick={onFetch}
      disabled={fetchStatus === 'loading' || !canFetch}
    >
      {fetchStatus === 'loading' ? 'Fetching…' : 'Fetch Models'}
    </button>
  {/snippet}
  <ModelPicker
    status={fetchStatus}
    models={fetchedModels}
    value={aiConfig.model}
    onChange={onModelChange}
    errorMessage={fetchError}
    providerLabel={aiConfig.provider}
  />
</PaneSection>

<!-- Both of these were already saved and already sent to the backend on every
     request; neither had a control anywhere in the dialog. -->
<SettingsGroup title="Generation limits">
  <SettingRow
    label="Max tokens"
    description="Ceiling on the length of one reply."
    forId="ai-max-tokens"
  >
    {#snippet control()}
      <input
        id="ai-max-tokens"
        type="number"
        min="256"
        max="200000"
        step="256"
        bind:value={aiConfig.maxTokens}
      />
    {/snippet}
  </SettingRow>
  <SettingRow
    label="Max turns"
    description="How many tool calls the agent may chain before it must answer."
    forId="ai-max-turns"
  >
    {#snippet control()}
      <input
        id="ai-max-turns"
        type="number"
        min="1"
        max="200"
        bind:value={aiConfig.maxTurns}
      />
    {/snippet}
  </SettingRow>
</SettingsGroup>

<style>
  .fetch {
    padding: 3px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-elevated);
    color: var(--text);
    font-size: var(--text-sm);
    box-shadow: var(--shadow-sm);
    transition:
      background var(--transition-fast),
      color var(--transition-fast);
  }
  .fetch:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .fetch:disabled {
    opacity: 0.5;
    cursor: default;
  }
  input[type='number'] {
    width: 96px;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    color: var(--text);
    font-size: var(--text-base);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  input:focus-visible {
    outline: none;
    border-color: var(--accent);
    box-shadow: var(--ring);
  }
</style>
