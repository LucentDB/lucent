// Pure search-matching helpers for the schema tree.
//
// The tree is databases → schemas → objects, with objects loaded lazily per
// schema. During an active search we only match against data that is loaded;
// a node whose children are not yet loaded matches only by its own name.

export function objectMatches(name: string, queryLower: string): boolean {
  if (!queryLower) return true;
  return name.toLowerCase().includes(queryLower);
}

export function schemaMatches(
  schema: { name: string },
  objects: { name: string }[] | undefined,
  queryLower: string,
): boolean {
  if (!queryLower) return true;
  if (objectMatches(schema.name, queryLower)) return true;
  if (!objects) return false;
  return objects.some((o) => objectMatches(o.name, queryLower));
}

export function dbMatches(
  dbName: string,
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
