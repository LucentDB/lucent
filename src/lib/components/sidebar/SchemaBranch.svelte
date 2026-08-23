<script>
  import { objectsQuery } from '../../queries/explorer.ts';

  let { conn, schema, expanded, ondata, children } = $props();

  const objects = objectsQuery(
    () => conn,
    () => schema.path,
    () => expanded,
  );

  // Surface loaded objects to the parent so search filtering and the match
  // badge can consider them, mirroring the shared objectsBySchema map the
  // generation-counter implementation kept. Report each data array ONCE:
  // re-reporting on observer churn would have the parent rewrite state,
  // re-render this branch, and re-fire this effect — an update loop.
  let reportedData = null;
  $effect(() => {
    if (objects.data && objects.data !== reportedData) {
      reportedData = objects.data;
      ondata?.(schema.name, objects.data);
    }
  });
</script>

{#if expanded}
  {#if objects.isPending}
    <div class="loading-line">Loading…</div>
  {:else if objects.error}
    <div class="sidebar-error">{objects.error}</div>
  {:else}
    {@render children(objects.data ?? [])}
  {/if}
{/if}
