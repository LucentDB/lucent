/**
 * Tool-call bookkeeping for notebook AI cells.
 *
 * A call is announced before it runs and reported after, and the two halves
 * rarely agree. On the ACP CLI path the agent announces an opaque shell
 * command — a free-text title, no arguments — and only the bridge, which
 * actually executed the tool, knows the real name and input. So a result
 * event repairs what the announcement could not know, and only that.
 */

import { argsMissing } from './tool-args.ts';

/** A tool call as it lives in `ai_state.tool_calls` — untyped in the store. */
export type CellToolCall = Record<string, unknown> & { id?: unknown };

/** The payload of a `tool_result` notebook event. */
export interface ToolResultUpdate {
  id: string;
  /** The name the bridge ran the tool under. Empty when unknown. */
  tool: string;
  summary: string;
  /** Arguments the tool really ran with — the only source on the CLI path. */
  input?: unknown;
  output?: unknown;
}

/** Tool ids are snake_case identifiers; an agent's free-text title is not. */
const TOOL_ID = /^[a-z][a-z0-9_]*$/;

function isToolId(v: unknown): boolean {
  return typeof v === 'string' && TOOL_ID.test(v);
}

/**
 * Applies one tool result to the matching call, returning a new list. Fills
 * only what the announcement lacked — a name the agent really reported and
 * arguments it really sent are never overwritten by the bridge's view.
 */
export function mergeToolResult(
  calls: readonly CellToolCall[],
  update: ToolResultUpdate,
): CellToolCall[] {
  return calls.map((call) => {
    if (call.id !== update.id) return call;
    return {
      ...call,
      name: update.tool && !isToolId(call.name) ? update.tool : call.name,
      args: argsMissing(call.args) ? (update.input ?? call.args) : call.args,
      summary: update.summary,
      output: update.output,
    };
  });
}
