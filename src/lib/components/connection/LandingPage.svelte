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
    connections
      .connectToProfile(id)
      .then(() => {
        const profile = connections.profiles.find((p) => p.id === id);
        if (profile) {
          onConnect?.({
            connectionId: id,
            host: profile.params['host'] ?? '',
            port: Number(profile.params['port']) || 0,
            user: profile.params['user'] ?? '',
            database: profile.params['database'] ?? '',
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
      .catch(() => {});
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
</script>

<div class="connection-manager">
  <!-- Header -->
  <div class="manager-header">
    <div class="brand">
      <div class="logo-container">
        <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="url(#logo-gradient)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" class="logo-svg">
          <defs>
            <linearGradient id="logo-gradient" x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stop-color="var(--accent)" />
              <stop offset="100%" stop-color="#a78bfa" />
            </linearGradient>
          </defs>
          <ellipse cx="12" cy="5" rx="9" ry="3" />
          <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" />
          <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
        </svg>
      </div>
      <h1 class="title">Lucent</h1>
    </div>
    <p class="tagline">
      Select a saved database or enter connection parameters to get started
    </p>
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
        onEdit={handleEditProfile}
        onDelete={handleDeleteProfile}
        onDuplicate={handleDuplicateProfile}
        onNewConnection={handleNewConnection}
      />

      <!-- Error banner -->
      {#if connectError || connections.errorMessage}
        <div class="error-banner">
          <span>{connections.errorMessage ?? connectError}</span>
          <button
            class="dismiss-btn"
            onclick={() => (connections.errorMessage = null)}>✕</button
          >
        </div>
      {/if}

      <!-- Inline connection form for quick connect -->
      {#if !connections.activeProfileId}
        <div class="quick-connect-container">
          <div class="quick-connect-header">
            <h3>Quick Connect</h3>
          </div>
          <ConnectionForm
            onSave={(p, _pw) => {
              handleSaveProfile(p, _pw);
              if (_pw) {
                connections.connectInline({
                  driver: p.driver,
                  params: { ...p.params },
                  secret: _pw,
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
        <h2>New Connection Profile</h2>
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
        <h2>Edit Connection Profile</h2>
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
    max-width: 780px;
    margin: 0 auto;
    width: 100%;
    height: 100%;
    box-sizing: border-box;
    position: relative;
  }
  .manager-header {
    text-align: center;
    padding: 44px 24px 24px;
    flex-shrink: 0;
    position: relative;
  }
  .brand {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 14px;
    margin-bottom: 10px;
  }
  .logo-container {
    width: 48px;
    height: 48px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--accent-soft);
    border-radius: var(--radius-lg);
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--accent) 15%, transparent),
      0 4px 20px color-mix(in srgb, var(--accent) 20%, transparent);
    flex-shrink: 0;
  }
  .logo-svg {
    filter: drop-shadow(0 2px 4px color-mix(in srgb, var(--accent) 30%, transparent));
  }
  .title {
    font-size: 30px;
    font-weight: 750;
    background: linear-gradient(135deg, var(--text) 40%, var(--accent));
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
    letter-spacing: -0.04em;
    margin: 0;
    line-height: 1.1;
  }
  .tagline {
    font-size: 14px;
    color: var(--text-muted);
    margin: 0;
    font-weight: 400;
    line-height: 1.5;
  }
  .manager-body {
    flex: 1;
    overflow-y: auto;
    padding: 0 24px 32px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .manager-body.form-panel {
    padding-top: 4px;
  }
  .panel-header {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-bottom: 20px;
  }
  .panel-header h2 {
    font-size: 17px;
    font-weight: 600;
    color: var(--text);
    margin: 0;
    letter-spacing: -0.02em;
  }
  .back-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition:
      all var(--transition-normal);
    box-shadow: var(--shadow-sm);
  }
  .back-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
    transform: translateX(-2px);
    border-color: color-mix(in srgb, var(--text) 30%, transparent);
  }
  .error-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 16px;
    margin: 12px 0;
    background: color-mix(in srgb, var(--error) 8%, transparent);
    color: var(--error);
    border-radius: var(--radius-md);
    font-size: 13px;
    border: 1px solid color-mix(in srgb, var(--error) 25%, transparent);
    animation: slide-in 0.2s ease-out;
  }
  @keyframes slide-in {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }
  .dismiss-btn {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--error);
    cursor: pointer;
    border-radius: 50%;
    flex-shrink: 0;
    font-size: 14px;
    transition: all var(--transition-fast);
  }
  .dismiss-btn:hover {
    background: color-mix(in srgb, var(--error) 15%, transparent);
    transform: scale(1.1);
  }
  .quick-connect-container {
    margin-top: 28px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .quick-connect-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 4px;
  }
  .quick-connect-header h3 {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
    margin: 0;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .shortcut-hint {
    text-align: center;
    font-size: 12px;
    color: var(--text-muted);
    padding: 14px 24px 18px;
    flex-shrink: 0;
    border-top: 1px solid var(--border-light);
    background: color-mix(in srgb, var(--bg-subtle) 50%, transparent);
  }
  .shortcut-hint kbd {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 20px;
    height: 20px;
    font-size: 11px;
    font-weight: 600;
    background: var(--bg-surface);
    color: var(--text-secondary);
    padding: 0 6px;
    border-radius: 5px;
    border: 1px solid var(--border);
    font-family: var(--font-mono);
    box-shadow: 0 1px 0 var(--border), 0 1px 2px rgba(0,0,0,0.05);
    margin: 0 1px;
  }
</style>

