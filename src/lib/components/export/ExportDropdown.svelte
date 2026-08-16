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

  const exportFormats = [
    { id: 'csv', label: 'CSV', ext: '.csv' },
    { id: 'json', label: 'JSON', ext: '.json' },
    { id: 'sql-insert', label: 'INSERTs', ext: '.sql' },
  ] as const;

  async function handleExport(formatId: string) {
    open = false;
    if (columns.length === 0 || rows.length === 0) return;

    const format =
      formatId === 'csv'
        ? { Csv: {} }
        : formatId === 'json'
          ? { Json: {} }
          : { SqlInsert: {} };

    const ext = exportFormats.find((f) => f.id === formatId)?.ext ?? '.csv';

    try {
      // The path must be chosen in a native Rust-side dialog so the write
      // command's approved-path gate accepts it (frontend paths are untrusted).
      const path = await invoke<string | null>('choose_export_path', {
        defaultName: `export${ext}`,
        filterName: formatId.toUpperCase(),
        extensions: [ext.slice(1)],
      });
      if (!path) return; // cancelled

      await invoke('export_results', {
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
        path,
      });
    } catch (e) {
      console.error('Export failed:', e);
    }
  }
</script>

<div class="export-dropdown">
  <button
    class="export-btn"
    {disabled}
    onclick={() => (open = !open)}
    title="Export results"
  >
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
    >
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
      <polyline points="7 10 12 15 17 10" />
      <line x1="12" y1="15" x2="12" y2="3" />
    </svg>
  </button>

  {#if open}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dropdown-backdrop" onclick={() => (open = false)}></div>
    <div class="dropdown-menu">
      {#each exportFormats as fmt}
        <button class="menu-item" onclick={() => handleExport(fmt.id)}>
          <span class="menu-label">Export as {fmt.label}</span>
          <span class="menu-ext">{fmt.ext}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .export-dropdown {
    position: relative;
  }
  .export-btn {
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
  .export-btn:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text);
  }
  .export-btn:disabled {
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
    min-width: 180px;
    overflow: hidden;
  }
  .menu-item {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
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
  .menu-label {
    font-weight: 500;
  }
  .menu-ext {
    font-size: 11px;
    color: var(--text-muted);
    font-family: var(--font-mono);
  }
</style>
