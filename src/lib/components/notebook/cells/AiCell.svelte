<script lang="ts">
  import TextCellEditor from './TextCellEditor.svelte';
  import AiCellOutput from '../AiCellOutput.svelte';
  import type {
    CellModel,
    NotebookModel,
  } from '../../../stores/notebook.svelte.ts';

  let {
    cell,
    model,
    editing: controlledEditing,
    onEnterEdit,
    onExitEdit,
  }: {
    cell: CellModel;
    model: NotebookModel;
    /** When supplied, notebook mode owns whether this cell is being edited. */
    editing?: boolean;
    onEnterEdit?: () => void;
    onExitEdit?: () => void;
  } = $props();

  let localEditing = $state(false);
  let isEditing = $derived(
    controlledEditing === undefined ? localEditing : controlledEditing,
  );
  let isRunning = $derived(cell.status === 'running');
  let hasOutput = $derived(
    isRunning ||
      cell.status === 'ok' ||
      cell.status === 'error' ||
      !!cell.ai_state,
  );

  function handleEditSql(sql: string) {
    const next =
      model.cells[model.cells.findIndex((c) => c.id === cell.id) + 1];
    // Reuse an empty SQL cell directly below rather than stacking a new one.
    if (next && next.kind === 'sql' && !next.source.trim()) {
      model.setCellSource(next.id, sql);
      model.select(next.id);
      return;
    }
    const id = model.insertCell(cell.id, 'below', 'sql');
    model.setCellSource(id, sql);
  }
</script>

<div class="ai-cell">
  <TextCellEditor
    source={cell.source}
    editing={isEditing && !isRunning}
    placeholder="Ask a question about your data…"
    renderMode="auto"
    onSourceChange={(v) => model.setCellSource(cell.id, v)}
    onRun={() => model.runCell(cell.id)}
    onRunAndAdvance={() => model.runAndAdvance(cell.id)}
    onEnterEdit={() => {
      if (isRunning) return;
      if (controlledEditing === undefined) localEditing = true;
      onEnterEdit?.();
    }}
    onExitEdit={() => {
      if (controlledEditing === undefined) {
        localEditing = false;
        model.enterCommandMode();
      }
      onExitEdit?.();
    }}
  />

  {#if hasOutput}
    <AiCellOutput {cell} {model} onEditSql={handleEditSql} />
  {/if}
</div>

<style>
  .ai-cell {
    position: relative;
  }
</style>
