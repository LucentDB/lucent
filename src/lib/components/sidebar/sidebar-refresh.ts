export interface ExplorerDatabase {
  name: string;
  is_current: boolean;
  [key: string]: unknown;
}

export interface ExplorerSchema {
  name: string;
  path: unknown;
  [key: string]: unknown;
}

export interface ExplorerFetchers {
  getDatabases: () => Promise<ExplorerDatabase[]>;
  getSchemas: () => Promise<ExplorerSchema[]>;
  getSchemaObjects: (namespace: unknown) => Promise<{ objects: unknown[] }>;
}

export interface ExplorerSnapshot {
  databases: ExplorerDatabase[];
  schemasByDb: Record<string, ExplorerSchema[]>;
  objectsBySchema: Record<string, unknown[]>;
}

/**
 * Fetches a complete catalog snapshot for the active connection.
 *
 * Nothing is returned until every schema and its objects have been fetched, so
 * callers can commit the snapshot atomically and keep the old tree intact if a
 * refresh fails halfway through.
 */
export async function fetchExplorerSnapshot(
  fetchers: ExplorerFetchers,
): Promise<ExplorerSnapshot> {
  const databases = await fetchers.getDatabases();
  const currentDatabases = databases.filter((database) => database.is_current);
  const schemasByDb: Record<string, ExplorerSchema[]> = {};
  const objectsBySchema: Record<string, unknown[]> = {};

  if (currentDatabases.length === 0) {
    return { databases, schemasByDb, objectsBySchema };
  }

  // The catalog IPC is scoped to the active connection, so one schema request
  // describes the current database tree even when the driver reports several
  // database labels.
  const schemas = await fetchers.getSchemas();
  for (const database of currentDatabases) {
    schemasByDb[database.name] = schemas;
  }

  const objectEntries = await Promise.all(
    schemas.map(async (schema) => {
      const result = await fetchers.getSchemaObjects(schema.path);
      return [schema.name, result.objects] as const;
    }),
  );
  for (const [schemaName, objects] of objectEntries) {
    objectsBySchema[schemaName] = objects;
  }

  return { databases, schemasByDb, objectsBySchema };
}
