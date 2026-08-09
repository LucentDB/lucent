// Formatting for the AI landing screen's "pick up where you left off" block.
// Pure functions over HistoryEntry so the display logic is testable without
// mounting a component or reaching for IPC.

import type { HistoryEntry } from '../../stores/history.svelte.ts';

/**
 * How many entries to show. The same in both layouts — the side panel is tall
 * enough that trimming to two only bought empty space.
 */
export const RECENT_LIMIT = 3;

/** SQL excerpts longer than this are ellipsised. */
const MAX_EXCERPT_LENGTH = 44;

/**
 * Collapses a multi-line SQL statement into a single-line excerpt. History
 * entries hold formatted SQL with newlines and runs of indentation, which
 * would otherwise blow out the row height or render as a ragged fragment.
 */
export function excerptSql(sql: string): string {
  const flat = sql.replace(/\s+/g, ' ').trim();
  return flat.length > MAX_EXCERPT_LENGTH
    ? flat.slice(0, MAX_EXCERPT_LENGTH - 1).trimEnd() + '…'
    : flat;
}

/** Leading statement keywords, so the excerpt can emphasise the verb. */
const SQL_VERB =
  /^(with|select|insert|update|delete|create|alter|drop|truncate|explain|analyze|begin|commit|rollback|grant|revoke|copy|vacuum)\b/i;

/**
 * Splits an excerpt into its leading SQL verb and the remainder, so the verb
 * can be weighted differently. Returns a null verb when the statement doesn't
 * start with a recognised keyword, in which case the whole string is `rest`.
 */
export function splitExcerpt(excerpt: string): {
  verb: string | null;
  rest: string;
} {
  const match = excerpt.match(SQL_VERB);
  if (!match) return { verb: null, rest: excerpt };
  return {
    verb: match[0].toUpperCase(),
    // Trimmed because the two parts are separated by a flex gap, not by this
    // whitespace — keeping it would double the space after the verb.
    rest: excerpt.slice(match[0].length).trim(),
  };
}

/**
 * Compact relative time. `now` is injectable so tests don't depend on the
 * clock. Returns an empty string for unparseable timestamps rather than
 * throwing or rendering "NaN ago".
 */
export function relativeTime(iso: string, now: number = Date.now()): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return '';

  const seconds = Math.max(0, Math.round((now - then) / 1000));
  if (seconds < 60) return 'just now';

  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;

  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;

  const days = Math.round(hours / 24);
  if (days === 1) return 'yesterday';
  if (days < 7) return `${days}d ago`;

  const weeks = Math.round(days / 7);
  return weeks === 1 ? '1w ago' : `${weeks}w ago`;
}

/**
 * Row counts as `1.2k` / `3.4M` so the meta line stays short. Returns null for
 * anything that isn't a finite number — a missing field must render as nothing
 * rather than "NaN rows".
 */
export function formatRowCount(count: unknown): string | null {
  if (typeof count !== 'number' || !Number.isFinite(count)) return null;
  if (count < 1000) return `${count}`;
  if (count < 1_000_000) {
    const k = count / 1000;
    return `${k < 10 ? k.toFixed(1).replace(/\.0$/, '') : Math.round(k)}k`;
  }
  const m = count / 1_000_000;
  return `${m < 10 ? m.toFixed(1).replace(/\.0$/, '') : Math.round(m)}M`;
}

/** Query duration, omitted when the field is missing or implausible. */
export function formatDuration(ms: unknown): string | null {
  if (typeof ms !== 'number' || !Number.isFinite(ms) || ms < 0) return null;
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const s = ms / 1000;
  return `${s < 10 ? s.toFixed(1).replace(/\.0$/, '') : Math.round(s)}s`;
}

/**
 * Collapses entries that ran the same statement. Browsing a table re-runs an
 * identical query on every page, so raw history is often the same SQL three
 * times over — three identical rows read as a rendering bug, not as history.
 * The newest occurrence wins, since history arrives newest-first.
 */
export function dedupeBySql(entries: HistoryEntry[]): HistoryEntry[] {
  const seen = new Set<string>();
  const out: HistoryEntry[] = [];
  for (const e of entries) {
    const key = e.sql.replace(/\s+/g, ' ').trim().toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(e);
  }
  return out;
}

/**
 * The muted meta line under an entry's excerpt: outcome, duration, age. A
 * failed query is surfaced as "failed" rather than hidden — a query that
 * errored is often exactly what you want the copilot's help with. Every part
 * is dropped when unavailable, so a missing field shortens the line instead of
 * poisoning it.
 */
export function describeEntry(entry: HistoryEntry, now?: number): string {
  const parts: string[] = [];

  if (entry.status === 'error') {
    parts.push('failed');
  } else {
    const rows = formatRowCount(entry.rowCount);
    parts.push(
      rows === null ? 'ran' : `${rows} ${rows === '1' ? 'row' : 'rows'}`,
    );
  }

  const duration = formatDuration(entry.durationMs);
  if (duration) parts.push(duration);

  const when = relativeTime(entry.executedAt, now);
  if (when) parts.push(when);

  return parts.join(' · ');
}

/**
 * The message sent when an entry is clicked. This is the AI surface, so a
 * click asks the copilot about the query rather than re-running it — the
 * editor already owns re-running.
 */
export function explainPrompt(entry: HistoryEntry): string {
  const intro =
    entry.status === 'error'
      ? 'This query failed. Explain why and give me a corrected version:'
      : 'Explain what this query does and suggest any improvements:';
  const error = entry.error ? `\n\nThe error was: ${entry.error}` : '';
  return `${intro}\n\n\`\`\`sql\n${entry.sql.trim()}\n\`\`\`${error}`;
}
