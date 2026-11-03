<script lang="ts">
  import type { ConnectionProfile } from '../../stores/connections.svelte';

  let {
    profile = null,
    onSave,
    onClose,
  }: {
    profile?: Partial<ConnectionProfile> | null;
    onSave?: (profile: Partial<ConnectionProfile>) => void;
    onClose?: () => void;
  } = $props();

  let enableExternalAgents = $state(false);
  let allowQueryHistory = $state(false);
  let copied = $state(false);

  $effect(() => {
    enableExternalAgents = profile?.enableExternalAgents ?? false;
    allowQueryHistory = profile?.allowQueryHistory ?? false;
  });

  export function getMcpSnippet(): string {
    return JSON.stringify(
      {
        mcpServers: {
          lucent: {
            command: 'lucent-db-tools-mcp',
          },
        },
      },
      null,
      2,
    );
  }

  export async function copyMcpSnippet() {
    const snippet = getMcpSnippet();
    if (navigator?.clipboard?.writeText) {
      await navigator.clipboard.writeText(snippet);
    }
    copied = true;
    setTimeout(() => {
      copied = false;
    }, 2000);
  }

  function handleSave() {
    onSave?.({
      ...profile,
      enableExternalAgents,
      allowQueryHistory,
    });
  }
</script>

<div class="connection-profile-drawer" role="dialog" aria-label="Connection Profile Settings">
  <div class="drawer-header">
    <h3>Connection Settings</h3>
    {#if onClose}
      <button type="button" class="close-btn" onclick={onClose} aria-label="Close">✕</button>
    {/if}
  </div>

  <div class="drawer-body">
    <div class="field-group">
      <span class="group-label">External AI Agents (MCP)</span>

      <label class="checkbox-label" for="enable-external-agents">
        <input
          id="enable-external-agents"
          type="checkbox"
          checked={enableExternalAgents}
          onchange={(e) => (enableExternalAgents = e.currentTarget.checked)}
        />
        <span>Allow external AI assistants (MCP)</span>
      </label>

      {#if enableExternalAgents}
        <div class="notice-box">
          <p class="warning-text">
            Enabling external AI access sets this profile to strictly read-only with local file system access disabled.
          </p>
        </div>

        <label class="checkbox-label" for="allow-query-history">
          <input
            id="allow-query-history"
            type="checkbox"
            checked={allowQueryHistory}
            onchange={(e) => (allowQueryHistory = e.currentTarget.checked)}
          />
          <span>Allow query history search in MCP</span>
        </label>

        <div class="snippet-section">
          <button
            type="button"
            class="copy-btn"
            onclick={copyMcpSnippet}
          >
            {copied ? '✓ Copied!' : 'Copy MCP Config for Cursor / Claude Desktop'}
          </button>
        </div>
      {/if}
    </div>
  </div>

  {#if onSave}
    <div class="drawer-footer">
      <button type="button" class="save-btn" onclick={handleSave}>Save</button>
    </div>
  {/if}
</div>

<style>
  .connection-profile-drawer {
    display: flex;
    flex-direction: column;
    background: var(--bg-surface, #ffffff);
    border: 1px solid var(--border, #e2e8f0);
    border-radius: var(--radius-lg, 8px);
    padding: 1.25rem;
    gap: 1rem;
  }

  .drawer-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .drawer-header h3 {
    margin: 0;
    font-size: 1.125rem;
    font-weight: 600;
  }

  .close-btn {
    background: transparent;
    border: none;
    cursor: pointer;
    font-size: 1rem;
    padding: 0.25rem;
  }

  .field-group {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .group-label {
    font-size: 0.75rem;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted, #64748b);
  }

  .checkbox-label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.875rem;
    cursor: pointer;
  }

  .notice-box {
    background: var(--bg-warning-subtle, #fef3c7);
    border: 1px solid var(--border-warning, #f59e0b);
    border-radius: 6px;
    padding: 0.75rem;
  }

  .warning-text {
    margin: 0;
    font-size: 0.8125rem;
    color: var(--text-warning, #92400e);
    line-height: 1.4;
  }

  .snippet-section {
    margin-top: 0.25rem;
  }

  .copy-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0.5rem 1rem;
    font-size: 0.8125rem;
    font-weight: 500;
    background: var(--bg-secondary, #f1f5f9);
    border: 1px solid var(--border, #cbd5e1);
    border-radius: 6px;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .copy-btn:hover {
    background: var(--bg-hover, #e2e8f0);
  }

  .drawer-footer {
    display: flex;
    justify-content: flex-end;
  }

  .save-btn {
    padding: 0.5rem 1.25rem;
    background: var(--primary, #3b82f6);
    color: white;
    border: none;
    border-radius: 6px;
    font-weight: 500;
    cursor: pointer;
  }
</style>
