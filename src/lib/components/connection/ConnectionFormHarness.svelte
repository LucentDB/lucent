<script lang="ts">
  import { QueryClientProvider, QueryClient } from '@tanstack/svelte-query';
  import ConnectionForm from './ConnectionForm.svelte';

  let { profile, onCancel } = $props();

  // Retry off so an intentionally-rejecting mock fails the test fast.
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
</script>

<QueryClientProvider {client}>
  <ConnectionForm {profile} {onCancel} />
</QueryClientProvider>
