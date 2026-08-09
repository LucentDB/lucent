import { describe, it, expect } from 'vitest';
import { badgeState } from './ReadOnlyBadge.svelte';

describe('badgeState', () => {
  it('is quiet when the engine enforces read-only', () => {
    const state = badgeState({
      driver: 'postgres',
      displayName: 'PostgreSQL',
      engineEnforcedReadonly: true,
      readonlyDisclosure: null,
    });
    expect(state.tone).toBe('neutral');
    expect(state.label).toBe('PostgreSQL');
    expect(state.warning).toBeNull();
  });

  it('warns, and says why, when only the SQL guard protects the database', () => {
    const state = badgeState({
      driver: 'duckdb',
      displayName: 'DuckDB',
      engineEnforcedReadonly: false,
      readonlyDisclosure:
        'Read-only is NOT enforced by this database engine. Lucent’s SQL guard is the only protection.',
    });
    expect(state.tone).toBe('warning');
    expect(state.label).toBe('DuckDB');
    expect(state.warning).toContain('NOT enforced');
  });

  it('shows nothing at all when disconnected', () => {
    expect(badgeState(null)).toEqual({
      tone: 'hidden',
      label: '',
      warning: null,
    });
  });
});
