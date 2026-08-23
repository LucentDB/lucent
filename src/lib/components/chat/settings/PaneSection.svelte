<script lang="ts">
  /**
   * A heading over free-form content, with no container of its own.
   *
   * The dialog has two vocabularies and they are not interchangeable:
   * SettingsGroup wraps a list of label-and-control rows in a bordered panel,
   * while this wraps a composite that already draws its own boxes, such as the
   * provider grid, the model picker, or the agent registry. Putting those
   * inside a SettingsGroup nested a bordered panel around a set of bordered
   * cards, which is the one container mistake that always reads as sloppy.
   */
  let {
    title,
    hint,
    action,
    children,
  }: {
    title: string;
    hint?: string;
    /** A control belonging to the heading, right-aligned beside it. */
    action?: import('svelte').Snippet;
    children: import('svelte').Snippet;
  } = $props();
</script>

<section class="section">
  <div class="head">
    <h3>{title}</h3>
    {#if action}
      <div class="action">{@render action()}</div>
    {/if}
  </div>
  {@render children()}
  {#if hint}
    <p class="hint">{hint}</p>
  {/if}
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 2px;
  }
  h3 {
    margin: 0;
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text);
  }
  /* The heading's own control, so "Fetch Models" sits with the thing it acts
     on instead of below the list it populates. */
  .action {
    margin-left: auto;
  }
  .hint {
    margin: 0;
    padding: 0 2px;
    font-size: var(--text-xs);
    color: var(--text-muted);
    line-height: 1.45;
  }
</style>
