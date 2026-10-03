// Pure search-matching helpers for the schema tree.
//
// The tree is databases → schemas → objects, with objects loaded lazily per
// schema. During an active search we only match against data that is loaded;
// a node whose children are not yet loaded matches only by its own name.

// WeakMap cache to store lowercased names by object identity,
// avoiding repeated String.prototype.toLowerCase() allocations per keystroke/render cycle.
const lowerNameCache = new WeakMap<object, string>();

function getLowerName(item: string | { name: string }): string {
  if (typeof item === 'object' && item !== null) {
    let lower = lowerNameCache.get(item);
    if (!lower) {
      lower = item.name.toLowerCase();
      lowerNameCache.set(item, lower);
    }
    return lower;
  }
  return item.toLowerCase();
}

export function objectMatches(
  item: string | { name: string },
  queryLower: string,
): boolean {
  if (!queryLower) return true;
  return getLowerName(item).includes(queryLower);
}

export function schemaMatches(
  schema: { name: string },
  objects: { name: string }[] | undefined,
  queryLower: string,
): boolean {
  if (!queryLower) return true;
  if (objectMatches(schema, queryLower)) return true;
  if (!objects) return false;
  return objects.some((o) => objectMatches(o, queryLower));
}

export function dbMatches(
  db: string | { name: string },
  schemas: { name: string }[] | undefined,
  objectsBySchema: Record<string, { name: string }[]>,
  queryLower: string,
): boolean {
  if (!queryLower) return true;
  if (objectMatches(db, queryLower)) return true;
  if (!schemas) return false;
  return schemas.some((s) =>
    schemaMatches(s, objectsBySchema[s.name], queryLower),
  );
}
