<script module lang="ts">
  /** A driver field as the rendering decisions need it. */
  interface FieldLike {
    kind: string;
  }

  /**
   * The HTML input type for a driver field kind.
   *
   * A `path` renders as text plus a Browse button rather than
   * `<input type="file">`: the browser's file input deliberately hides the
   * real filesystem path, and a path is exactly what the driver needs.
   *
   * Exported and pure so the rendering logic is testable without mounting.
   */
  export function fieldInputType(field: FieldLike): string {
    switch (field.kind) {
      case 'number':
        return 'number';
      case 'password':
        return 'password';
      default:
        return 'text';
    }
  }

  /** True when the field needs a Browse button (a filesystem path). */
  export function needsFilePicker(field: FieldLike): boolean {
    return field.kind === 'path';
  }
</script>

<script lang="ts">
  import {
    connections,
    type ConnectionProfile,
  } from '../../stores/connections.svelte';
  import { driversQuery } from '../../queries/drivers.ts';

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

  // Driver descriptors are static per build — one IPC call per app run.
  const drivers = driversQuery();

  let name = $state(profile?.name ?? '');
  let driver = $state(profile?.driver ?? 'postgres');
  let params = $state<Record<string, string>>({ ...(profile?.params ?? {}) });
  let alias = $state(profile?.alias ?? '');
  let password = $state('');
  let group = $state(profile?.group ?? '');
  let color = $state(profile?.color ?? '#3b82f6');
  let showPassword = $state(false);

  let isNew = $derived(!profile);
  let saving = $state(false);
  let testing = $state(false);

  let testResult = $state<string | null>(null);
  let testError = $state<string | null>(null);

  /** Field descriptors for the selected driver — drives the form's fields. */
  const descriptor = $derived(
    (drivers.data ?? []).find((d) => d.id === driver) ?? null,
  );

  /**
   * A driver's parameters are meaningless to another driver: switching the
   * driver must not carry stale fields (host/port/user/database) into a
   * DuckDB profile — they would be saved into the profile and sent to probes.
   * The seeding effect below re-adds the new driver's own defaults.
   */
  function resetParamsForDriver() {
    params = {};
    testResult = null;
    testError = null;
  }

  /** Seed defaults for fields this driver defines but the profile lacks. */
  $effect(() => {
    if (!descriptor) return;
    const next = { ...params };
    let changed = false;
    for (const field of descriptor.fields) {
      if (next[field.key] === undefined && field.default !== null) {
        next[field.key] = field.default;
        changed = true;
      }
    }
    if (changed) params = next;
  });

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

  // ─── Sync form fields when profile changes ───────────────────────────
  $effect(() => {
    if (profile) {
      name = profile.name ?? '';
      driver = profile.driver ?? 'postgres';
      params = { ...(profile.params ?? {}) };
      alias = profile.alias ?? '';
      group = profile.group ?? '';
      color = profile.color ?? '#3b82f6';
    }
  });

  // ─── Actions ──────────────────────────────────────────────────────────

  import { open } from '@tauri-apps/plugin-dialog';

  /**
   * Pick a database file. Cancelling leaves the current value alone — clearing
   * it would silently discard a path the user already typed.
   */
  async function browseFor(key: string) {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: 'DuckDB database', extensions: ['duckdb', 'db'] }],
      });
      if (typeof selected === 'string') {
        params = { ...params, [key]: selected };
      }
    } catch (e) {
      console.error('File picker failed:', e);
    }
  }

  /**
   * Params the selected driver's descriptor declares, preserving entered
   * values. Prunes params left over from a different driver (e.g. postgres
   * host/port in a duckdb profile) so they never get saved or probed.
   */
  function driverParams(): Record<string, string> {
    if (!descriptor) return { ...params };
    return Object.fromEntries(
      descriptor.fields
        .filter((f) => params[f.key] !== undefined)
        .map((f) => [f.key, params[f.key]]),
    );
  }

  async function handleSave() {
    saving = true;
    try {
      const p: ConnectionProfile = {
        id: profile?.id ?? crypto.randomUUID(),
        name: name || 'Untitled',
        driver,
        alias: alias.trim() || null,
        params: driverParams(),
        sshTunnelId: profile?.sshTunnelId ?? null,
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
    testing = true;
    testResult = null;
    testError = null;
    try {
      if (profile?.id) {
        const result = await connections.testConnection(profile.id);
        if (result.success) {
          testResult = result.message || 'Connection successful';
        } else {
          testError = result.message;
        }
      } else {
        const tempId = `temp-test-${crypto.randomUUID()}`;
        const tempProfile: ConnectionProfile = {
          id: tempId,
          name: name || 'Test',
          driver,
          alias: null,
          params: driverParams(),
          sshTunnelId: null,
          group: group || null,
          color: color || null,
          icon: null,
          lastUsed: null,
          createdAt: '',
          updatedAt: '',
        };
        // Use direct invoke so the connections query is NOT refetched and NO card flashes at top of UI
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke('save_connection', {
          profile: tempProfile,
          password: password || null,
        });
        try {
          const result = await invoke<{ success: boolean; message: string }>(
            'test_connection',
            { id: tempId },
          );
          if (result.success) {
            testResult = 'Connection successful';
          } else {
            testError = result.message;
          }
        } finally {
          await invoke('delete_connection', { id: tempId });
        }
      }
    } catch (e: any) {
      testError = typeof e === 'string' ? e : (e?.message ?? 'Test failed');
    } finally {
      testing = false;
    }
  }

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
  <!-- Scrollable form body -->
  <div class="form-body">
    <div class="form-columns">
      <!-- ─── Left column: credentials ─────────────────────────────── -->
      <div class="col-primary">
        <div class="field-group">
          <span class="group-label">Connection</span>

          <!-- Name + Driver on same row -->
          <div class="field-row">
            <label class="field flex-2">
              <span class="label-text">Name</span>
              <div class="input-wrapper">
                <svg
                  class="field-icon"
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                >
                  <path
                    d="M20.59 13.41l-7.17 7.17a2 2 0 0 1-2.83 0L2 12V2h10l8.59 8.59a2 2 0 0 1 0 2.82z"
                  />
                  <line x1="7" y1="7" x2="7.01" y2="7" />
                </svg>
                <input
                  type="text"
                  bind:value={name}
                  placeholder="My Database"
                  required
                />
              </div>
            </label>

            <label class="field flex-1">
              <span class="label-text">Driver</span>
              <select
                bind:value={driver}
                class="styled-select"
                onchange={resetParamsForDriver}
              >
                {#each drivers.data ?? [] as d (d.id)}
                  <option value={d.id}>{d.displayName}</option>
                {/each}
              </select>
            </label>
          </div>

          <!-- Driver-defined connection parameters -->
          {#if descriptor}
            {#if driver === 'postgres'}
              <div class="field-row">
                <label class="field flex-2" for="field-host">
                  <span class="label-text">Host</span>
                  <input
                    id="field-host"
                    class="plain-input"
                    type="text"
                    placeholder="127.0.0.1"
                    required
                    bind:value={params['host']}
                  />
                </label>
                <label class="field flex-1" for="field-port">
                  <span class="label-text">Port</span>
                  <input
                    id="field-port"
                    class="plain-input"
                    type="number"
                    placeholder="5432"
                    required
                    bind:value={params['port']}
                  />
                </label>
              </div>

              <div class="field-row">
                <label class="field flex-1" for="field-user">
                  <span class="label-text">User</span>
                  <input
                    id="field-user"
                    class="plain-input"
                    type="text"
                    placeholder="postgres"
                    required
                    bind:value={params['user']}
                  />
                </label>
                <label class="field flex-1">
                  <span class="label-text">Password</span>
                  <div class="input-wrapper password-wrapper">
                    <input
                      type={showPassword ? 'text' : 'password'}
                      class="plain-input password-input"
                      bind:value={password}
                      placeholder={isNew ? 'Password' : 'Leave blank to keep'}
                    />
                    <button
                      type="button"
                      class="eye-btn"
                      onclick={() => (showPassword = !showPassword)}
                      title={showPassword ? 'Hide password' : 'Show password'}
                    >
                      {#if showPassword}
                        <svg
                          width="14"
                          height="14"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2"
                        >
                          <path
                            d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"
                          />
                          <line x1="1" y1="1" x2="23" y2="23" />
                        </svg>
                      {:else}
                        <svg
                          width="14"
                          height="14"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2"
                        >
                          <path
                            d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"
                          />
                          <circle cx="12" cy="12" r="3" />
                        </svg>
                      {/if}
                    </button>
                  </div>
                </label>
              </div>

              <div class="field-row">
                <label class="field flex-1" for="field-database">
                  <span class="label-text">Database</span>
                  <input
                    id="field-database"
                    class="plain-input"
                    type="text"
                    placeholder="postgres"
                    required
                    bind:value={params['database']}
                  />
                </label>
                <label class="field flex-1" for="field-ssl_mode">
                  <span class="label-text">SSL Mode</span>
                  <select
                    id="field-ssl_mode"
                    class="styled-select"
                    bind:value={params['ssl_mode']}
                  >
                    <option value="disable">disable</option>
                    <option value="prefer">prefer</option>
                    <option value="require">require</option>
                  </select>
                </label>
              </div>
            {:else}
              {#each descriptor.fields as field (field.key)}
                <label class="field" for={`field-${field.key}`}>
                  <span class="label-text">{field.label}</span>
                  {#if field.kind === 'select'}
                    <select
                      id={`field-${field.key}`}
                      class="styled-select"
                      bind:value={params[field.key]}
                    >
                      {#each field.options as option (option)}
                        <option value={option}>{option}</option>
                      {/each}
                    </select>
                  {:else if needsFilePicker(field)}
                    <div class="browse-row">
                      <input
                        id={`field-${field.key}`}
                        class="plain-input"
                        type={fieldInputType(field)}
                        placeholder={field.placeholder ?? ''}
                        required={field.required}
                        bind:value={params[field.key]}
                      />
                      <button
                        type="button"
                        class="browse-btn"
                        onclick={() => browseFor(field.key)}
                      >
                        Browse…
                      </button>
                    </div>
                  {:else}
                    <input
                      id={`field-${field.key}`}
                      class="plain-input"
                      type={fieldInputType(field)}
                      placeholder={field.placeholder ?? ''}
                      required={field.required}
                      bind:value={params[field.key]}
                    />
                  {/if}
                </label>
              {/each}

              <!-- Password (keychain secret — only drivers that use one) -->
              {#if descriptor?.hasSecret}
                <label class="field">
                  <span class="label-text">Password</span>
                  <div class="input-wrapper password-wrapper">
                    <input
                      type={showPassword ? 'text' : 'password'}
                      class="plain-input password-input"
                      bind:value={password}
                      placeholder={isNew ? 'Password' : 'Leave blank to keep'}
                    />
                    <button
                      type="button"
                      class="eye-btn"
                      onclick={() => (showPassword = !showPassword)}
                      title={showPassword ? 'Hide password' : 'Show password'}
                    >
                      {#if showPassword}
                        <svg
                          width="14"
                          height="14"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2"
                        >
                          <path
                            d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"
                          />
                          <line x1="1" y1="1" x2="23" y2="23" />
                        </svg>
                      {:else}
                        <svg
                          width="14"
                          height="14"
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2"
                        >
                          <path
                            d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"
                          />
                          <circle cx="12" cy="12" r="3" />
                        </svg>
                      {/if}
                    </button>
                  </div>
                </label>
              {/if}
            {/if}
          {/if}
        </div>
      </div>

      <!-- ─── Right column: metadata ───────────────────────────────── -->
      <div class="col-secondary">
        <div class="field-group">
          <span class="group-label">Metadata</span>

          <!-- Alias -->
          <label class="field">
            <span class="label-text">Alias (@mention)</span>
            <input
              type="text"
              class="plain-input"
              bind:value={alias}
              placeholder="e.g. prod-warehouse"
            />
          </label>

          <!-- Group tag -->
          <label class="field">
            <span class="label-text">Group Tag</span>
            <input
              type="text"
              bind:value={group}
              placeholder="e.g. Production"
              class="plain-input"
            />
          </label>

          <!-- Badge Color -->
          <div class="field">
            <span class="label-text">Badge Color</span>
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
          </div>
        </div>
      </div>
    </div>
  </div>

  <!-- ─── Sticky action footer ─────────────────────────────────────── -->
  <div class="form-actions">
    <!-- Test result (if any) spans full width above buttons -->
    {#if testResult || testError}
      <div class="test-result-row">
        {#if testResult}
          <span class="test-badge test-success selectable">
            ✓ {testResult}
          </span>
        {/if}
        {#if testError}
          <span class="test-badge test-error selectable">
            ✕ {testError}
          </span>
        {/if}
      </div>
    {/if}

    <div class="action-row">
      <button
        type="button"
        class="test-btn"
        class:loading={testing}
        onclick={handleTest}
        disabled={testing}
      >
        {#if testing}
          <span class="spinner-sm"></span>
          Testing…
        {:else}
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
            <polyline points="22 4 12 14.01 9 11.01" />
          </svg>
          Test Connection
        {/if}
      </button>

      <div class="action-spacer"></div>

      {#if onCancel}
        <button type="button" class="cancel-btn" onclick={() => onCancel?.()}>
          Cancel
        </button>
      {/if}
      <button type="submit" class="save-btn" disabled={saving}>
        <span
          >{saving
            ? 'Saving…'
            : isNew
              ? 'Connect & Save'
              : 'Save Connection'}</span
        >
        <span class="btn-shortcut">⌘↵</span>
      </button>
    </div>
  </div>
</form>

<style>
  /* ─── Root form layout ─────────────────────────────────────────────── */
  .connection-form {
    display: flex;
    flex-direction: column;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card, 0 2px 8px rgba(0, 0, 0, 0.06));
    overflow: visible;
  }

  /* Form body in natural document flow */
  .form-body {
    padding: 16px 18px 12px;
    overflow: visible;
  }

  /* ─── Two-column grid ──────────────────────────────────────────────── */
  .form-columns {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 18px;
  }

  .col-primary,
  .col-secondary {
    display: flex;
    flex-direction: column;
  }

  /* ─── Field groups ─────────────────────────────────────────────────── */
  .field-group {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .group-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
    margin-bottom: 2px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .field-row {
    display: flex;
    gap: 10px;
    align-items: flex-start;
  }
  .flex-1 {
    flex: 1;
  }
  .flex-2 {
    flex: 2;
  }

  .label-text {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  /* ─── Input controls (consistent 34px height) ──────────────────────── */
  .input-wrapper {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
  }
  .field-icon {
    position: absolute;
    left: 10px;
    color: var(--text-muted);
    pointer-events: none;
    flex-shrink: 0;
  }

  .input-wrapper input,
  .plain-input,
  .styled-select {
    width: 100%;
    height: 34px;
    padding: 0 10px 0 32px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-input);
    color: var(--text);
    font-size: 13px;
    outline: none;
    transition:
      border-color 0.15s ease,
      box-shadow 0.15s ease;
    box-sizing: border-box;
    display: flex;
    align-items: center;
  }
  .plain-input {
    padding: 0 10px;
  }
  .password-input {
    padding: 0 32px 0 10px;
  }
  .styled-select {
    padding: 0 10px;
    cursor: pointer;
  }

  .browse-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .browse-row .plain-input {
    flex: 1;
    min-width: 0;
  }
  .browse-btn {
    height: 34px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-size: 13px;
    font-weight: 500;
    white-space: nowrap;
    box-sizing: border-box;
    cursor: pointer;
    transition:
      background 0.12s,
      color 0.12s,
      border-color 0.12s;
  }
  .browse-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
    border-color: var(--border-hover, var(--border));
  }

  .input-wrapper input:focus,
  .plain-input:focus,
  .styled-select:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 15%, transparent);
  }

  .eye-btn {
    position: absolute;
    right: 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    border-radius: var(--radius-sm);
    padding: 0;
    transition: color 0.12s;
  }
  .eye-btn:hover {
    color: var(--text);
  }

  /* ─── Color picker ─────────────────────────────────────────────────── */
  .color-picker {
    display: flex;
    gap: 5px;
    flex-wrap: wrap;
    align-items: center;
    padding: 2px 0;
  }
  .color-swatch {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    transition:
      transform 0.12s ease,
      border-color 0.12s ease;
    padding: 0;
  }
  .color-swatch:hover {
    transform: scale(1.15);
  }
  .color-swatch.selected {
    border-color: var(--text);
    transform: scale(1.15);
    box-shadow: 0 0 0 2px var(--bg-surface);
  }

  /* ─── Sticky action footer ─────────────────────────────────────────── */
  .form-actions {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 18px 14px;
    border-top: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg-surface) 95%, var(--bg-elevated));
    flex-shrink: 0;
  }

  .test-result-row {
    min-width: 0;
  }

  .action-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .action-spacer {
    flex: 1;
  }

  /* ─── Buttons (consistent 34px height) ─────────────────────────────── */
  .test-btn,
  .cancel-btn,
  .save-btn,
  .test-badge {
    height: 34px;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-md);
    font-size: 13px;
    font-weight: 500;
  }

  .test-btn {
    flex-shrink: 0;
    white-space: nowrap;
    gap: 6px;
    padding: 0 12px;
    border: 1px solid var(--border);
    background: var(--bg-surface);
    color: var(--text-secondary);
    cursor: pointer;
    transition:
      background 0.12s,
      color 0.12s,
      border-color 0.12s;
  }
  .test-btn:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text);
    border-color: var(--border-hover, var(--border));
  }
  .test-btn:disabled {
    opacity: 0.6;
  }

  .spinner-sm {
    width: 12px;
    height: 12px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .test-badge {
    min-width: 0;
    min-height: 34px;
    height: auto;
    padding: 6px 12px;
    justify-content: flex-start;
    text-align: left;
    line-height: 1.35;
    overflow-wrap: anywhere;
    width: 100%;
  }
  .test-success {
    color: var(--success, #22c55e);
    background: color-mix(in srgb, var(--success, #22c55e) 12%, transparent);
  }
  .test-error {
    color: var(--error, #ef4444);
    background: color-mix(in srgb, var(--error, #ef4444) 12%, transparent);
  }

  .cancel-btn {
    flex-shrink: 0;
    white-space: nowrap;
    padding: 0 14px;
    border: 1px solid var(--border);
    background: var(--bg-surface);
    color: var(--text-secondary);
    cursor: pointer;
  }
  .cancel-btn:hover {
    background: var(--bg-hover);
    color: var(--text);
  }

  .save-btn {
    flex-shrink: 0;
    white-space: nowrap;
    gap: 8px;
    padding: 0 16px;
    border: none;
    background: var(--accent);
    color: #fff;
    font-weight: 600;
    cursor: pointer;
    transition:
      background 0.15s ease,
      transform 0.1s ease;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
  }
  .save-btn:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .save-btn:disabled {
    opacity: 0.6;
  }

  .btn-shortcut {
    font-size: 11px;
    font-family:
      -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: rgba(255, 255, 255, 0.2);
    color: #fff;
    padding: 2px 6px;
    border-radius: 4px;
    line-height: 1;
    font-weight: 500;
    letter-spacing: 0.02em;
  }

  /* ─── Responsive: single column on narrow widths ───────────────────── */
  @media (max-width: 560px) {
    .form-columns {
      grid-template-columns: 1fr;
      gap: 14px;
    }
    .field-row {
      flex-direction: column;
    }
    .action-row {
      flex-wrap: wrap;
    }
  }
</style>
