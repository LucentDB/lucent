<script lang="ts">
  import Table from '@lucide/svelte/icons/table';
  import Layers2 from '@lucide/svelte/icons/layers-2';
  import Layers from '@lucide/svelte/icons/layers';
  import Braces from '@lucide/svelte/icons/braces';
  import Hash from '@lucide/svelte/icons/hash';
  import SquareTerminal from '@lucide/svelte/icons/square-terminal';
  import FileCode from '@lucide/svelte/icons/file-code';
  import BookOpen from '@lucide/svelte/icons/book-open';
  import Database from '@lucide/svelte/icons/database';
  import Folder from '@lucide/svelte/icons/folder';
  import FolderOpen from '@lucide/svelte/icons/folder-open';

  let {
    kind,
    size = 13,
    class: className = '',
    strokeWidth = 1.6,
    open = false,
  }: {
    kind: string;
    size?: number;
    class?: string;
    strokeWidth?: number;
    open?: boolean;
  } = $props();

  const iconMap: Record<string, any> = {
    table: Table,
    view: Layers2,
    matview: Layers,
    function: Braces,
    sequence: Hash,
    query: SquareTerminal,
    source: FileCode,
    notebook: BookOpen,
    database: Database,
    schema: Folder,
    'schema-open': FolderOpen,
  };

  let IconComponent = $derived(
    kind === 'schema'
      ? open
        ? FolderOpen
        : Folder
      : iconMap[kind] ?? FileCode,
  );
</script>

<IconComponent
  {size}
  {strokeWidth}
  class={className}
/>
