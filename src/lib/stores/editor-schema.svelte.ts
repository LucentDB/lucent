import { getEditorSchema } from '../ipc/client.js';

export interface EditorColumn {
  name: string;
  type_name: string;
}

export interface EditorTable {
  schema: string;
  name: string;
  columns: EditorColumn[];
}

class EditorSchemaStore {
  tables = $state<EditorTable[]>([]);
  loaded = $state(false);
  /**
   * Monotonic request generation. refresh() only commits its result when it
   * is still the latest request — a superseded (slow) response must never
   * overwrite the active connection's schema with stale tables.
   */
  private generation = 0;

  async refresh() {
    const generation = ++this.generation;
    try {
      const tables = await getEditorSchema();
      if (generation === this.generation) this.tables = tables;
    } catch {
      // Decorative-feature failure mode, matching schema-summary.svelte.ts:
      // autocomplete degrades to keyword-only, never surfaces an error. A
      // stale failure must not wipe a newer success either.
      if (generation === this.generation) this.tables = [];
    } finally {
      if (generation === this.generation) this.loaded = true;
    }
  }
}

export const editorSchema = new EditorSchemaStore();
