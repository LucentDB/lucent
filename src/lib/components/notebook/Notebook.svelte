<script lang="ts">
  import NotebookToolbar from './NotebookToolbar.svelte';
  import CellList from './CellList.svelte';
  import SqlCell from './cells/SqlCell.svelte';
  import MarkdownCell from './cells/MarkdownCell.svelte';
  import AiCell from './cells/AiCell.svelte';
  import { notebooks } from '../../stores/notebooks.svelte.ts';
  import { createCommandKeymap } from './keymap.ts';
  import type {
    CellModel,
    NotebookModel,
  } from '../../stores/notebook.svelte.ts';

  let {
    tabId,
    filePath = null as string | null,
    connectionId = '',
    database = '',
  }: {
    tabId: string;
    filePath?: string | null;
    connectionId?: string;
    database?: string;
  } = $props();

  // The registry owns the model, so switching tabs preserves cells, outputs, and
  // the attached DB session. Keyed on tabId, so each notebook tab gets its own.
  // Must be $state + $effect, not $derived: ensure() mutates the registry's
  // reactive map, and mutating state during $derived evaluation is forbidden
  // (state_unsafe_mutation). The initializer covers first mount; the effect
  // re-points at the registry when the tab (or its attach spec) changes, since
  // App.svelte reuses this component instance across notebook-tab switches.
  // ensure() is idempotent, so re-running it is cheap and never re-attaches.
  //
  // The ensure() call is routed through a small helper so the $state
  // initialiser snapshots the props once.  The $effect below handles
  // every subsequent change; initial snapshots are by design here.
  function _initModel(tid: string, fp: string | null, cid: string, db: string) {
    return notebooks.ensure(tid, {
      filePath: fp,
      connectionId: cid,
      database: db,
    });
  }
  // svelte-ignore state_referenced_locally -- intentional one-shot snapshot; $effect below handles updates
  let model = $state<NotebookModel>(
    _initModel(tabId, filePath, connectionId, database),
  );

  $effect(() => {
    model = notebooks.ensure(tabId, { filePath, connectionId, database });
  });

  let connectionName = $derived(model.metadata.connectionName ?? '');
  let databaseName = $derived(model.metadata.database ?? '');

  let handleCommandKey = $derived(createCommandKeymap(model));

  function onKeydown(e: KeyboardEvent) {
    // Edit mode belongs to the editors: CodeMirror and the textareas own their keys.
    if (model.mode === 'edit') return;
    // Ignore keys originating inside a text entry, even in command mode.
    const t = e.target as HTMLElement | null;
    if (
      t &&
      (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))
    ) {
      return;
    }
    if (handleCommandKey(e)) {
      e.preventDefault();
      e.stopPropagation();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="notebook"
  role="application"
  aria-label="SQL notebook"
  tabindex="-1"
  onkeydown={onKeydown}
>
  <NotebookToolbar
    onRunAll={() => model.runAll()}
    onClearOutputs={async () => {
      await model.clearOutputs();
    }}
    onRestartSession={() => model.restartSession()}
    isRunning={model.runningCellId !== null}
    runAllProgress={model.runAllProgress}
    {connectionName}
    {databaseName}
  />
  <CellList {model} onSelect={(id) => model.select(id)}>
    {#snippet children(cell: CellModel)}
      {#if cell.kind === 'sql'}
        <SqlCell
          source={cell.source}
          status={cell.status}
          cells={model.cells}
          cellId={cell.id}
          {model}
          selected={model.selectedCellId === cell.id}
          focused={model.selectedCellId === cell.id && model.mode === 'edit'}
          onSourceChange={(v) => model.setCellSource(cell.id, v)}
        />
      {:else if cell.kind === 'markdown'}
        <MarkdownCell
          source={cell.source}
          status={cell.status}
          onSourceChange={(v) => model.setCellSource(cell.id, v)}
        />
      {:else if cell.kind === 'ai'}
        <AiCell {cell} {model} />
      {/if}
    {/snippet}
  </CellList>
</div>

<style>
  .notebook {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: visible;
  }
</style>
