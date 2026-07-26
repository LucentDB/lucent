<script lang="ts">
  import {
    connections,
    type ConnectionProfile,
  } from '../../stores/connections.svelte';
  import ConnectionList from './ConnectionList.svelte';
  import ConnectionForm from './ConnectionForm.svelte';
  import { addRecentConnection } from '../../stores/recent.js';

  let {
    onConnect,
    connectError = null,
  }: {
    onConnect?: (config: any) => void;
    connectError?: string | null;
  } = $props();

  // ─── View state ───────────────────────────────────────────────────────

  type ViewState =
    | { mode: 'list' }
    | { mode: 'form-new' }
    | { mode: 'form-edit'; profile: ConnectionProfile }
    | { mode: 'detail'; profile: ConnectionProfile };

  let view = $state<ViewState>({ mode: 'list' });
  let selectedId = $state<string | null>(null);

  // ─── Actions ──────────────────────────────────────────────────────────

  function handleSelect(id: string) {
    selectedId = id;
    // Connect via the connections store (this calls invoke('connect', { connectionId, config: null }))
    connections
      .connectToProfile(id)
      .then(() => {
        // On success, notify parent with profile details so App.svelte can
        // update UI state without reconnecting (which would need a password).
        const profile = connections.profiles.find((p) => p.id === id);
        if (profile) {
          onConnect?.({
            connectionId: id,
            host: profile.host,
            port: profile.port,
            user: profile.user,
            database: profile.database,
          });
        } else {
          onConnect?.({
            connectionId: id,
            host: '',
            port: 0,
            user: '',
            database: '',
          });
        }
      })
      .catch(() => {
        // Error is already set in connections store
      });
  }

  function handleNewConnection() {
    view = { mode: 'form-new' };
  }

  function handleEditProfile(profile: ConnectionProfile) {
    view = { mode: 'form-edit', profile };
  }

  function handleSaveProfile(profile: ConnectionProfile, password?: string) {
    connections
      .saveProfile(profile, password)
      .then(() => {
        view = { mode: 'list' };
      })
      .catch((e) => {
        console.error('Failed to save profile:', e);
      });
  }

  function handleDeleteProfile(id: string) {
    if (confirm('Delete this connection profile?')) {
      connections.deleteProfile(id);
    }
  }

  function handleDuplicateProfile(id: string) {
    connections.duplicateProfile(id);
  }

  function handleTestProfile(id: string) {
    connections.testConnection(id).then((result) => {
      if (!result.success) {
        alert(result.message);
      }
    });
  }

  function handleCancelForm() {
    view = { mode: 'list' };
  }

  function handleInlineConnect(config: {
    host: string;
    port: number;
    user: string;
    password: string;
    database: string;
  }) {
    connections
      .connectInline(config)
      .then(() => {
        addRecentConnection(config);
        onConnect?.(config);
      })
      .catch(() => {});
  }
</script>

