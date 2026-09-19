<script lang="ts">
  import SettingsGroup from './SettingsGroup.svelte';
  import SettingRow from './SettingRow.svelte';
  import Switch from './Switch.svelte';

  /**
   * Configuration for the one selected ACP agent.
   *
   * Split out of AgentsPane so `acp` can be a non-nullable prop: Svelte
   * compiles each `{#snippet}` to its own function, so a `{#if aiConfig.acp}`
   * guard in the parent does not narrow the type inside the snippets that
   * render the controls. Taking the block as a required prop moves the null
   * check to the one place that can do it, and the registry pane gets shorter
   * for it.
   */
  let {
    acp,
    envRows = $bindable([]),
    onEnvChange,
  }: {
    acp: {
      agentId: string;
      command: string | null;
      env: Record<string, string>;
      autoDenyPermissions: boolean;
    };
    envRows?: { id: number; key: string; value: string }[];
    onEnvChange: () => void;
  } = $props();

  /** Advanced fields stay folded: most agents need neither. */
  let showAdvanced = $state(false);

  // Rows are keyed by a monotonic id, not by array index. Keying an each-block
  // by index changes the key of every row after a removal, which is the exact
  // case a key exists to prevent: Svelte reuses the wrong input and the values
  // appear to shift up a row.
  let nextId = $state(0);
  function addRow() {
    envRows = [...envRows, { id: nextId++, key: '', value: '' }];
  }
  function removeRow(id: number) {
    envRows = envRows.filter((r) => r.id !== id);
    onEnvChange();
  }
</script>

<SettingsGroup title="Active agent">
  <SettingRow label="Selected agent">
    {#snippet control()}
      <code class="agent-id selectable">{acp.agentId}</code>
    {/snippet}
  </SettingRow>
  <SettingRow
    label="Auto-deny permission requests"
    description="Refuses the agent's tool-permission prompts immediately instead of asking you."
  >
    {#snippet control()}
      <Switch
        bind:checked={acp.autoDenyPermissions}
        label="Auto-deny the agent's tool-permission requests"
      />
    {/snippet}
  </SettingRow>
</SettingsGroup>

<div class="advanced">
  <button
    type="button"
    class="disclose"
    aria-expanded={showAdvanced}
    onclick={() => (showAdvanced = !showAdvanced)}
  >
    <svg
      class="chev"
      class:open={showAdvanced}
      width="10"
      height="10"
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M6 4l4 4-4 4"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
    Advanced
  </button>

  {#if showAdvanced}
    <SettingsGroup
      hint="Only needed when the agent is not on your PATH or wants extra environment variables."
    >
      <SettingRow label="Command override" forId="acp-command" stacked>
        {#snippet control()}
          <input
            id="acp-command"
            type="text"
            class="mono wide selectable"
            bind:value={acp.command}
            placeholder="npx @opencode/agent --headless"
            spellcheck="false"
          />
        {/snippet}
      </SettingRow>
      <SettingRow label="Environment variables" stacked>
        {#snippet control()}
          <div class="env">
            {#each envRows as row (row.id)}
              <div class="env-row">
                <input
                  type="text"
                  class="mono selectable"
                  placeholder="KEY"
                  bind:value={row.key}
                  aria-label="Environment key"
                  oninput={onEnvChange}
                  spellcheck="false"
                />
                <input
                  type="text"
                  class="mono selectable"
                  placeholder="value"
                  bind:value={row.value}
                  aria-label="Environment value"
                  oninput={onEnvChange}
                  spellcheck="false"
                />
                <button
                  type="button"
                  class="remove"
                  aria-label="Remove environment variable"
                  onclick={() => removeRow(row.id)}
                >
                  <svg
                    width="12"
                    height="12"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    aria-hidden="true"
                  >
                    <path d="M18 6 6 18M6 6l12 12" />
                  </svg>
                </button>
              </div>
            {/each}
            <button type="button" class="add" onclick={addRow}>
              Add variable
            </button>
          </div>
        {/snippet}
      </SettingRow>
    </SettingsGroup>
  {/if}
</div>

<style>
  .agent-id {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }
  .advanced {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .disclose {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    align-self: flex-start;
    padding: 2px 6px 2px 2px;
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }
  .disclose:hover {
    color: var(--text);
  }
  .chev {
    transition: transform var(--transition-normal);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .env {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
  }
  .env-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1.4fr) 22px;
    gap: 4px;
  }
  input {
    min-width: 0;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
    color: var(--text);
    font-size: var(--text-base);
  }
  input.mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
  }
  input.wide {
    width: 100%;
  }
  input:focus-visible {
    outline: none;
    border-color: var(--accent);
    box-shadow: var(--ring);
  }
  .remove,
  .add {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-muted);
  }
  .remove {
    width: 22px;
    height: 26px;
  }
  .remove:hover {
    color: var(--danger);
    background: var(--danger-bg);
  }
  .add {
    align-self: flex-start;
    padding: 3px 8px;
    border-color: var(--border);
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }
  .add:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
</style>
