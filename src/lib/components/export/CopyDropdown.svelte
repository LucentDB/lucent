<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';

  let {
    disabled = false,
    columns = [],
    rows = [],
  }: {
    disabled?: boolean;
    columns?: { name: string; typeName: string }[];
    rows?: any[][];
  } = $props();

  let open = $state(false);

  const copyFormats = [
    { id: 'csv', label: 'CSV' },
    { id: 'json', label: 'JSON' },
    { id: 'sql-insert', label: 'INSERTs' },
  ] as const;

  async function handleCopy(formatId: string) {
    open = false;
    if (columns.length === 0 || rows.length === 0) return;

    const format =
      formatId === 'csv'
        ? { Csv: {} }
        : formatId === 'json'
          ? { Json: {} }
          : { SqlInsert: {} };

    try {
      await invoke('copy_results', {
        columns,
        rows,
        format,
        options: {
          format,
          includeHeader: true,
          delimiter: ',',
          nullString: '\\N',
          tableName: formatId === 'sql-insert' ? 'table_name' : null,
        },
      });
    } catch (e) {
      console.error('Copy failed:', e);
    }
  }
</script>

<div class="copy-dropdown">
  <button
    class="copy-btn"
    {disabled}
    onclick={() => (open = !open)}
    title="Copy results"
  >
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
    >
      <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </svg>
  </button>

  {#if open}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dropdown-backdrop" onclick={() => (open = false)}></div>
    <div class="dropdown-menu">
      {#each copyFormats as fmt}
        <button class="menu-item" onclick={() => handleCopy(fmt.id)}>
          Copy as {fmt.label}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .copy-dropdown {
    position: relative;
  }
  .copy-btn {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    color: var(--text-secondary);
    cursor: pointer;
  }
  .copy-btn:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text);
  }
  .copy-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .dropdown-backdrop {
    position: fixed;
    inset: 0;
    z-index: 99;
  }
  .dropdown-menu {
    position: absolute;
    top: 100%;
    right: 0;
    margin-top: 4px;
    z-index: 100;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-lg);
    min-width: 160px;
    overflow: hidden;
  }
  .menu-item {
    width: 100%;
    padding: 8px 14px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    cursor: pointer;
    text-align: left;
  }
  .menu-item:hover {
    background: var(--bg-hover);
  }
</style>
