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
  const descriptor = $derived(connections.driverFor(driver));

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
        // Use direct invoke so connections.profiles store is NOT mutated and NO card flashes at top of UI
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
  <div class="form-card">
    <div class="card-section">
      <h3 class="section-title">Database Credentials</h3>

      <!-- Name -->
      <label class="field">
        <span class="label-text">Connection Name</span>
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

      <!-- Driver -->
      <label class="field">
        <span class="label-text">Driver</span>
        <select
          bind:value={driver}
          class="styled-select"
          onchange={resetParamsForDriver}
        >
          {#each connections.drivers as d (d.id)}
            <option value={d.id}>{d.displayName}</option>
          {/each}
        </select>
      </label>

      <!-- Driver-defined connection parameters -->
      {#if descriptor}
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
      {/if}

      <!-- Password (keychain secret — only drivers that use one) -->
      {#if descriptor?.hasSecret}
        <label class="field">
          <span class="label-text">Password</span>
          <div class="input-wrapper">
            <input
              type={showPassword ? 'text' : 'password'}
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
                  <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
                  <circle cx="12" cy="12" r="3" />
                </svg>
              {/if}
            </button>
          </div>
        </label>
      {/if}

      <!-- Alias — the @mention handle the AI uses to address this connection -->
      <label class="field">
        <span class="label-text">Alias (@mention)</span>
        <input
          type="text"
          class="plain-input"
          bind:value={alias}
          placeholder="e.g. prod-warehouse"
        />
      </label>
    </div>

    <div class="card-section border-top">
      <h3 class="section-title">Environment & Tag</h3>
      <div class="field-row align-center">
        <label class="field flex-1">
          <span class="label-text">Group Tag</span>
          <input
            type="text"
            bind:value={group}
            placeholder="e.g. Production, Development"
            class="plain-input"
          />
        </label>
        <label class="field flex-1">
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
        </label>
      </div>
    </div>
  </div>

  <!-- Actions -->
  <div class="form-actions">
    <div class="test-area">
      <button
        type="button"
        class="test-btn"
        class:loading={testing}
        onclick={handleTest}
        disabled={testing}
      >
        {#if testing}
          <span class="spinner-sm"></span>
          Testing...
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
      {#if testResult}
        <span class="test-badge test-success">
          ✓ {testResult}
        </span>
      {/if}
      {#if testError}
        <span class="test-badge test-error">
          ✕ {testError}
        </span>
      {/if}
    </div>

    <div class="save-area">
      {#if onCancel}
        <button type="button" class="cancel-btn" onclick={() => onCancel?.()}>
          Cancel
        </button>
      {/if}
      <button type="submit" class="save-btn" disabled={saving}>
        <span
          >{saving
            ? 'Saving...'
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
  .connection-form {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .form-card {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card, 0 2px 8px rgba(0, 0, 0, 0.06));
    overflow: hidden;
  }
  .card-section {
    padding: 18px 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .card-section.border-top {
    border-top: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg-surface) 95%, var(--bg-elevated));
  }
  .section-title {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
    margin: 0 0 2px 0;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .field-row {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }
  .field-row.align-center {
    align-items: center;
  }
  .flex-1 {
    flex: 1;
  }
  .label-text {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
  }
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

  /* Consistent 36px control height across all inputs & dropdowns */
  .input-wrapper input,
  .plain-input,
  .styled-select {
    width: 100%;
    height: 36px;
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
    height: 36px;
    padding: 0 14px;
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
  .styled-select {
    padding: 0 10px;
    cursor: pointer;
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
  .color-picker {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: center;
    height: 36px;
    padding: 0;
  }
  .color-swatch {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    transition:
      transform 0.12s ease,
      border-color 0.12s ease;
    padding: 0;
  }
  .color-swatch:hover {
    transform: scale(1.2);
  }
  .color-swatch.selected {
    border-color: var(--text);
    transform: scale(1.2);
    box-shadow: 0 0 0 2px var(--bg-surface);
  }

  .form-actions {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: start;
    gap: 12px;
    padding-top: 4px;
  }
  .test-area {
    min-width: 0;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: start;
    gap: 10px;
  }

  /* Consistent 36px height across all buttons & status badges */
  .test-btn,
  .cancel-btn,
  .save-btn,
  .test-badge {
    height: 36px;
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
    justify-self: start;
    white-space: nowrap;
    gap: 6px;
    padding: 0 14px;
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
    min-height: 36px;
    height: auto;
    padding: 8px 12px;
    justify-content: flex-start;
    text-align: left;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }
  .test-success {
    color: var(--success, #22c55e);
    background: color-mix(in srgb, var(--success, #22c55e) 12%, transparent);
  }
  .test-error {
    color: var(--error, #ef4444);
    background: color-mix(in srgb, var(--error, #ef4444) 12%, transparent);
  }
  .save-area {
    display: flex;
    flex-shrink: 0;
    gap: 10px;
    align-items: center;
  }
  .cancel-btn {
    flex-shrink: 0;
    white-space: nowrap;
    padding: 0 16px;
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
    padding: 0 18px;
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

  @media (max-width: 600px) {
    .form-actions {
      grid-template-columns: 1fr;
    }
    .test-area {
      grid-template-columns: 1fr;
    }
    .test-badge {
      width: 100%;
    }
    .save-area {
      width: 100%;
      justify-content: flex-end;
    }
  }

  /* Clean readymade text badge for keyboard shortcut */
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
</style>
