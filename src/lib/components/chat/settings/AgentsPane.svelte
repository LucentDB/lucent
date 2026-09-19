<script lang="ts">
  import AcpRegistryPanel from '../AcpRegistryPanel.svelte';
  import AcpAgentConfig from './AcpAgentConfig.svelte';
  import PaneSection from './PaneSection.svelte';
  import { aiConfig } from '../../../stores/ai-config.svelte.ts';
  import type { RegistryAgentSummary } from '../../../ipc/ai.ts';

  let {
    agents,
    loading,
    error,
    envRows = $bindable([]),
    onInstall,
    onUninstall,
    onEnvChange,
  }: {
    agents: RegistryAgentSummary[];
    loading: boolean;
    error: string;
    envRows?: { id: number; key: string; value: string }[];
    onInstall: (id: string) => void;
    onUninstall: (id: string) => void;
    onEnvChange: () => void;
  } = $props();

  // Configuration for the selected agent only appears when one is selected;
  // the registry below is always browsable, so an agent can be installed
  // before it is picked.
  let selected = $derived(aiConfig.provider === 'acp' ? aiConfig.acp : null);
</script>

{#if selected}
  <AcpAgentConfig acp={selected} bind:envRows {onEnvChange} />
{/if}

<PaneSection
  title="Registry"
  hint="Installed agents become selectable as providers. Lucent's database tools work with any of them."
>
  {#if error}
    <p class="error selectable" role="alert">{error}</p>
  {/if}
  <AcpRegistryPanel {agents} {loading} {onInstall} {onUninstall} />
</PaneSection>

<style>
  .error {
    margin: 0 0 var(--space-2);
    padding: 6px 8px;
    border: 1px solid color-mix(in oklch, var(--danger) 35%, transparent);
    border-radius: var(--radius-sm);
    background: var(--danger-bg);
    color: var(--danger);
    font-size: var(--text-xs);
  }
</style>
