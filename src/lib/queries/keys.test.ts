import { describe, it, expect } from 'vitest';
import { qk } from './keys.ts';

describe('qk', () => {
  it('gives explorer branch keys a shared prefix so one invalidate hits all', () => {
    const conn = 'profile-1';
    const prefix = qk.explorer(conn);
    for (const key of [
      qk.databases(conn),
      qk.schemas(conn),
      qk.objects(conn, ['public']),
    ]) {
      expect(key.slice(0, prefix.length)).toEqual([...prefix]);
    }
  });

  it('separates explorer caches per connection', () => {
    expect(qk.databases('a')).not.toEqual(qk.databases('b'));
  });

  it('builds a stable history key from an equal filter', () => {
    const filter = {
      connectionId: null,
      search: 'select',
      favoriteOnly: false,
    };
    expect(qk.history({ ...filter })).toEqual(qk.history({ ...filter }));
  });

  it('distinguishes history keys by filter', () => {
    const base = { connectionId: null, search: null, favoriteOnly: false };
    expect(qk.history(base)).not.toEqual(
      qk.history({ ...base, favoriteOnly: true }),
    );
  });

  it('keeps drivers and connections on distinct roots', () => {
    expect(qk.drivers()[0]).not.toBe(qk.connections()[0]);
  });
});
