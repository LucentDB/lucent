<script>
  import { QueryClientProvider, QueryClient } from '@tanstack/svelte-query';
  import Sidebar from './Sidebar.svelte';

  let { onObjectClick } = $props();
  // Retry off so an intentionally-rejecting mock fails the test fast.
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
</script>

<QueryClientProvider {client}>
  <Sidebar {onObjectClick} />
</QueryClientProvider>
