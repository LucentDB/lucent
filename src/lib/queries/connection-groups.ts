import type { ConnectionProfile } from '../stores/connections.svelte.ts';

export interface ProfileGroup {
  /** Group label, or '' for the ungrouped bucket. */
  name: string;
  profiles: ConnectionProfile[];
}

/**
 * Buckets profiles by their `group` field. Named groups sort alphabetically
 * and the ungrouped bucket sorts last, matching the previous store-derived
 * behaviour exactly.
 */
export function groupProfiles(profiles: ConnectionProfile[]): ProfileGroup[] {
  const grouped = new Map<string, ConnectionProfile[]>();

  for (const p of profiles) {
    const key = p.group ?? '__ungrouped__';
    if (!grouped.has(key)) grouped.set(key, []);
    grouped.get(key)!.push(p);
  }

  const groups: ProfileGroup[] = [];
  for (const [key, list] of grouped) {
    groups.push({ name: key === '__ungrouped__' ? '' : key, profiles: list });
  }

  groups.sort((a, b) => {
    if (a.name === '' && b.name === '') return 0;
    if (a.name === '') return 1;
    if (b.name === '') return -1;
    return a.name.localeCompare(b.name);
  });

  return groups;
}
