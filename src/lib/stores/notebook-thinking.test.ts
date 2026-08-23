import { describe, it, expect } from 'vitest';
import {
  appendThinkingChunk,
  closeOpenThinking,
  isThinking,
  type ThinkingMessage,
} from './notebook-thinking.ts';

const asThinking = (m: unknown) => m as ThinkingMessage;

describe('notebook thinking segments', () => {
  it('merges chunks into the open segment while streaming', () => {
    let msgs: unknown[] = [];
    msgs = appendThinkingChunk(msgs, 'Let me ', false, 1_000);
    msgs = appendThinkingChunk(msgs, 'check the schema.', true, 1_100);
    expect(msgs).toHaveLength(1);
    expect(asThinking(msgs[0]).thinking).toBe('Let me check the schema.');
    expect(asThinking(msgs[0])._startedAt).toBe(1_000);
  });

  it('starts a new segment once streaming has been interrupted', () => {
    let msgs: unknown[] = [];
    msgs = appendThinkingChunk(msgs, 'first', false, 1_000);
    msgs = closeOpenThinking(msgs, 5_000); // a tool call interrupts
    msgs = appendThinkingChunk(msgs, 'second', false, 5_000);
    expect(msgs).toHaveLength(2);
    expect(asThinking(msgs[1]).thinking).toBe('second');
  });

  it('times each segment on its own, not from the start of the run', () => {
    // The regression this guards: one start time was stamped on every segment
    // and every duration was computed at the end, so a one-sentence closing
    // thought reported the same 89 seconds as the long opening analysis.
    let msgs: unknown[] = [];
    msgs = appendThinkingChunk(msgs, 'long analysis', false, 0);
    msgs = closeOpenThinking(msgs, 80_000); // tool call at t=80s
    msgs = appendThinkingChunk(msgs, 'I have the data.', false, 80_000);
    msgs = closeOpenThinking(msgs, 82_000); // run ends at t=82s

    expect(asThinking(msgs[0]).durationMs).toBe(80_000);
    expect(asThinking(msgs[1]).durationMs).toBe(2_000);
  });

  it('never restamps a segment that already closed', () => {
    let msgs: unknown[] = [];
    msgs = appendThinkingChunk(msgs, 'first', false, 0);
    msgs = closeOpenThinking(msgs, 1_000);
    const before = asThinking(msgs[0]).durationMs;
    msgs = closeOpenThinking(msgs, 99_000); // run ends much later
    expect(asThinking(msgs[0]).durationMs).toBe(before);
  });

  it('closing with nothing open is a no-op', () => {
    const msgs = closeOpenThinking([], 1_000);
    expect(msgs).toEqual([]);
    const closed = closeOpenThinking([{ thinking: 'x', durationMs: 5 }], 9_000);
    expect(asThinking(closed[0]).durationMs).toBe(5);
  });

  it('drops the internal start marker when a segment closes', () => {
    let msgs: unknown[] = appendThinkingChunk([], 'x', false, 10);
    msgs = closeOpenThinking(msgs, 20);
    expect('_startedAt' in (msgs[0] as object)).toBe(false);
    expect(asThinking(msgs[0]).durationMs).toBe(10);
  });

  it('treats a segment with no start as instant rather than as the epoch', () => {
    const msgs = closeOpenThinking([{ thinking: 'orphan' }], 1_700_000_000_000);
    expect(asThinking(msgs[0]).durationMs).toBe(0);
  });

  it('leaves non-thinking messages alone', () => {
    const other = { note: 'not thinking' };
    const msgs = closeOpenThinking([other], 1_000);
    expect(msgs[0]).toBe(other);
    expect(isThinking(other)).toBe(false);
  });

  it('returns new arrays rather than mutating in place', () => {
    const original: unknown[] = [];
    const appended = appendThinkingChunk(original, 'x', false, 0);
    expect(original).toHaveLength(0);
    const closed = closeOpenThinking(appended, 1);
    expect(closed).not.toBe(appended);
    expect(asThinking(appended[0]).durationMs).toBeUndefined();
  });
});
