import { describe, it, expect } from 'vitest';
import { groupProfiles } from './connection-groups.ts';

/** Minimal profile — grouping only reads `group`. */
function p(id: string, group: string | null) {
  return { id, name: id, group } as never;
}

describe('groupProfiles', () => {
  it('sorts named groups alphabetically', () => {
    const groups = groupProfiles([p('a', 'prod'), p('b', 'dev')]);
    expect(groups.map((g) => g.name)).toEqual(['dev', 'prod']);
  });

  it('puts ungrouped profiles last under an empty name', () => {
    const groups = groupProfiles([p('a', null), p('b', 'prod')]);
    expect(groups.map((g) => g.name)).toEqual(['prod', '']);
  });

  it('keeps profile order within a group', () => {
    const groups = groupProfiles([p('a', 'x'), p('b', 'x')]);
    expect(groups[0].profiles.map((q) => q.id)).toEqual(['a', 'b']);
  });

  it('returns an empty array for no profiles', () => {
    expect(groupProfiles([])).toEqual([]);
  });
});