<div class="connection-manager">
  <!-- Header -->
  <div class="manager-header">
    <div class="brand">
      <span class="logo">⌬</span>
      <h1 class="title">Lucent</h1>
    </div>
    <p class="tagline">Connect to a PostgreSQL database to get started</p>
  </div>

  {#if view.mode === 'list'}
    <!-- Connection List -->
    <div class="manager-body">
      <ConnectionList
        profiles={connections.profiles}
        groupedProfiles={connections.groupedProfiles}
        loading={connections.loading}
        activeProfileId={connections.activeProfileId}
        testingIds={connections.testingIds}
        onSelect={handleSelect}
        onTest={handleTestProfile}
        onDelete={handleDeleteProfile}
        onDuplicate={handleDuplicateProfile}
        onNewConnection={handleNewConnection}
      />

      <!-- Error banner -->
      {#if connectError || connections.errorMessage}
        <div class="error-banner">
          {connections.errorMessage ?? connectError}
          <button
            class="dismiss-btn"
            onclick={() => (connections.errorMessage = null)}>✕</button
          >
        </div>
      {/if}

      <!-- Inline connection form for quick connect -->
      {#if !connections.activeProfileId}
        <div class="inline-section">
          <div class="inline-header">
            <h3>Quick Connect</h3>
          </div>
          <ConnectionForm
            onSave={(p, _pw) => {
              // Save and then connect
              handleSaveProfile(p, _pw);
              // If password is provided, connect too
              if (_pw) {
                connections.connectInline({
                  host: p.host,
                  port: p.port,
                  user: p.user,
                  password: _pw,
                  database: p.database,
                });
              }
            }}
          />
        </div>
      {/if}
    </div>
  {:else if view.mode === 'form-new'}
    <!-- New Connection Form -->
    <div class="manager-body form-panel">
      <div class="panel-header">
        <button class="back-btn" onclick={handleCancelForm}>
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <line x1="19" y1="12" x2="5" y2="12" />
            <polyline points="12 19 5 12 12 5" />
          </svg>
          Back
        </button>
        <h2>New Connection</h2>
      </div>
      <ConnectionForm onSave={handleSaveProfile} onCancel={handleCancelForm} />
    </div>
  {:else if view.mode === 'form-edit'}
    <!-- Edit Connection Form -->
    <div class="manager-body form-panel">
      <div class="panel-header">
        <button class="back-btn" onclick={handleCancelForm}>
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <line x1="19" y1="12" x2="5" y2="12" />
            <polyline points="12 19 5 12 12 5" />
          </svg>
          Back
        </button>
        <h2>Edit Connection</h2>
      </div>
      <ConnectionForm
        profile={view.profile}
        onSave={handleSaveProfile}
        onCancel={handleCancelForm}
      />
    </div>
  {/if}

  <!-- Keyboard shortcut hint -->
  <div class="shortcut-hint">
    {#if view.mode === 'list'}
      Press <kbd>/</kbd> to search, <kbd>↑</kbd><kbd>↓</kbd> to navigate,
      <kbd>Enter</kbd> to connect
    {/if}
  </div>
</div>

<style>
  .connection-manager {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    overflow: hidden;
    max-width: 640px;
    margin: 0 auto;
    width: 100%;
  }
  .manager-header {
    text-align: center;
    padding: 24px 24px 16px;
    flex-shrink: 0;
  }
  .brand {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    margin-bottom: 4px;
  }
  .logo {
    font-size: 28px;
    color: var(--accent);
  }
  .title {
    font-size: 22px;
    font-weight: 700;
    color: var(--text);
    letter-spacing: -0.03em;
    margin: 0;
  }
  .tagline {
    font-size: 13px;
    color: var(--text-muted);
    margin: 0;
  }
  .manager-body {
    flex: 1;
    overflow-y: auto;
    padding: 0 16px 16px;
  }
  .manager-body.form-panel {
    padding-top: 4px;
  }
  .panel-header {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 16px;
  }
  .panel-header h2 {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
    margin: 0;
  }
  .back-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-size: 13px;
    cursor: pointer;
  }
  .back-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .error-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 16px;
    margin: 12px 0;
    background: color-mix(in srgb, var(--error) 10%, transparent);
    color: var(--error);
    border-radius: var(--radius-md);
    font-size: 13px;
    border: 1px solid color-mix(in srgb, var(--error) 30%, transparent);
  }
  .dismiss-btn {
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--error);
    cursor: pointer;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .dismiss-btn:hover {
    background: color-mix(in srgb, var(--error) 20%, transparent);
  }
  .inline-section {
    margin-top: 20px;
    padding: 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
  }
  .inline-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
  }
  .inline-header h3 {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
    margin: 0;
  }
  .shortcut-hint {
    text-align: center;
    font-size: 11px;
    color: var(--text-muted);
    padding: 8px 16px;
    flex-shrink: 0;
  }
  .shortcut-hint kbd {
    font-size: 10px;
    background: var(--bg-hover);
    padding: 1px 5px;
    border-radius: 3px;
    border: 1px solid var(--border);
    font-family: var(--font-mono);
  }
</style>
