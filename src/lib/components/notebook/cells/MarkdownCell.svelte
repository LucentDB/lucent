<script lang="ts">
  import TextCellEditor from './TextCellEditor.svelte';
  import { toggleTaskAtIndex } from '../../chat/markdown.ts';

  let {
    source,
    status,
    onSourceChange,
  }: {
    source: string;
    status: string;
    onSourceChange?: (val: string) => void;
  } = $props();

  let editing = $state(false);
  let userClosed = $state(false);

  // Auto-enter editing only for a brand-new empty cell.
  $effect(() => {
    if (!editing && !userClosed && status === 'pending' && !source) {
      editing = true;
    }
  });
</script>

<TextCellEditor
  {source}
  {editing}
  placeholder="Empty markdown cell — click to edit"
  renderMode="markdown"
  showToolbar={true}
  {onSourceChange}
  onEnterEdit={() => {
    editing = true;
    userClosed = false;
  }}
  onExitEdit={() => {
    editing = false;
    userClosed = true;
  }}
  onToggleTask={(index, checked) =>
    onSourceChange?.(toggleTaskAtIndex(source, index, checked))}
/>
