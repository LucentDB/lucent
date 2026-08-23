<script lang="ts">
  /**
   * One setting: what it is on the left, the control that changes it on the
   * right. Every row in the dialog uses this, so the eye can find the controls
   * down a single right edge instead of hunting a different layout per card.
   *
   * `stacked` is for controls too wide to sit beside a label (the provider
   * grid, the model list). They keep the label above and the control below,
   * rather than being crammed into the right column.
   */
  let {
    label,
    description,
    forId,
    stacked = false,
    control,
  }: {
    label: string;
    description?: string;
    /** Set when the control is a real form element, so the label binds to it. */
    forId?: string;
    stacked?: boolean;
    control: import('svelte').Snippet;
  } = $props();
</script>

<div class="row" class:stacked>
  <div class="text">
    {#if forId}
      <label class="label" for={forId}>{label}</label>
    {:else}
      <span class="label">{label}</span>
    {/if}
    {#if description}
      <span class="description">{description}</span>
    {/if}
  </div>
  <div class="control">
    {@render control()}
  </div>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: 9px 12px;
    min-height: 38px;
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
  }
  .label {
    font-size: var(--text-base);
    color: var(--text);
  }
  /* The explanation, not a second label. What used to be a 90-character
     sentence inside a checkbox label belongs here. */
  .description {
    font-size: var(--text-xs);
    color: var(--text-muted);
    line-height: 1.45;
  }
  .control {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .row.stacked {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
  }
  .row.stacked .control {
    justify-content: flex-start;
  }
</style>
