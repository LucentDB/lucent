import { createQuery, queryOptions } from '@tanstack/svelte-query';
import { listDrivers } from '../ipc/client.js';
import { qk } from './keys.ts';
import type { DriverDescriptor } from '../stores/connections.svelte.ts';

export type { DriverDescriptor };

/**
 * Driver descriptors are baked into the binary, so once fetched they never go
 * stale. `staleTime: Infinity` means the form can ask for them on every mount
 * and pay for exactly one IPC call per app run.
 */
export function driversOptions() {
  return queryOptions({
    queryKey: qk.drivers(),
    queryFn: () => listDrivers() as Promise<DriverDescriptor[]>,
    staleTime: Infinity,
  });
}

/** Component-facing constructor. Call during component init. */
export function driversQuery() {
  return createQuery(() => driversOptions());
}
