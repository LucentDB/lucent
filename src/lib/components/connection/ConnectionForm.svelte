<script lang="ts">
  import {
    connections,
    type ConnectionProfile,
  } from '../../stores/connections.svelte';

  let {
    profile = null,
    onSave,
    onCancel,
  }: {
    profile?: ConnectionProfile | null;
    onSave?: (profile: ConnectionProfile, password?: string) => void;
    onCancel?: () => void;
  } = $props();

  // ─── Form state ───────────────────────────────────────────────────────

  let name = $state(profile?.name ?? '');
  let host = $state(profile?.host ?? '127.0.0.1');
  let port = $state(String(profile?.port ?? 5432));
  let user = $state(profile?.user ?? 'postgres');
  let password = $state('');
  let database = $state(profile?.database ?? 'postgres');
  let sslMode = $state(profile?.sslMode ?? 'prefer');
  let group = $state(profile?.group ?? '');
  let color = $state(profile?.color ?? '#3b82f6');

  let isNew = $derived(!profile);
  let saving = $state(false);

  // ─── SSH tunnel fields ───────────────────────────────────────────────

  // SSH tunneling is configured here but never wired into connect() on the
  // backend, so the UI is hidden to avoid asserting an unhonored capability.
  // Re-enable once the tunnel is actually implemented.
  // See docs/superpowers/specs/2026-07-19-lucent-trust-quality-pass-design.md
  const SSH_TUNNEL_ENABLED = false;

  let useSsh = $state(!!profile?.sshTunnelId);
  let sshHost = $state('');
  let sshPort = $state('22');
  let sshUser = $state('');
  let sshAuthMethod = $state<'password' | 'key'>('password');
  let sshKeyPath = $state('');
  let sshPassword = $state('');

  // ─── Sync form fields when profile changes ───────────────────────────
  $effect(() => {
    if (profile) {
      name = profile.name ?? '';
      host = profile.host ?? '127.0.0.1';
      port = String(profile.port ?? 5432);
      user = profile.user ?? 'postgres';
      database = profile.database ?? 'postgres';
      sslMode = profile.sslMode ?? 'prefer';
      group = profile.group ?? '';
      color = profile.color ?? '#3b82f6';
      useSsh = !!profile.sshTunnelId;
      // SSH config will be loaded separately when editing
    }
  });
  let testResult = $state<string | null>(null);
  let testError = $state<string | null>(null);

  // ─── Color palette ────────────────────────────────────────────────────

  const colorPalette = [
    '#3b82f6',
    '#6366f1',
    '#8b5cf6',
    '#a855f7',
    '#ec4899',
    '#ef4444',
    '#f97316',
    '#eab308',
    '#22c55e',
    '#14b8a6',
    '#06b6d4',
    '#64748b',
  ];

  // ─── Derived ─────────────────────────────────────────────────────────

  let sshLabel = $derived(`Tunnel to ${host}`);
  let sshConfigId = $derived(
    profile?.sshTunnelId ?? (useSsh ? `ssh-${profile?.id ?? 'new'}` : null),
  );

  // ─── Actions ──────────────────────────────────────────────────────────

  async function handleSave() {
    saving = true;
    try {
      // Handle SSH config save first. Preserve any stored sshTunnelId on a
      // round-trip even while the UI is disabled, so existing profiles aren't
      // silently stripped of their tunnel reference.
      let sshTunnelId = profile?.sshTunnelId ?? null;
      if (useSsh && SSH_TUNNEL_ENABLED) {
        const sshId =
          sshTunnelId ?? `ssh-${profile?.id ?? crypto.randomUUID()}`;
        const sshConfig = {
          id: sshId,
          label: sshLabel,
          host: sshHost || host,
          port: parseInt(sshPort, 10) || 22,
          user: sshUser || user,
          authMethod:
            sshAuthMethod === 'key'
              ? { method: 'key', keyPath: sshKeyPath }
              : { method: 'password' },
        };
        // Save SSH config via IPC (best-effort)
        try {
          const { saveSshConfig } = await import('../../ipc/client.js');
          await saveSshConfig(sshConfig, sshPassword || null);
        } catch (e) {
          console.warn('Failed to save SSH config:', e);
        }
        sshTunnelId = sshId;
      }

      const p: ConnectionProfile = {
        id: profile?.id ?? crypto.randomUUID(),
        name: name || 'Untitled',
        driver: 'postgres',
        host,
        port: parseInt(port, 10) || 5432,
        user,
        database,
        sslMode: sslMode as ConnectionProfile['sslMode'],
        sshTunnelId,
        group: group || null,
        color: color || null,
        icon: profile?.icon ?? null,
        lastUsed: profile?.lastUsed ?? null,
        createdAt: profile?.createdAt ?? '',
        updatedAt: profile?.updatedAt ?? '',
      };
      onSave?.(p, password || undefined);
    } finally {
      saving = false;
    }
  }

  async function handleTest() {
    if (!profile?.id) return;
    testResult = null;
    testError = null;
    try {
      const result = await connections.testConnection(profile.id);
      if (result.success) {
        testResult = result.message;
      } else {
        testError = result.message;
      }
    } catch (e: any) {
      testError = typeof e === 'string' ? e : (e?.message ?? 'Test failed');
    }
  }

  // Reset test state when profile changes
  $effect(() => {
    if (profile?.id) {
      testResult = null;
      testError = null;
    }
  });
