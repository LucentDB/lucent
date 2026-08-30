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
    <span>Update {available.version}</span>
    <button onclick={install} disabled={installing}>
      {installing ? 'Installing…' : 'Install'}
    </button>
  </div>
{/if}

<style>
  .update {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.8rem;
  }
  button {
    cursor: pointer;
  }
  button:disabled {
    cursor: default;
    opacity: 0.6;
  }
</style>
