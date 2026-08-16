<script lang="ts">
  import type { AgentPermissionPayload } from '../../ipc/ai.ts';

  let {
    permission,
    onAllow,
    onReject,
  }: {
    permission: AgentPermissionPayload;
    onAllow?: () => void;
    onReject?: () => void;
  } = $props();
</script>

<div class="perm-card">
  <div class="perm-hdr">
    <span class="perm-icon">🛡️</span>
    <span class="perm-title">{permission.title}</span>
    <button
      class="perm-close"
      aria-label="Dismiss permission request"
      title="Reject and dismiss"
      onclick={onReject}>×</button
    >
  </div>
  <p class="perm-desc">{permission.description}</p>
  {#if permission.options.length > 0}
    <div class="perm-options">
      {#each permission.options as opt (opt.id)}
        <span class="perm-option">{opt.name}</span>
      {/each}
    </div>
  {/if}
  <div class="perm-actions">
    <button class="btn-reject" onclick={onReject}>Reject</button>
    <button class="btn-allow" onclick={onAllow}>Allow once</button>
  </div>
</div>

<style>
  .perm-card {
    border: 1px solid var(--accent);
    border-radius: var(--radius-md);
    padding: 14px;
    margin: 8px 0;
    background: var(--bg-surface);
  }
  .perm-hdr {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 8px;
  }
  .perm-icon {
    font-size: var(--text-lg);
  }
  .perm-title {
    font-weight: var(--weight-semibold);
    font-size: var(--text-md);
    flex: 1;
  }
  .perm-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
    flex-shrink: 0;
    transition: all var(--transition-fast);
  }
  .perm-close:hover {
    background: var(--danger-bg);
    color: var(--danger);
  }
  .perm-desc {
    font-size: var(--text-sm);
    color: var(--text-secondary);
    margin-bottom: 8px;
  }
  .perm-options {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 10px;
  }
  .perm-option {
    font-size: var(--text-xs);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    padding: 3px 8px;
    border-radius: var(--radius-full);
  }
  .perm-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
  .btn-allow {
    background: var(--accent);
    color: #fff;
    border: none;
    padding: 7px 20px;
    border-radius: var(--radius-md);
    cursor: pointer;
    font-weight: var(--weight-semibold);
    font-size: var(--text-sm);
    transition: opacity var(--transition-fast);
  }
  .btn-allow:hover {
    opacity: 0.9;
  }
  .btn-reject {
    background: transparent;
    border: 1px solid var(--border);
    padding: 7px 20px;
    border-radius: var(--radius-md);
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-secondary);
    transition: all var(--transition-fast);
  }
  .btn-reject:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
</style>