</script>

<form
  class="connection-form"
  onsubmit={(e) => {
    e.preventDefault();
    handleSave();
  }}
>
  <!-- Name -->
  <label class="field">
    <span class="label-text">Name</span>
    <input type="text" bind:value={name} placeholder="My Database" required />
  </label>

  <div class="field-row">
    <label class="field flex-1">
      <span class="label-text">Host</span>
      <input type="text" bind:value={host} placeholder="localhost" />
    </label>
    <label class="field port-field">
      <span class="label-text">Port</span>
      <input
        type="number"
        bind:value={port}
        placeholder="5432"
        min="1"
        max="65535"
      />
    </label>
  </div>

  <div class="field-row">
    <label class="field flex-1">
      <span class="label-text">User</span>
      <input type="text" bind:value={user} placeholder="postgres" />
    </label>
    <label class="field flex-1">
      <span class="label-text">Password</span>
      <input
        type="password"
        bind:value={password}
        placeholder={isNew ? '' : 'Leave blank to keep current'}
      />
    </label>
  </div>

  <div class="field-row">
    <label class="field flex-2">
      <span class="label-text">Database</span>
      <input type="text" bind:value={database} placeholder="postgres" />
    </label>
    <label class="field">
      <span class="label-text">SSL Mode</span>
      <select bind:value={sslMode}>
        <option value="disable">Disable</option>
        <option value="prefer">Prefer</option>
        <option value="require">Require</option>
      </select>
    </label>
  </div>

  <!-- Group -->
  <label class="field">
    <span class="label-text">Group</span>
    <input
      type="text"
      bind:value={group}
      placeholder="e.g. Production, Development"
    />
  </label>

  <!-- Color -->
  <label class="field">
    <span class="label-text">Color</span>
    <div class="color-picker">
      {#each colorPalette as c}
        <button
          type="button"
          class="color-swatch"
          class:selected={color === c}
          style="background: {c}"
          onclick={() => (color = c)}
          title={c}
        ></button>
      {/each}
    </div>
  </label>

  <!-- SSH Tunnel — hidden until the backend actually wires the tunnel. -->
  {#if SSH_TUNNEL_ENABLED}
    <div class="ssh-section">
      <label class="toggle-field">
        <input type="checkbox" bind:checked={useSsh} />
        <span class="toggle-label">Use SSH Tunnel</span>
      </label>

      {#if useSsh}
        <div class="ssh-fields">
          <div class="field-row">
            <label class="field flex-1">
              <span class="label-text">SSH Host</span>
              <input type="text" bind:value={sshHost} placeholder={host} />
            </label>
            <label class="field port-field">
              <span class="label-text">SSH Port</span>
              <input
                type="number"
                bind:value={sshPort}
                placeholder="22"
                min="1"
                max="65535"
              />
            </label>
          </div>
          <div class="field-row">
            <label class="field flex-1">
              <span class="label-text">SSH User</span>
              <input type="text" bind:value={sshUser} placeholder={user} />
            </label>
            <label class="field flex-1">
              <span class="label-text">Auth Method</span>
              <select bind:value={sshAuthMethod}>
                <option value="password">Password</option>
                <option value="key">Key File</option>
              </select>
            </label>
          </div>
          {#if sshAuthMethod === 'password'}
            <label class="field">
              <span class="label-text">SSH Password</span>
              <input
                type="password"
                bind:value={sshPassword}
                placeholder="SSH password"
              />
            </label>
          {:else}
            <label class="field">
              <span class="label-text">Key File Path</span>
              <input
                type="text"
                bind:value={sshKeyPath}
                placeholder="/home/user/.ssh/id_rsa"
              />
            </label>
            <label class="field">
              <span class="label-text">Passphrase (optional)</span>
              <input
                type="password"
                bind:value={sshPassword}
                placeholder="Key passphrase"
              />
            </label>
          {/if}
        </div>
      {/if}
    </div>
  {/if}

  <!-- Actions -->
  <div class="form-actions">
    <div class="test-area">
      {#if profile?.id}
        <button type="button" class="test-btn" onclick={handleTest}>
          Test Connection
        </button>
      {/if}
      {#if testResult}
        <span class="test-success">{testResult}</span>
      {/if}
      {#if testError}
        <span class="test-error">{testError}</span>
      {/if}
    </div>
    <div class="save-area">
      <button type="button" class="cancel-btn" onclick={() => onCancel?.()}
        >Cancel</button
      >
      <button type="submit" class="save-btn" disabled={saving}>
        {saving ? 'Saving...' : isNew ? 'Create' : 'Save'}
      </button>
    </div>
  </div>
</form>

<style>
  .connection-form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field-row {
    display: flex;
    gap: 12px;
  }
  .flex-1 {
    flex: 1;
  }
  .flex-2 {
    flex: 2;
  }
  .port-field {
    width: 100px;
  }
  .label-text {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
  }
  .field input,
  .field select {
    width: 100%;
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    color: var(--text);
    font-size: 13px;
    outline: none;
    transition: border-color 0.12s;
    box-sizing: border-box;
  }
  .field input:focus,
  .field select:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .field select {
    cursor: pointer;
  }
  .color-picker {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    padding: 4px 0;
  }
  .color-swatch {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    transition:
      transform 0.1s,
      border-color 0.1s;
    padding: 0;
  }
  .color-swatch:hover {
    transform: scale(1.15);
  }
  .color-swatch.selected {
    border-color: var(--text);
    transform: scale(1.15);
    box-shadow: 0 0 0 2px white;
  }

  /* ── SSH Tunnel ───────────────────────────── */
  .ssh-section {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 12px;
    background: var(--bg-surface);
  }
  .toggle-field {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  .toggle-field input[type='checkbox'] {
    accent-color: var(--accent);
  }
  .toggle-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .ssh-fields {
    margin-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }

  .form-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
  }
  .test-area {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .test-btn {
    padding: 6px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    color: var(--text);
    font-size: 13px;
    cursor: pointer;
  }
  .test-btn:hover {
    background: var(--bg-hover);
  }
  .test-success {
    font-size: 12px;
    color: var(--success, #22c55e);
  }
  .test-error {
    font-size: 12px;
    color: var(--error, #ef4444);
  }
  .save-area {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .cancel-btn {
    padding: 7px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    color: var(--text);
    font-size: 13px;
    cursor: pointer;
  }
  .cancel-btn:hover {
    background: var(--bg-hover);
  }
  .save-btn {
    padding: 7px 20px;
    border: none;
    border-radius: var(--radius-md);
    background: var(--accent);
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.12s;
  }
  .save-btn:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .save-btn:disabled {
    opacity: 0.6;
  }
</style>
