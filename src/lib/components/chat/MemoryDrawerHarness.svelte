<script lang="ts">
  import MemoryDrawer from './MemoryDrawer.svelte';

  let {
    onClose,
    connectionId = 'conn-1',
    emulateInertBlur = false,
  }: {
    onClose?: () => void;
    connectionId?: string;
    /**
     * Emulate the browser's inert semantics: making the background inert blurs
     * the focused opener to <body>. jsdom does not implement this, so the
     * regression test opts in to prove focus restore does not depend on
     * `document.activeElement`.
     */
    emulateInertBlur?: boolean;
  } = $props();

  let isOpen = $state(false);
  let triggerEl = $state<HTMLButtonElement | null>(null);

  function open() {
    isOpen = true;
    if (emulateInertBlur) {
      (document.activeElement as HTMLElement | null)?.blur();
    }
  }
</script>

<!-- A stand-in for the app content behind the drawer. The `.panel-header` in
     ChatPanel holds the real trigger; this mirrors it so focus restore can be
     exercised without dragging in the whole chat panel. -->
<div class="background">
  <button data-testid="background-button" type="button">Background</button>
</div>

<button
  data-testid="trigger"
  bind:this={triggerEl}
  type="button"
  onclick={open}
>
  Open memory
</button>

<MemoryDrawer bind:isOpen {connectionId} {onClose} {triggerEl} />
