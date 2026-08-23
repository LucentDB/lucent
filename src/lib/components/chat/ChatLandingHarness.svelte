<script lang="ts">
  import {
    QueryClientProvider,
    type QueryClient,
  } from '@tanstack/svelte-query';
  import type { ComponentProps } from 'svelte';
  import ChatLanding from './ChatLanding.svelte';

  let {
    client,
    ...rest
  }: { client: QueryClient } & ComponentProps<typeof ChatLanding> = $props();
</script>

<QueryClientProvider {client}>
  <ChatLanding {...rest} />
</QueryClientProvider>
