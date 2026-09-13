<script>
  // The row-number cell doubles as the row selector. No mature SQL client
  // spends 32px of a 12-column grid on a checkbox column. Spec D1.
  let { rowNumber, selected = false, onSelect } = $props();
</script>

<button
  class="gutter"
  class:selected
  type="button"
  aria-label="Select row {rowNumber}"
  aria-pressed={selected}
  onclick={onSelect}
>
  {rowNumber}
</button>

<style>
  .gutter {
    display: block;
    width: 100%;
    box-sizing: border-box;
    padding: 0 0.35rem;
    border: 0;
    background: transparent;
    color: var(--text-dim, #7d8590);
    font: inherit;
    font-variant-numeric: tabular-nums;
    text-align: right;
    cursor: default;
    overflow: hidden;
    text-overflow: clip;
    white-space: nowrap;
  }

  .gutter:hover {
    color: var(--text, #e6edf3);
    background: var(--bg-hover, rgba(255, 255, 255, 0.04));
  }

  .gutter.selected {
    background: var(--accent-soft, rgba(120, 140, 255, 0.18));
    color: var(--accent, #6b7cff);
  }
</style>
