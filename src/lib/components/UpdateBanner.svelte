<script lang="ts">
  import { check } from '@tauri-apps/plugin-updater';
  import { relaunch } from '@tauri-apps/plugin-process';

  type Available = { version: string; downloadAndInstall: () => Promise<void> };

  let available: Available | null = $state(null);
  let installing = $state(false);

  // Silent on failure by design: the app is fully usable offline, and an
  // update check is never worth interrupting the user with an error.
  $effect(() => {
    check()
      .then((u) => {
        if (u) available = u as Available;
      })
      .catch(() => {});
  });

  async function install() {
    if (!available) return;
    installing = true;
    try {
      await available.downloadAndInstall();
      await relaunch();
    } catch {
      installing = false;
    }
  }
</script>

{#if available}
  <div class="update" role="status">
    <span class="version-tag">Update {available.version}</span>
    <button
      type="button"
      onclick={install}
      disabled={installing}
      aria-busy={installing}
    >
      {installing ? 'Installing…' : 'Install'}
    </button>
  </div>
{/if}

<style>
  .update {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    padding: 3px 8px;
    border-radius: var(--radius-sm);
    background: var(--accent-soft, rgba(127, 127, 255, 0.12));
    border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
    color: var(--text);
  }
  .version-tag {
    font-weight: 500;
  }
  button {
    background: var(--accent);
    color: #ffffff;
    border: none;
    border-radius: 4px;
    padding: 2px 8px;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: all var(--transition-fast);
  }
  button:hover:not(:disabled) {
    opacity: 0.9;
    transform: scale(1.02);
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  button:disabled {
    cursor: default;
    opacity: 0.6;
  }
</style>
