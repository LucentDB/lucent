<script lang="ts">
  import TextCellEditor from './TextCellEditor.svelte';
  import { toggleTaskAtIndex } from '../../chat/markdown.ts';

  let {
    source,
    status,
    editing: controlledEditing,
    onSourceChange,
    onRun,
    onRunAndAdvance,
    onEnterEdit,
    onExitEdit,
  }: {
    source: string;
    status: string;
    /** When supplied, notebook mode owns whether this cell is being edited. */
    editing?: boolean;
    onSourceChange?: (val: string) => void;
    onRun?: () => void;
    onRunAndAdvance?: () => void;
    onEnterEdit?: () => void;
    onExitEdit?: () => void;
  } = $props();

  let localEditing = $state(false);
  let userClosed = $state(false);
  let isEditing = $derived(
    controlledEditing === undefined ? localEditing : controlledEditing,
  );

  // Standalone MarkdownCell usage keeps its original empty-cell convenience;
  // Notebook.svelte passes `editing` and owns the mode when embedded.
  $effect(() => {
    if (controlledEditing !== undefined) return;
    if (!localEditing && !userClosed && status === 'pending' && !source) {
      localEditing = true;
    }
  });

  function enterEdit() {
    if (controlledEditing === undefined) {
      localEditing = true;
      userClosed = false;
    }
    onEnterEdit?.();
  }

  function exitEdit() {
    if (controlledEditing === undefined) {
      localEditing = false;
      userClosed = true;
    }
    onExitEdit?.();
  }
</script>

<TextCellEditor
  {source}
  editing={isEditing}
  placeholder="Empty markdown cell — click to edit"
  renderMode="markdown"
  showToolbar={true}
  {onSourceChange}
  {onRun}
  {onRunAndAdvance}
  onEnterEdit={enterEdit}
  onExitEdit={exitEdit}
  onToggleTask={(index, checked) =>
    onSourceChange?.(toggleTaskAtIndex(source, index, checked))}
/>
