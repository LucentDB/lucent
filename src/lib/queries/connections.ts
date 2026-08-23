import {
  createMutation,
  createQuery,
  queryOptions,
  useQueryClient,
  type QueryClient,
} from '@tanstack/svelte-query';
import {
  listConnections,
  saveConnection as ipcSaveConnection,
  deleteConnection as ipcDeleteConnection,
  duplicateConnection as ipcDuplicateConnection,
} from '../ipc/client.js';
import { qk } from './keys.ts';
import type { ConnectionProfile } from '../stores/connections.svelte.ts';

export type { ConnectionProfile };

// client.js is untyped; its default parameter (`password = null`) makes TS
// infer the second argument as `null` only. Widen it once, here, instead of
// casting at every call site.
const ipcSave = ipcSaveConnection as (
  profile: ConnectionProfile,
  password: string | null,
) => Promise<unknown>;

export function connectionsOptions() {
  return queryOptions({
    queryKey: qk.connections(),
    queryFn: () => listConnections() as Promise<ConnectionProfile[]>,
  });
}

/** Component-facing constructor. Call during component init. */
export function connectionsQuery() {
  return createQuery(() => connectionsOptions());
}

// ─── Imperative mutation helpers ────────────────────────────────────────────
// Exported as plain functions taking an explicit client so they are callable
// from tests and from non-component code paths. The `*Mutation()` wrappers
// below are the component-facing form.

export interface SaveVars {
  profile: ConnectionProfile;
  password?: string | null;
}

export async function saveConnection(
  client: QueryClient,
  vars: SaveVars,
): Promise<ConnectionProfile> {
  const saved = (await ipcSave(
    vars.profile,
    vars.password ?? null,
  )) as ConnectionProfile;
  await client.invalidateQueries({ queryKey: qk.connections() });
  return saved;
}

export async function deleteConnection(
  client: QueryClient,
  vars: { id: string },
): Promise<void> {
  await ipcDeleteConnection(vars.id);
  await client.invalidateQueries({ queryKey: qk.connections() });
}

export async function duplicateConnection(
  client: QueryClient,
  vars: { id: string },
): Promise<ConnectionProfile> {
  const copy = (await ipcDuplicateConnection(vars.id)) as ConnectionProfile;
  await client.invalidateQueries({ queryKey: qk.connections() });
  return copy;
}

// ─── Component-facing mutations ─────────────────────────────────────────────

export function saveConnectionMutation() {
  const client = useQueryClient();
  return createMutation(() => ({
    mutationFn: (vars: SaveVars) => saveConnection(client, vars),
  }));
}

export function deleteConnectionMutation() {
  const client = useQueryClient();
  return createMutation(() => ({
    mutationFn: (vars: { id: string }) => deleteConnection(client, vars),
  }));
}

export function duplicateConnectionMutation() {
  const client = useQueryClient();
  return createMutation(() => ({
    mutationFn: (vars: { id: string }) => duplicateConnection(client, vars),
  }));
}
