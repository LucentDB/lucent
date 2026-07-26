import { describe, it, expect, beforeEach, vi } from 'vitest';

const invokeMock = vi.fn().mockResolvedValue(undefined);
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import {
  chat,
  createConversation,
  createNewTab,
  closeTab,
  appendToThinking,
  finalizeActiveThinkingSegment,
  demoteContentToNote,
  addToolCallSegments,
  updateToolResult,
  finalizeSession,
  setSessionExpanded,
} from './chat.svelte.ts';

function seedMessage(messageId: string, content = '') {
  const conv = createConversation('conn_1');
  conv.messages = [
    {
      id: messageId,
      role: 'assistant' as const,
      content,
      createdAt: Date.now(),
    },
  ];
  chat.conversations = [conv];
  chat.activeConversationId = conv.id;
  return conv;
}

// Re-fetch conversation from store — $state creates internal copies, so
// the original `conv` reference doesn't see mutations made by store functions.
function getConv(id: string) {
  return chat.conversations.find((c) => c.id === id)!;
}

describe('appendToThinking', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
  });

  it('creates a session and a streaming thinking segment on the first chunk', () => {
    const conv = seedMessage('m1');
    appendToThinking(conv.id, 'm1', 'Investigating ');
    const c = getConv(conv.id);
    const session = c.messages[0].session!;
    expect(session.active).toBe(true);
    expect(session.segments).toHaveLength(1);
    expect(session.segments[0]).toMatchObject({
      type: 'thinking',
      content: 'Investigating ',
      streaming: true,
    });
  });

  it('appends subsequent chunks to the same in-progress thinking segment', () => {
    const conv = seedMessage('m1');
    appendToThinking(conv.id, 'm1', 'Investigating ');
    appendToThinking(conv.id, 'm1', 'the schema');
    const c = getConv(conv.id);
    const session = c.messages[0].session!;
    expect(session.segments).toHaveLength(1);
    expect(session.segments[0]).toMatchObject({
      content: 'Investigating the schema',
    });
  });

  it('starts a new thinking segment after the previous one was finalized', () => {
    const conv = seedMessage('m1');
    appendToThinking(conv.id, 'm1', 'first');
    finalizeActiveThinkingSegment(conv.id, 'm1');
    appendToThinking(conv.id, 'm1', 'second');
    const c = getConv(conv.id);
    const session = c.messages[0].session!;
    expect(session.segments).toHaveLength(2);
    expect(session.segments[0]).toMatchObject({
      content: 'first',
      streaming: false,
    });
    expect(session.segments[1]).toMatchObject({
      content: 'second',
      streaming: true,
    });
  });
});

describe('finalizeActiveThinkingSegment', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
  });

  it('marks the in-progress segment done and computes its duration', () => {
    const conv = seedMessage('m1');
    appendToThinking(conv.id, 'm1', 'x');
    finalizeActiveThinkingSegment(conv.id, 'm1');
    const c = getConv(conv.id);
    const seg = c.messages[0].session!.segments[0];
    expect(seg).toMatchObject({ streaming: false });
    expect((seg as { durationMs?: number }).durationMs).toBeGreaterThanOrEqual(
      0,
    );
  });

  it('is a no-op if there is no session yet', () => {
    const conv = seedMessage('m1');
    expect(() => finalizeActiveThinkingSegment(conv.id, 'm1')).not.toThrow();
    expect(conv.messages[0].session).toBeUndefined();
  });

  it('is a no-op if the last segment is not an in-progress thinking segment', () => {
    const conv = seedMessage('m1');
    appendToThinking(conv.id, 'm1', 'x');
    finalizeActiveThinkingSegment(conv.id, 'm1');
    const c = getConv(conv.id);
    const before = { ...c.messages[0].session!.segments[0] };
    finalizeActiveThinkingSegment(conv.id, 'm1');
    expect(c.messages[0].session!.segments[0]).toEqual(before);
  });
});

