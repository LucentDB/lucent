// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  driftAlertsFor,
  initMemoryDriftListeners,
  recordDriftAlerts,
  removeDriftAlert,
  __resetForTests,
} from './memoryDrift.svelte';
import type { DriftAlert } from '../ipc/ai.ts';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, handler: (e: { payload: unknown }) => void) => {
    (globalThis as any).__listeners[event] = handler;
    return Promise.resolve(() => {});
  }),
}));

function alert(overrides: Partial<DriftAlert> = {}): DriftAlert {
  return {
    connection_key: 'conn-a',
    memory_id: 'm1',
    rule_text: 'active users',
    reason: "Table 'public.users' was dropped",
    schema_name: 'public',
    table_name: 'users',
    column_name: null,
    ...overrides,
  };
}

beforeEach(() => {
  (globalThis as any).__listeners = {};
  __resetForTests();
});

describe('memoryDrift store (B-C5)', () => {
  it('routes a memory:drift_detected event to the matching connection', async () => {
    await initMemoryDriftListeners();
    const handler = (globalThis as any).__listeners['memory:drift_detected'];
    handler({ payload: [alert({ connection_key: 'conn-a' })] });

    expect(driftAlertsFor('conn-a').map((a) => a.memory_id)).toEqual(['m1']);
    expect(driftAlertsFor('conn-b')).toEqual([]);
  });

  it('merges repeated events and dedupes by memory id', async () => {
    await initMemoryDriftListeners();
    const handler = (globalThis as any).__listeners['memory:drift_detected'];
    handler({ payload: [alert()] });
    handler({ payload: [alert({ connection_key: 'conn-b' })] });
    handler({ payload: [alert({ reason: 'updated reason' })] });

    expect(driftAlertsFor('conn-a')).toHaveLength(1);
    expect(driftAlertsFor('conn-a')[0].reason).toBe('updated reason');
    expect(driftAlertsFor('conn-b')).toHaveLength(1);
  });

  it('ignores alerts without a connection key', async () => {
    await initMemoryDriftListeners();
    const handler = (globalThis as any).__listeners['memory:drift_detected'];
    handler({ payload: [alert({ connection_key: undefined })] });

    expect(driftAlertsFor('conn-a')).toEqual([]);
  });

  it('removeDriftAlert forgets a single row and prunes the bucket', () => {
    recordDriftAlerts('conn-a', [
      alert({ memory_id: 'm1' }),
      alert({ memory_id: 'm2' }),
    ]);
    expect(driftAlertsFor('conn-a')).toHaveLength(2);

    removeDriftAlert('conn-a', 'm1');
    expect(driftAlertsFor('conn-a').map((a) => a.memory_id)).toEqual(['m2']);

    removeDriftAlert('conn-a', 'm2');
    expect(driftAlertsFor('conn-a')).toEqual([]);
  });
});
