<script lang="ts">
  import type { CellModel } from '../../stores/notebook.svelte.ts';

  let {
    source,
    cells,
  }: {
    source: string;
    cells?: CellModel[];
    expanded?: boolean;
  } = $props();

  const CELL_REF_RE =
    /\$\{([a-f0-9]{8})\}|\$([a-f0-9]{8})\.([a-z_][a-z0-9_]*)/g;

  function extractIds(src: string): Set<string> {
    const ids = new Set<string>();
    let match;
    const re = /\$\{([a-f0-9]{8})\}|\$([a-f0-9]{8})\./g;
    while ((match = re.exec(src)) !== null) {
      ids.add(match[1] ?? match[2]);
    }
    return ids;
  }

  // Recursively collect all referenced cell IDs (transitive closure)
  let allReferencedIds = $derived.by(() => {
    if (!cells) return new Set<string>();
    const cellMap = new Map(cells.map((c) => [c.id, c]));
    const visited = new Set<string>();
    const queue = [...extractIds(source)];
    while (queue.length > 0) {
      const id = queue.pop()!;
      if (visited.has(id)) continue;
      visited.add(id);
      const cell = cellMap.get(id);
      if (cell?.source) {
        for (const depId of extractIds(cell.source)) {
          if (!visited.has(depId)) queue.push(depId);
        }
      }
    }
    return visited;
  });

  // Build CTEs in dependency order (topological sort)
  let ctes = $derived.by(() => {
    if (!cells || allReferencedIds.size === 0) return '';
    const cellMap = new Map(cells.map((c) => [c.id, c]));
    const parts: string[] = [];
    const added = new Set<string>();

    // Helper to resolve cell references in a source string
    function resolveRefs(src: string): string {
      return src.replace(CELL_REF_RE, (_m, braceId, dotId, col) => {
        const cellId = braceId ?? dotId;
        if (braceId) return `_cell_${cellId}`;
        return `(SELECT ${col} FROM _cell_${cellId} LIMIT 1)`;
      });
    }

    // Helper to add cell and its dependencies first
    function addWithDeps(id: string) {
      if (added.has(id) || !allReferencedIds.has(id)) return;
      const cell = cellMap.get(id);
      if (!cell?.source) return;
      // Add dependencies first
      for (const depId of extractIds(cell.source)) {
        addWithDeps(depId);
      }
      // Resolve references in this cell's source using already-resolved deps
      const resolved = resolveRefs(cell.source);
      parts.push(`_cell_${id} AS (${resolved})`);
      added.add(id);
    }

    for (const id of allReferencedIds) {
      addWithDeps(id);
    }

    return parts.length > 0 ? `WITH ${parts.join(',\n  ')}` : '';
  });

  // Replace cell references with table names
  let mainQuery = $derived(
    source.replace(CELL_REF_RE, (_match, braceId, dotId, col) => {
      const cellId = braceId ?? dotId;
      if (braceId) return `_cell_${cellId}`;
      return `(SELECT ${col} FROM _cell_${cellId} LIMIT 1)`;
    }),
  );

  let preview = $derived(ctes ? `${ctes}\n${mainQuery}` : mainQuery);

  let show = $state(true);
</script>

{#if preview !== source}
  <div class="ref-preview">
    <button
      class="preview-toggle"
      onclick={() => (show = !show)}
      aria-expanded={show}
    >
      {show ? '▾' : '▸'} Resolved query
    </button>
    {#if show}
      <pre class="preview-sql">{preview}</pre>
    {/if}
  </div>
{/if}

<style>
  .ref-preview {
    margin: 4px 12px 8px;
  }
  .preview-toggle {
    padding: 0;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: var(--text-xs);
    cursor: pointer;
  }
  .preview-toggle:hover {
    color: var(--text-secondary);
  }
  .preview-sql {
    margin: 4px 0 0;
    padding: 8px;
    background: var(--bg-subtle);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-muted);
    white-space: pre-wrap;
    max-height: 140px;
    overflow: auto;
  }
</style>
