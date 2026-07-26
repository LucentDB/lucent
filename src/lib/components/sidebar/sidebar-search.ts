// Pure search-matching helpers for the schema tree.
//
// The tree is databases → schemas → objects, with objects loaded lazily per
// schema. During an active search we only match against data that is loaded;
// a node whose children are not yet loaded matches only by its own name.

export function objectMatches(name: string, query: string): boolean {
  if (!query) return true;
  return name.toLowerCase().includes(query.toLowerCase());
}

export function schemaMatches(
  schema: { name: string },
  objects: { name: string }[] | undefined,
  query: string,
): boolean {
  if (!query) return true;
  if (objectMatches(schema.name, query)) return true;
  if (!objects) return false;
  return objects.some((o) => objectMatches(o.name, query));
}

export function dbMatches(
  dbName: string,
  schemas: { name: string }[] | undefined,
  objectsBySchema: Record<string, { name: string }[]>,
  query: string,
): boolean {
  if (!query) return true;
  if (objectMatches(dbName, query)) return true;
  if (!schemas) return false;
  return schemas.some((s) => schemaMatches(s, objectsBySchema[s.name], query));
}
