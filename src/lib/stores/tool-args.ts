/**
 * Shared by the chat pane and notebook AI cells, which both render a tool
 * card built from two halves: what the agent announced before the call ran,
 * and what the bridge reported after it did.
 *
 * An announcement can arrive with `raw_input: {}` rather than with nothing at
 * all — ACP agents do this routinely, announcing the call before the arguments
 * are settled. Guards written as `args === null` read that as "the agent told
 * us its arguments, and they were empty" and refuse to backfill, which is why
 * notebook tool cards rendered `INPUT {}` beside an output full of rows. An
 * empty object carries no more information than a null, and this says so.
 */

/**
 * Whether a call's announced arguments tell us nothing, and a result that
 * carries some may therefore fill them in. Null, undefined, an empty object,
 * an empty array and a blank string all mean the same thing here.
 */
export function argsMissing(args: unknown): boolean {
  if (args === null || args === undefined) return true;
  if (typeof args === 'string') return args.trim().length === 0;
  if (Array.isArray(args)) return args.length === 0;
  if (typeof args === 'object') return Object.keys(args).length === 0;
  return false;
}
