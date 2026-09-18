// Pure search-matching helpers for the schema tree.
//
// The tree is databases → schemas → objects, with objects loaded lazily per
// schema. During an active search we only match against data that is loaded;
// a node whose children are not yet loaded matches only by its own name.

const lowerCache = new WeakMap<object, string>();

function getLowerName(nameOrObj: string | { name: string }): string {
  if (typeof nameOrObj === 'string') {
    return nameOrObj.toLowerCase();
  }
  let cached = lowerCache.get(nameOrObj);
  if (!cached) {
    cached = nameOrObj.name.toLowerCase();
    lowerCache.set(nameOrObj, cached);
  }
  return cached;
}

export function objectMatches(
  nameOrObj: string | { name: string },
  queryLower: string,
): boolean {
  if (!queryLower) return true;
  return getLowerName(nameOrObj).includes(queryLower);
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
  dbName: string | { name: string },
  schemas: { name: string }[] | undefined,
  objectsBySchema: Record<string, { name: string }[]>,
  queryLower: string,
): boolean {
  if (!queryLower) return true;
  if (objectMatches(dbName, queryLower)) return true;
  if (!schemas) return false;
  return schemas.some((s) =>
    schemaMatches(s, objectsBySchema[s.name], queryLower),
  );
}
