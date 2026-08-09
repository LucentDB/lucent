<script lang="ts">
  import Cell from './Cell.svelte';
  import AddCellButton from './AddCellButton.svelte';
  import type {
    NotebookModel,
    CellModel,
  } from '../../stores/notebook.svelte.ts';

  let {
    model,
    onSelect,
    children,
  }: {
    model: NotebookModel;
    onSelect?: (id: string) => void;
    children?: import('svelte').Snippet<[CellModel]>;
  } = $props();

  let dragCellId = $state<string | null>(null);
  let dragOverIdx = $state<number | null>(null);
  let startIdx = $state<number>(0);

  function getTargetIdx(clientY: number): number {
    const wrappers = Array.from(
      document.querySelectorAll<HTMLElement>('.cell-wrapper'),
    );
    for (let i = 0; i < wrappers.length; i++) {
      const rect = wrappers[i].getBoundingClientRect();
      const mid = rect.top + rect.height / 2;
      if (clientY < mid) {
        return i;
      }
    }
    return Math.max(0, wrappers.length - 1);
  }

  function onGripDown(cellId: string, e: PointerEvent) {
    if (e.button !== 0) return; // Left click only
    e.preventDefault();

    const gripTarget = e.currentTarget as HTMLElement;
    try {
      gripTarget.setPointerCapture(e.pointerId);
    } catch {
      // Ignore if setPointerCapture is unsupported in testing env
    }

    dragCellId = cellId;
    startIdx = model.cells.findIndex((c) => c.id === cellId);
    if (startIdx < 0) return;

    dragOverIdx = startIdx;
    document.body.style.userSelect = 'none';
    document.body.style.webkitUserSelect = 'none';

    function onMove(ev: PointerEvent) {
      ev.preventDefault();
      const targetIdx = getTargetIdx(ev.clientY);
      dragOverIdx = targetIdx;
    }

    function onUp(ev: PointerEvent) {
      document.removeEventListener('pointermove', onMove);
      document.removeEventListener('pointerup', onUp);
      try {
        if (gripTarget.hasPointerCapture(ev.pointerId)) {
          gripTarget.releasePointerCapture(ev.pointerId);
        }
      } catch {
        // Ignore
      }
      document.body.style.userSelect = '';
      document.body.style.webkitUserSelect = '';

      if (
        dragOverIdx !== null &&
        dragOverIdx !== startIdx &&
        dragOverIdx >= 0 &&
        dragOverIdx < model.cells.length
      ) {
        model.moveCell(cellId, dragOverIdx - startIdx);
      }
      dragCellId = null;
      dragOverIdx = null;
    }

    document.addEventListener('pointermove', onMove);
    document.addEventListener('pointerup', onUp);
  }
</script>

<div class="cell-list">
  {#if model.cells.length === 0}
    <div class="empty-state">
      <p class="empty-text">No cells yet</p>
      <AddCellButton onAdd={(kind) => model.addCell(null, kind)} />
    </div>
  {:else}
    {#each model.cells as cell, idx (cell.id)}
      <div class="cell-wrapper" class:dragging={dragCellId === cell.id}>
        {#if dragOverIdx === idx && dragCellId !== cell.id}
          <div class="insertion-marker"></div>
        {/if}
        <Cell
          {cell}
          {model}
          selected={model.selectedCellId === cell.id}
          onSelect={() => onSelect?.(cell.id)}
          onGripDown={(e) => onGripDown(cell.id, e)}
        >
          {#if children}
            {@render children(cell)}
          {/if}
        </Cell>
        <AddCellButton onAdd={(kind) => model.addCell(cell.id, kind)} />
      </div>
    {/each}
  {/if}
</div>

<style>
  .cell-list {
    display: flex;
    flex-direction: column;
    flex: 1;
  }
  .cell-wrapper {
    position: relative;
    transition:
      opacity 0.15s ease,
      transform 0.15s ease;
  }
  .cell-wrapper.dragging {
    opacity: 0.45;
  }
  .insertion-marker {
    height: 3px;
    background: var(--accent);
    box-shadow: 0 0 8px var(--accent);
    margin: 3px 0;
    border-radius: 2px;
  }
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 48px 24px;
    gap: 16px;
  }
  .empty-text {
    color: var(--text-muted);
    font-size: var(--text-sm);
    margin: 0;
  }
</style>
