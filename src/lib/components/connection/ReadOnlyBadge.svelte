<script module>
  /**
   * Derive the badge's presentation from the connection's capabilities.
   *
   * Exported and pure so the warning logic is testable without mounting.
   * `warning` is null when the guarantee is intact — the badge deliberately
   * says nothing rather than reassuring, so its presence is the signal.
   *
   * @param {{driver: string, displayName: string, engineEnforcedReadonly: boolean,
   *          readonlyDisclosure: string | null} | null} capabilities
   */
  export function badgeState(capabilities) {
    if (!capabilities) return { tone: 'hidden', label: '', warning: null };
    return {
      tone: capabilities.engineEnforcedReadonly ? 'neutral' : 'warning',
      label: capabilities.displayName,
      warning: capabilities.readonlyDisclosure ?? null,
    };
  }
</script>

<script>
  let { capabilities = null } = $props();
  const state = $derived(badgeState(capabilities));
</script>

{#if state.tone !== 'hidden'}
  <span
    class="badge"
    class:warning={state.tone === 'warning'}
    title={state.warning ?? state.label}
  >
    {state.label}
    {#if state.tone === 'warning'}<span aria-hidden="true">⚠</span>{/if}
  </span>
{/if}

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.125rem 0.5rem;
    border-radius: 999px;
    font-size: 0.75rem;
    background: var(--surface-2, #eee);
    color: var(--text-2, #444);
  }
  .badge.warning {
    background: var(--warning-bg, #fdf0d5);
    color: var(--warning-fg, #8a5300);
  }
</style>
