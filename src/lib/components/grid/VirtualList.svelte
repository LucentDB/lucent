<script>
  import { computeVirtualRange } from './virtualRange.js';

  let {
    itemCount = 0,
    rowHeight = 33,
    scrollTop = 0,
    viewportHeight = 600,
    overscan = 10,
    children,
  } = $props();

  let range = $derived(
    computeVirtualRange({
      scrollTop,
      viewportHeight,
      rowHeight,
      itemCount,
      overscan,
    }),
  );
</script>

{@render children(range)}
