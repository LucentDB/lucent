// The single QueryClient for the app. Exported as a value so tests can inspect
// defaults; components must reach it through `useQueryClient()` so the provider
// stays the one source of truth.
import { QueryClient } from '@tanstack/svelte-query';

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // One retry: Tauri IPC failures are usually real (no connection, bad
      // command), not transient network blips, so retrying hard just delays
      // the error the user needs to see.
      retry: 1,
      // A Tauri window regains focus on every dialog dismiss and devtools
      // toggle. Refetching there would fire catalog IPC constantly.
      refetchOnWindowFocus: false,
      // Catalog data changes when the user changes it, not on a timer.
      staleTime: 30_000,
    },
  },
});
