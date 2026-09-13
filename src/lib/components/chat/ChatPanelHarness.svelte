<script lang="ts">
  import {
    QueryClientProvider,
    type QueryClient,
  } from '@tanstack/svelte-query';
  import type { ComponentProps } from 'svelte';
  import ChatPanel from './ChatPanel.svelte';

  let {
    client,
    ...rest
  }: { client: QueryClient } & ComponentProps<typeof ChatPanel> = $props();
</script>

<QueryClientProvider {client}>
  <ChatPanel {...rest} />
</QueryClientProvider>
