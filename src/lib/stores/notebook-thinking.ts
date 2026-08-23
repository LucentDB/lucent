/**
 * Thinking-segment bookkeeping for notebook AI cells.
 *
 * An AI cell's reasoning arrives as a stream of chunks broken up by tool
 * calls: think → call → think → call → … Each stretch is its own card with
 * its own duration, so each one has to be timed from when *it* started to
 * when *it* ended.
 *
 * The previous inline version stamped one start time (the cell's) on every
 * segment and then, when the run finished, set every segment's duration to
 * `now - thatOneStart`. Every card showed the whole cell's runtime: a
 * one-sentence closing thought claimed the same 89 seconds as the long
 * opening analysis.
 */

/** A thinking message as it lives in `ai_state.messages`. */
export type ThinkingMessage = {
  thinking: string;
  /** Wall-clock ms when this segment started. Removed once it closes. */
  _startedAt?: number;
  /** How long this segment ran. Present once the segment has closed. */
  durationMs?: number;
};

/** `ai_state.messages` is untyped in the store; narrowing stays local. */
export type CellMessage = unknown;

export function isThinking(m: unknown): m is ThinkingMessage {
  return (
    typeof m === 'object' &&
    m !== null &&
    'thinking' in m &&
    typeof (m as ThinkingMessage).thinking === 'string'
  );
}

/** A segment is open until its duration is stamped. */
function isOpen(m: CellMessage): boolean {
  return isThinking(m) && m.durationMs === undefined;
}

/**
 * Adds a streamed chunk: appended to the open segment when one is streaming,
 * otherwise it starts a new segment timed from `now`.
 */
export function appendThinkingChunk(
  messages: CellMessage[],
  chunk: string,
  streaming: boolean,
  now: number,
): CellMessage[] {
  const last = messages.at(-1);
  if (streaming && last && isOpen(last)) {
    const merged: ThinkingMessage = {
      ...(last as ThinkingMessage),
      thinking: (last as ThinkingMessage).thinking + chunk,
    };
    return [...messages.slice(0, -1), merged];
  }
  return [...messages, { thinking: chunk, _startedAt: now }];
}

/**
 * Closes the open segment, if any, stamping how long it ran. Segments that
 * already closed keep the duration they were given — that is the whole point:
 * the reasoning before a tool call is timed separately from the reasoning
 * after it.
 *
 * Called when a tool call interrupts the reasoning and again when the run
 * ends. Idempotent, so the second call on an already-closed tail is a no-op.
 */
export function closeOpenThinking(
  messages: CellMessage[],
  now: number,
): CellMessage[] {
  const i = messages.findIndex(isOpen);
  if (i === -1) return messages;
  const open = messages[i] as ThinkingMessage;
  const { _startedAt, ...rest } = open;
  const closed: ThinkingMessage = {
    ...rest,
    // A missing start can only mean a chunk arrived before `thinking_started`;
    // 0 reads as "instant" rather than as the epoch.
    durationMs: _startedAt === undefined ? 0 : Math.max(0, now - _startedAt),
  };
  return [...messages.slice(0, i), closed, ...messages.slice(i + 1)];
}
