<script lang="ts">
  /**
   * A group of related settings as one inset panel of hairline-separated rows.
   *
   * This is the dialog's only container. The layout it replaces used cards with
   * uppercase micro-titles, a nested `<details>`, and one card with no title at
   * all; a single grouped-list vocabulary is what makes a settings surface
   * scannable, and it is the pattern the platform's own settings use.
   */
  let {
    title,
    hint,
    children,
  }: {
    /** Omit for a group whose contents speak for themselves. */
    title?: string;
    hint?: string;
    children: import('svelte').Snippet;
  } = $props();
</script>

<section class="group">
  {#if title}
    <h3 class="group-title">{title}</h3>
  {/if}
  <div class="rows">
    {@render children()}
  </div>
  {#if hint}
    <p class="group-hint">{hint}</p>
  {/if}
</section>

<style>
  .group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  /* Sentence case at text size, not an uppercase tracked-out micro-label. A
     group heading is a heading, not a decorative eyebrow. */
  .group-title {
    margin: 0;
    padding: 0 2px;
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text);
  }
  .rows {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-elevated);
    overflow: hidden;
  }
  /* The separator belongs to the group, so a row does not need to know
     whether it is last. */
  .rows > :global(* + *) {
    border-top: 1px solid var(--border-light);
  }
  .group-hint {
    margin: 0;
    padding: 0 2px;
    font-size: var(--text-xs);
    color: var(--text-muted);
    line-height: 1.45;
  }
</style>
