<script lang="ts">
  import ProviderPicker from '../ProviderPicker.svelte';
  import SettingsGroup from './SettingsGroup.svelte';
  import SettingRow from './SettingRow.svelte';
  import PaneSection from './PaneSection.svelte';
  import {
    aiConfig,
    type AiProviderId,
  } from '../../../stores/ai-config.svelte.ts';
  import type { InstalledAcpAgent } from '../../../ipc/ai.ts';

  let {
    apiKey = $bindable(''),
    installedAgents,
    onProviderChange,
  }: {
    apiKey?: string;
    installedAgents: InstalledAcpAgent[];
    /** ProviderPicker reports a plain id; the shell narrows it. */
    onProviderChange: (id: string, agentId?: string) => void;
  } = $props();

  let showKey = $state(false);

  /** Only these two talk to a base URL the user chooses. */
  const SHOWS_ENDPOINT = new Set(['ollama', 'custom']);
  let showsEndpoint = $derived(SHOWS_ENDPOINT.has(aiConfig.provider));
  let isAcp = $derived(aiConfig.provider === 'acp');
</script>

<PaneSection title="Provider">
  <ProviderPicker
    value={aiConfig.provider}
    acpAgentId={aiConfig.acp?.agentId}
    {installedAgents}
    onChange={onProviderChange}
  />
</PaneSection>

{#if isAcp}
  <!-- An agent authenticates itself, so there is no key to enter here. The
       old layout hid the key field and left an empty card behind; saying why
       the field is absent is more useful than an absence. -->
  <SettingsGroup
    title="Authentication"
    hint="Agents sign in through their own CLI, so Lucent stores no key for them. Configure the agent under Agents."
  >
    <SettingRow
      label="Signed in as"
      description="The agent handles its own credentials."
    >
      {#snippet control()}
        <code class="agent-id selectable"
          >{aiConfig.acp?.agentId ?? 'no agent selected'}</code
        >
      {/snippet}
    </SettingRow>
  </SettingsGroup>
{:else}
  <SettingsGroup
    title="Authentication"
    hint="Leave the key blank to keep the one already saved. Keys are stored in the OS keychain, never in the notebook file."
  >
    <!-- forId associates the row's own label with the input, so clicking the
         text focuses the field and the accessible name is the visible text. -->
    <SettingRow label="API Key" forId="ai-api-key">
      {#snippet control()}
        <span class="key-wrap">
          <input
            id="ai-api-key"
            type={showKey ? 'text' : 'password'}
            bind:value={apiKey}
            placeholder={showsEndpoint ? 'Optional' : 'sk-…'}
            autocomplete="off"
            spellcheck="false"
          />
          <button
            type="button"
            class="eye"
            onclick={() => (showKey = !showKey)}
            aria-label={showKey ? 'Hide API key' : 'Show API key'}
          >
            {#if showKey}
              <svg
                viewBox="0 0 24 24"
                width="14"
                height="14"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
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
                width="14"
                height="14"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path
                  d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"
                />
                <circle cx="12" cy="12" r="3" />
              </svg>
            {/if}
          </button>
        </span>
      {/snippet}
    </SettingRow>

    {#if showsEndpoint}
      <SettingRow
        label="Endpoint"
        description={aiConfig.provider === 'ollama'
          ? 'Where the local Ollama server is listening.'
          : 'An OpenAI-compatible base URL.'}
        forId="ai-endpoint"
      >
        {#snippet control()}
          <input
            id="ai-endpoint"
            class="endpoint selectable"
            type="url"
            bind:value={aiConfig.endpoint}
            placeholder={aiConfig.provider === 'ollama'
              ? 'http://localhost:11434/v1'
              : 'https://your-endpoint/v1'}
            spellcheck="false"
          />
        {/snippet}
      </SettingRow>
    {/if}
  </SettingsGroup>
{/if}

<style>
  .key-wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
  }
  input {
    width: 240px;
    max-width: 100%;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    color: var(--text);
    font-size: var(--text-base);
  }
  .key-wrap input {
    padding-right: 28px;
    font-family: var(--font-mono);
  }
  input:focus-visible {
    outline: none;
    border-color: var(--accent);
    box-shadow: var(--ring);
  }
  .endpoint {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  .eye {
    position: absolute;
    right: 2px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-muted);
    cursor: pointer;
  }
  .eye:hover {
    color: var(--text);
    background: var(--bg-hover);
  }
  .agent-id {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }
</style>