describe('demoteContentToNote', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
  });

  it('moves non-empty content into a note segment and clears content', () => {
    const conv = seedMessage('m1', 'Let me also check the views…');
    demoteContentToNote(conv.id, 'm1');
    const c = getConv(conv.id);
    const msg = c.messages[0];
    expect(msg.content).toBe('');
    expect(msg.session!.segments).toContainEqual({
      type: 'note',
      content: 'Let me also check the views…',
    });
  });

  it('is a no-op when content is empty', () => {
    const conv = seedMessage('m1', '');
    demoteContentToNote(conv.id, 'm1');
    expect(conv.messages[0].session).toBeUndefined();
  });
});

describe('addToolCallSegments and updateToolResult', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
  });

  it('appends one tool_call segment per tool', () => {
    const conv = seedMessage('m1');
    addToolCallSegments(conv.id, 'm1', [
      { id: 'call_1', name: 'search_schema', args: { query: 'users' } },
      { id: 'call_2', name: 'run_readonly_query', args: { sql: 'select 1' } },
    ]);
    const c = getConv(conv.id);
    const session = c.messages[0].session!;
    expect(session.segments).toHaveLength(2);
    expect(session.segments[0]).toMatchObject({
      type: 'tool_call',
      call: { id: 'call_1', name: 'search_schema' },
    });
  });

  it('updates the matching tool_call segment by id, leaving others untouched', () => {
    const conv = seedMessage('m1');
    addToolCallSegments(conv.id, 'm1', [
      { id: 'call_1', name: 'search_schema', args: {} },
      { id: 'call_2', name: 'run_readonly_query', args: {} },
    ]);
    updateToolResult(conv.id, 'm1', 'call_2', { summary: '4 rows' });
    const c = getConv(conv.id);
    const session = c.messages[0].session!;
    const seg1 = session.segments[0] as {
      type: 'tool_call';
      call: { summary: string | null };
    };
    const seg2 = session.segments[1] as {
      type: 'tool_call';
      call: { summary: string | null };
    };
    expect(seg1.call.summary).toBeNull();
    expect(seg2.call.summary).toBe('4 rows');
  });
});

describe('finalizeSession', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
  });

  it('finalizes any in-progress thinking segment and the session itself', () => {
    const conv = seedMessage('m1');
    appendToThinking(conv.id, 'm1', 'x');
    finalizeSession(conv.id, 'm1');
    const c = getConv(conv.id);
    const session = c.messages[0].session!;
    expect(session.active).toBe(false);
    expect(session.durationMs).toBeGreaterThanOrEqual(0);
    expect(session.segments[0]).toMatchObject({ streaming: false });
  });

  it('is a no-op if there is no session', () => {
    const conv = seedMessage('m1');
    expect(() => finalizeSession(conv.id, 'm1')).not.toThrow();
  });

  it('is a no-op if already finalized (does not overwrite durationMs)', () => {
    const conv = seedMessage('m1');
    appendToThinking(conv.id, 'm1', 'x');
    finalizeSession(conv.id, 'm1');
    const c = getConv(conv.id);
    const first = c.messages[0].session!.durationMs;
    finalizeSession(conv.id, 'm1');
    expect(c.messages[0].session!.durationMs).toBe(first);
  });
});

describe('closeTab', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
    invokeMock.mockClear();
  });

  it('tells the backend to evict the conversation, not just the local list', () => {
    // Regression test: the backend's AppState.conversations map only ever
    // grew (see src-tauri/src/commands.rs close_conversation) because
    // closing a tab used to be a purely local, frontend-only operation.
    const conv = createNewTab('conn_1');
    closeTab(conv.id);
    expect(invokeMock).toHaveBeenCalledWith('close_conversation', {
      conversationId: conv.id,
    });
  });

  it('is a no-op (including no backend call) for an unknown conversation id', () => {
    closeTab('does-not-exist');
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

describe('setSessionExpanded', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
  });

  it('sets the expanded flag on an existing session', () => {
    const conv = seedMessage('m1');
    appendToThinking(conv.id, 'm1', 'x');
    setSessionExpanded(conv.id, 'm1', true);
    const c = getConv(conv.id);
    expect(c.messages[0].session!.expanded).toBe(true);
  });

  it('is a no-op if there is no session', () => {
    const conv = seedMessage('m1');
    expect(() => setSessionExpanded(conv.id, 'm1', true)).not.toThrow();
  });
});
