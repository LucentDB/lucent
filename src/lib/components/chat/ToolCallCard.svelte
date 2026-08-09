<script lang="ts">
  import type { ToolCallCard as T } from '../../stores/chat.svelte.ts';
  let { tool, cellCompleted = false }: { tool: T; cellCompleted?: boolean } =
    $props();
  let open = $state(false);

  const icons: Record<string, string> = {
    get_objects_info: 'schema',
    search_objects: 'search',
    run_readonly_query: 'query',
    preview_dml: 'lock',
  };

  let statusIcon = $derived.by(() => {
    if (tool.summary === 'error' || tool.summary?.startsWith('error'))
      return 'error';
    if (tool.summary) return 'done';
    // No summary yet: still running, or cell finished without providing one.
    return cellCompleted ? 'done' : 'spinner';
  });

  let statusLabel = $derived.by(() => {
    if (tool.summary === 'error') return 'Failed';
    if (tool.summary?.startsWith('error')) return tool.summary;
    if (tool.summary) return tool.summary;
    return cellCompleted ? 'Done' : 'Running…';
  });

  function argDisplay(args: unknown): string {
    if (typeof args === 'string') return args;
    try {
      return JSON.stringify(args, null, 2);
    } catch {
      return String(args);
    }
  }

  function formatCell(v: unknown): string {
    if (v === null || v === undefined) return 'NULL';
    if (typeof v === 'object') return JSON.stringify(v);
    return String(v);
  }
</script>

<div
  class="tcc"
  class:open
  class:done={!!tool.summary}
  class:err={statusIcon === 'error'}
>
  <button class="tcc-hdr" onclick={() => (open = !open)}>
    <span class="tcc-icon">
      {#if statusIcon === 'spinner'}
        <svg
          class="spin"
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
        >
          <circle
            cx="12"
            cy="12"
            r="10"
            stroke-dasharray="31.4 31.4"
            stroke-linecap="round"
          />
        </svg>
      {:else if statusIcon === 'error'}
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
        >
          <circle cx="12" cy="12" r="10" /><line
            x1="15"
            y1="9"
            x2="9"
            y2="15"
          /><line x1="9" y1="9" x2="15" y2="15" />
        </svg>
      {:else}
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <polyline points="20 6 9 17 4 12" />
        </svg>
      {/if}
    </span>
    <span class="tcc-name">{tool.name.replace(/_/g, ' ')}</span>
    <span class="tcc-status">{statusLabel}</span>
    <svg
      class="tcc-chevron"
      class:open
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
    >
      <path
        d="M6 4l4 4-4 4"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  </button>

  {#if open}
    <div class="tcc-body">
      <div class="tcc-section">
        <div class="tcc-section-label">Input</div>
        <pre class="tcc-code">{argDisplay(tool.args)}</pre>
      </div>

      {#if tool.output}
        <div class="tcc-section">
          <div class="tcc-section-label">Output</div>
          {#if tool.output.type === 'query_result'}
            <div class="tcc-preview">
              <div class="tcc-preview-hdr">{tool.output.sql}</div>
              {#if tool.output.columns && tool.output.rows}
                <table class="tcc-table">
                  <thead>
                    <tr
                      >{#each tool.output.columns as col}<th>{col.name}</th
                        >{/each}</tr
                    >
                  </thead>
                  <tbody>
                    {#each tool.output.rows as row}
                      <tr
                        >{#each row as cell}<td>{formatCell(cell)}</td
                          >{/each}</tr
                      >
                    {/each}
                  </tbody>
                </table>
              {/if}
            </div>
          {:else if tool.output.type === 'text'}
            <pre class="tcc-code tcc-output-text">{tool.output.data}</pre>
          {:else if tool.output.type === 'dml_preview'}
            <div class="tcc-dml">
              <span class="tcc-dml-desc">{tool.output.description}</span>
              <pre class="tcc-code">{tool.output.sql}</pre>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .tcc {
    margin: 2px 0;
  }

  .tcc-hdr {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 2px 0;
    background: none;
    border: none;
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-muted);
    text-align: left;
  }

  .tcc-hdr:hover {
    color: var(--text-secondary);
  }

  .tcc-icon {
    width: 14px;
    height: 14px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .tcc-icon :global(.spin) {
    animation: rot 0.8s linear infinite;
  }

  @keyframes rot {
    to {
      transform: rotate(360deg);
    }
  }

  .tcc.err .tcc-icon {
    color: var(--danger);
  }

  .tcc.done .tcc-icon {
    color: var(--success);
  }

  .tcc-name {
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
    text-transform: capitalize;
    font-size: var(--text-xs);
  }

  .tcc-status {
    font-size: var(--text-xs);
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .tcc-status::before {
    content: '· ';
  }

  .tcc.err .tcc-status {
    color: var(--danger);
  }

  .tcc-chevron {
    flex-shrink: 0;
    color: var(--text-muted);
    opacity: 0;
    transition:
      transform 0.15s ease,
      opacity 0.15s ease;
  }

  .tcc-hdr:hover .tcc-chevron {
    opacity: 1;
  }

  .tcc-chevron.open {
    transform: rotate(90deg);
  }

  .tcc-body {
    margin-top: 4px;
    margin-left: 20px;
    padding: 8px 10px;
    background: var(--bg-subtle);
    border-radius: var(--radius-sm);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .tcc-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .tcc-section-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
    font-weight: var(--weight-semibold);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .tcc-code {
    margin: 0;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    background: var(--bg-surface);
    padding: 6px 8px;
    border-radius: var(--radius-sm);
    overflow-x: auto;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-all;
  }

  .tcc-output-text {
    color: var(--text-secondary);
  }

  .tcc-preview {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }

  .tcc-preview-hdr {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    padding: 4px 8px;
    background: var(--bg-subtle);
    color: var(--accent);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    white-space: nowrap;
  }

  .tcc-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-xs);
  }

  .tcc-table th {
    padding: 3px 6px;
    text-align: left;
    font-weight: var(--weight-semibold);
    color: var(--text-secondary);
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }

  .tcc-table td {
    padding: 2px 6px;
    border-bottom: 1px solid var(--border-light);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tcc-table tr:last-child td {
    border-bottom: none;
  }

  .tcc-dml {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .tcc-dml-desc {
    font-size: var(--text-sm);
    color: var(--warning);
  }
</style>
