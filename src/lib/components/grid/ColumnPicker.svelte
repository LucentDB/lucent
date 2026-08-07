<script>
  let { columns = [], onPick = null, onClose = null } = $props();

  let query = $state('');
  let inputEl = $state(null);

  // ⚡ Bolt: Cache query.trim().toLowerCase() outside the filter loop to prevent
  // massive string allocation overhead on tables with many columns.
  let queryLower = $derived(query.trim().toLowerCase());

  let matches = $derived(
    columns.filter((c) => c.name.toLowerCase().includes(queryLower)),
  );

  $effect(() => {
    inputEl?.focus();
  });

  function pick(name) {
    onPick?.(name);
    onClose?.();
  }

  function handleKeydown(e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose?.();
      return;
    }
    if (e.key === 'Enter' && matches.length > 0) {
      e.preventDefault();
      pick(matches[0].name);
    }
  }
</script>

<div class="column-picker">
  <input
    bind:this={inputEl}
    class="picker-search"
    type="text"
    placeholder="Search columns…"
    bind:value={query}
    onkeydown={handleKeydown}
  />
  <div class="picker-list">
    {#each matches as col (col.name)}
      <button class="picker-item" onclick={() => pick(col.name)}>
        <span class="picker-name">{col.name}</span>
        <span class="picker-type">{col.type_name}</span>
      </button>
    {/each}
    {#if matches.length === 0}
      <div class="picker-empty">No matching columns</div>
    {/if}
  </div>
</div>

<style>
  .column-picker {
    position: absolute;
    z-index: 50;
    width: 260px;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
  }
  .picker-search {
    width: 100%;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-surface);
    color: var(--text);
    font-size: var(--text-sm);
    outline: none;
  }
  .picker-search:focus {
    border-color: var(--accent);
  }
  .picker-list {
    max-height: 240px;
    margin-top: 6px;
    overflow-y: auto;
  }
  .picker-item {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
    width: 100%;
    padding: 5px 8px;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    cursor: pointer;
    text-align: left;
  }
  .picker-item:hover {
    background: var(--bg-hover);
  }
  .picker-name {
    color: var(--text);
    font-size: var(--text-sm);
  }
  .picker-type {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }
  .picker-empty {
    padding: 8px;
    color: var(--text-muted);
    font-size: var(--text-sm);
  }
</style>
