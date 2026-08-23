<script lang="ts">
  /**
   * The one switch in the settings dialog. Three hand-rolled copies of this
   * markup were inlined across the old single-page layout, which is how they
   * drifted: consistency of form controls is the whole point of a settings
   * surface, and a switch that looks different in two places means one of them
   * is wrong.
   */
  let {
    checked = $bindable(false),
    label,
    describedBy,
  }: {
    checked?: boolean;
    /** Accessible name. The visible text lives in the row beside the switch. */
    label: string;
    describedBy?: string;
  } = $props();
</script>

<button
  type="button"
  role="switch"
  class="sw"
  aria-checked={checked}
  aria-label={label}
  aria-describedby={describedBy}
  onclick={() => (checked = !checked)}
>
  <span class="thumb"></span>
</button>

<style>
  /* AppKit proportions: a 26x16 track reads as a system control, where the
     40px+ tracks of web UI kits read as a toy. */
  .sw {
    flex-shrink: 0;
    position: relative;
    width: 26px;
    height: 16px;
    padding: 0;
    border: none;
    border-radius: var(--radius-full);
    background: var(--border);
    cursor: pointer;
    transition: background var(--transition-normal);
  }
  .sw[aria-checked='true'] {
    background: var(--accent);
  }
  .sw:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 12px;
    height: 12px;
    border-radius: var(--radius-full);
    background: var(--bg-elevated);
    box-shadow: var(--shadow-sm);
    /* Transform, not `left`: animating a layout property here would lay out
       the row on every frame of the toggle. */
    transition: transform var(--transition-normal);
  }
  .sw[aria-checked='true'] .thumb {
    transform: translateX(10px);
  }
  @media (prefers-reduced-motion: reduce) {
    .sw,
    .thumb {
      transition: none;
    }
  }
</style>
