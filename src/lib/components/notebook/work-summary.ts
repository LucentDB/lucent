/**
 * One line describing what an AI cell actually did.
 *
 * The header used to carry "1 tool call · 68.0s", right-aligned: a count of
 * calls says nothing about the work, and putting it at the far edge of the bar
 * read as run metadata rather than as the record of what the model did. This
 * names the tools instead — "Ran 1 query · thought for 25s" — so the collapsed
 * state is still informative.
 */

import { isThinking } from '../../stores/notebook-thinking.ts';

/** A tool call as it lives in `ai_state.tool_calls`. Only the field this
 *  reads is named, so both the typed chat card and the store's loose JSON
 *  satisfy it. */
interface ToolCallLike {
  name?: unknown;
}

/**
 * Verb phrases for Lucent's own tools, as functions of how many times each
 * ran. Written lowercase because they are sentence fragments — only whichever
 * lands first gets capitalised. Anything else is an agent's free-text title
 * (ACP carries no tool name) and is shown as given.
 */
const PHRASES: Record<string, (n: number) => string> = {
  run_readonly_query: (n) => `ran ${n} ${n === 1 ? 'query' : 'queries'}`,
  get_objects_info: () => 'inspected schema',
  search_objects: () => 'searched schema',
  preview_dml: (n) => `previewed ${n} ${n === 1 ? 'change' : 'changes'}`,
};

/**
 * A phrase, plus whether it is a name rather than a fragment. A name — an
 * agent's own title for the tool — keeps the case it was given wherever it
 * lands in the sentence; a fragment is capitalised only when it comes first.
 */
interface Phrase {
  text: string;
  isName: boolean;
}

/** Tool ids are snake_case identifiers; an agent's free-text title is not. */
const TOOL_ID = /^[a-z][a-z0-9_]*$/;

function phraseFor(name: string, n: number): Phrase {
  const known = PHRASES[name];
  if (known) return { text: known(n), isName: false };
  // An unknown tool id still reads better with its underscores spent, but a
  // free-text title (`psql -c "select 1"`) must survive verbatim.
  const label = TOOL_ID.test(name)
    ? name.charAt(0).toUpperCase() + name.slice(1).replace(/_/g, ' ')
    : name;
  return { text: n === 1 ? label : `${label} ×${n}`, isName: true };
}

/** Seconds, rounded as ThinkingCard rounds them, so the two never disagree. */
function seconds(ms: number): number {
  return Math.max(1, Math.round(ms / 1000));
}

/**
 * A sentence naming each tool the cell used, in the order it first used them,
 * followed by the total time spent reasoning. Empty when the cell has nothing
 * to report yet — the caller renders no strip at all in that case.
 */
export function workSummary(
  toolCalls: readonly ToolCallLike[],
  messages: readonly unknown[],
): string {
  // Insertion-ordered, so the phrases follow the order the model worked in.
  const counts = new Map<string, number>();
  for (const call of toolCalls) {
    const name = typeof call?.name === 'string' ? call.name : '';
    if (!name) continue;
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  const parts = [...counts].map(([name, n]) => phraseFor(name, n));

  const thinkingMs = messages
    .filter(isThinking)
    .reduce((total, m) => total + (m.durationMs ?? 0), 0);
  if (thinkingMs > 0) {
    parts.push({ text: `thought for ${seconds(thinkingMs)}s`, isName: false });
  }

  if (parts.length === 0) return '';
  const [first, ...rest] = parts;
  const opening = first.isName
    ? first.text
    : first.text.charAt(0).toUpperCase() + first.text.slice(1);
  return [opening, ...rest.map((p) => p.text)].join(' · ');
}
