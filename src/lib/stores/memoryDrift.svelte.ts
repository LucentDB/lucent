import { listen } from '@tauri-apps/api/event';
import type { DriftAlert } from '../ipc/ai.ts';

/**
 * Genuine Gate 1 drift alerts pushed by the background schema indexer
 * (`memory:drift_detected`), keyed by the frontend memory connection key
 * (profile id or `host:port/database`).
 *
 * `MemoryDrawer.refresh()` prefers these over its synthesized fallback (F-I2),
 * so the real reason a rule was invalidated — e.g. "Table 'public.users' was
 * dropped" — reaches the user instead of a generic sentence (B-C5). A backend
 * event therefore connects the indexer's cascade to the drawer without the user
 * having to press "Consolidate".
 */
export const memoryDrift = $state({
  byConnection: new Map<string, DriftAlert[]>(),
});

/** Store (or merge) genuine alerts for one connection, deduping by memory id. */
export function recordDriftAlerts(
  connectionKey: string,
  alerts: DriftAlert[],
): void {
  if (!connectionKey || alerts.length === 0) return;
  const merged = new Map(
    (memoryDrift.byConnection.get(connectionKey) ?? []).map((a) => [
      a.memory_id,
      a,
    ]),
  );
  for (const alert of alerts) merged.set(alert.memory_id, alert);
  memoryDrift.byConnection.set(connectionKey, [...merged.values()]);
}

/** Alerts currently retained for a connection (empty when none). */
export function driftAlertsFor(connectionKey: string): DriftAlert[] {
  return memoryDrift.byConnection.get(connectionKey) ?? [];
}

/** Forget a single alert (e.g. after the rule is deleted or the drift resolved). */
export function removeDriftAlert(connectionKey: string, memoryId: string): void {
  const existing = memoryDrift.byConnection.get(connectionKey);
  if (!existing) return;
  const remaining = existing.filter((a) => a.memory_id !== memoryId);
  if (remaining.length === 0) {
    memoryDrift.byConnection.delete(connectionKey);
  } else {
    memoryDrift.byConnection.set(connectionKey, remaining);
  }
}

/** Forget every retained alert for a connection. */
export function clearDriftAlerts(connectionKey: string): void {
  memoryDrift.byConnection.delete(connectionKey);
}

/**
 * Register the backend `memory:drift_detected` listener. Called once from App's
 * `onMount`, alongside `initIndexingListeners`. Each alert carries its own
 * `connection_key`, so alerts for a background connection are retained under the
 * right key even when a different connection is open as they arrive.
 */
export async function initMemoryDriftListeners(): Promise<void> {
  await listen<DriftAlert[]>('memory:drift_detected', (event) => {
    for (const alert of event.payload ?? []) {
      const key = alert.connection_key;
      if (key) recordDriftAlerts(key, [alert]);
    }
  });
}

export function __resetForTests(): void {
  memoryDrift.byConnection.clear();
}
