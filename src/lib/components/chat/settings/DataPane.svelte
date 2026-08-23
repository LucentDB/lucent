<script lang="ts">
  import SettingsGroup from './SettingsGroup.svelte';
  import SettingRow from './SettingRow.svelte';
  import Switch from './Switch.svelte';
  import { aiConfig } from '../../../stores/ai-config.svelte.ts';
</script>

<!-- Row limit and sampling both govern how much of the database the model is
     allowed to pull in, so they belong together. Both were already saved and
     sent on every request; neither had a control anywhere in the dialog. -->
<SettingsGroup title="Data the model may read">
  <SettingRow
    label="Row limit"
    description="Most rows the model may read back from one query."
    forId="ai-row-limit"
  >
    {#snippet control()}
      <input
        id="ai-row-limit"
        type="number"
        min="1"
        max="10000"
        step="100"
        bind:value={aiConfig.rowLimit}
      />
    {/snippet}
  </SettingRow>
  <SettingRow
    label="Sample column values"
    description="Reads up to 1,000 rows per column so the schema index can match on contents, not just names."
  >
    {#snippet control()}
      <Switch
        bind:checked={aiConfig.sampleColumnValues}
        label="Sample column values"
      />
    {/snippet}
  </SettingRow>
</SettingsGroup>

<SettingsGroup title="Safety">
  <SettingRow
    label="Confirm before writes"
    description="Estimates how many rows a DML statement would touch and asks first."
  >
    {#snippet control()}
      <Switch
        bind:checked={aiConfig.enableBlastRadiusCheck}
        label="Confirm before writes"
      />
    {/snippet}
  </SettingRow>
</SettingsGroup>

<style>
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
