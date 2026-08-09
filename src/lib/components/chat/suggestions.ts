// Builds the AI landing screen's suggestion chips from a summary of the
// connected schema. Kept free of Svelte and IPC so the interesting logic —
// which prompts get offered, and what happens when the schema is unknown —
// is unit-testable without mounting a component.

export interface SchemaTable {
  name: string;
  rowCount: number | null;
}

export interface SchemaSummary {
  /** Null when no schema with tables was found. */
  schema: string | null;
  tables: SchemaTable[];
}

export interface Suggestion {
  /** Short text shown on the chip. May be truncated for display. */
  label: string;
  /** Icon name understood by Icon.svelte. */
  icon: string;
  /** The full text actually sent to the agent when clicked. */
  prompt: string;
}

/** Maximum chips shown. More than this turns the landing into a menu. */
const MAX_SUGGESTIONS = 4;

/** Chip labels longer than this get an ellipsis; the prompt stays intact. */
const MAX_LABEL_LENGTH = 42;

/**
 * Prompts that work regardless of what's in the schema. These are the floor:
 * offered always, and the entire set when the schema is unknown (still
 * loading, load failed, or genuinely empty).
 */
const GENERIC_SUGGESTIONS: readonly Suggestion[] = [
  {
    label: 'Which tables have the most rows?',
    icon: 'trending',
    prompt: 'Which tables have the most rows?',
  },
  {
    label: 'Give me an overview of this schema',
    icon: 'table',
    prompt:
      'Give me an overview of this database — the main tables, what they ' +
      'hold, and how they connect.',
  },
];

function truncateLabel(text: string): string {
  return text.length > MAX_LABEL_LENGTH
    ? text.slice(0, MAX_LABEL_LENGTH - 1).trimEnd() + '…'
    : text;
}

/**
 * Tables ordered largest first. `get_schema_objects` COALESCEs the
 * `pg_stat_user_tables` estimate to 0, so a null count is defensive rather
 * than expected — either way an un-analyzed table sorts last but is never
 * dropped, since it's still a real table worth asking about.
 */
function byRowCountDesc(tables: SchemaTable[]): SchemaTable[] {
  return [...tables].sort((a, b) => (b.rowCount ?? -1) - (a.rowCount ?? -1));
}

/**
 * Grounded suggestions for a schema, falling back to generic ones when there's
 * nothing to ground them in. Never returns an empty array — an empty
 * suggestions block would read as a broken screen.
 */
export function buildSuggestions(summary: SchemaSummary | null): Suggestion[] {
  const tables = summary?.tables ?? [];
  if (tables.length === 0) return [...GENERIC_SUGGESTIONS];

  const ranked = byRowCountDesc(tables);
  const schema = summary?.schema ?? null;

  // Prompts name `schema.table` when the tables aren't in `public`, so the
  // agent doesn't have to guess where to look. Labels stay unqualified —
  // the chip has no room, and the user already knows their own database.
  const qualify = (table: string) =>
    schema && schema !== 'public' ? `${schema}.${table}` : table;

  const grounded: Suggestion[] = [];
  const largest = ranked[0];

  grounded.push({
    label: truncateLabel(`Summarize the ${largest.name} table`),
    icon: 'chart',
    prompt:
      `Summarize the ${qualify(largest.name)} table — what does each column ` +
      `hold, and how many rows are there?`,
  });

  if (ranked.length >= 2) {
    const second = ranked[1];
    grounded.push({
      label: truncateLabel(`How do ${largest.name} and ${second.name} relate?`),
      icon: 'search',
      prompt:
        `How do the ${qualify(largest.name)} and ${qualify(second.name)} ` +
        `tables relate? Show me the foreign keys and an example join.`,
    });
  }

  return [...grounded, ...GENERIC_SUGGESTIONS].slice(0, MAX_SUGGESTIONS);
}

/**
 * What the copilot can do, shown on the disconnected screen in place of
 * suggestions. Deliberately names no tables — there is no schema to name.
 */
export const CAPABILITIES: readonly { icon: string; text: string }[] = [
  { icon: 'search', text: 'Explore your schema and explain how tables relate' },
  { icon: 'query', text: 'Write and run SQL from a plain-English question' },
  { icon: 'chart', text: 'Summarize results and follow up on what it finds' },
];
