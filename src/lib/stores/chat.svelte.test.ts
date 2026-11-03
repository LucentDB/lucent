import { describe, it, expect, beforeEach, vi } from 'vitest';

const invokeMock = vi.fn().mockResolvedValue(undefined);
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import type { PersistedChatMessage } from '../ipc/ai.ts';
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
  markStoppedToolCalls,
  finalizeSession,
  setSessionExpanded,
  hydrateConversations,
  openConversation,
  deleteConversationHistory,
  closeOtherTabs,
  closeTabsToRight,
  closeTabsToLeft,
  closeAllTabs,
  adaptPersistedMessage,
  persistConversationMessage,
  OPEN_TABS_STORAGE_KEY,
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

  it('backfills args when the call reported none, and never overwrites real ones', () => {
    const conv = seedMessage('m1');
    // ACP's CLI path: the agent announced a shell command, so the card has no
    // structured input — the bridge, which executed the call, supplies it.
    addToolCallSegments(conv.id, 'm1', [
      { id: 'tc1', name: './lucent-tool run_readonly_query …', args: null },
      { id: 'tc2', name: 'search_schema', args: {} },
      { id: 'tc3', name: 'run_readonly_query', args: { sql: 'select 1' } },
    ]);
    updateToolResult(conv.id, 'm1', 'tc1', {
      summary: '3 rows',
      args: { sql: 'select 3' },
    });
    updateToolResult(conv.id, 'm1', 'tc2', {
      summary: 'done',
      args: { query: 'invoices' },
    });
    // The agent's own arguments win: a backfill must not rewrite them.
    updateToolResult(conv.id, 'm1', 'tc3', {
      summary: '1 row',
      args: { sql: 'SOMETHING ELSE' },
    });
    const segs = getConv(conv.id).messages[0].session!.segments;
    const args = (i: number) =>
      (segs[i] as { type: 'tool_call'; call: { args: unknown } }).call.args;
    expect(args(0)).toEqual({ sql: 'select 3' });
    expect(args(1)).toEqual({ query: 'invoices' });
    expect(args(2)).toEqual({ sql: 'select 1' });
  });

  it('leaves args alone when no backfill is offered', () => {
    const conv = seedMessage('m1');
    addToolCallSegments(conv.id, 'm1', [
      { id: 'tc1', name: 'run_readonly_query', args: { sql: 'select 1' } },
    ]);
    updateToolResult(conv.id, 'm1', 'tc1', { summary: '1 row' });
    const seg = getConv(conv.id).messages[0].session!.segments[0];
    if (seg.type === 'tool_call')
      expect(seg.call.args).toEqual({ sql: 'select 1' });
  });

  it('updateToolResult applies the explicit status', () => {
    const conv = seedMessage('m1');
    addToolCallSegments(conv.id, 'm1', [
      { id: 'tc1', name: 'run_readonly_query', args: {} },
    ]);
    updateToolResult(conv.id, 'm1', 'tc1', {
      summary: 'read-only guard refused',
      status: 'failed',
    });
    const seg = getConv(conv.id).messages[0].session!.segments[0];
    expect(seg.type).toBe('tool_call');
    if (seg.type === 'tool_call') {
      expect(seg.call.status).toBe('failed');
      expect(seg.call.summary).toBe('read-only guard refused');
    }
  });

  it('updateToolResult falls back to the error-prefix convention', () => {
    const conv = seedMessage('m1');
    addToolCallSegments(conv.id, 'm1', [{ id: 'tc1', name: 'x', args: {} }]);
    updateToolResult(conv.id, 'm1', 'tc1', { summary: 'error: boom' });
    const seg = getConv(conv.id).messages[0].session!.segments[0];
    if (seg.type === 'tool_call') expect(seg.call.status).toBe('failed');
  });

  it('markStoppedToolCalls only touches unresolved calls', () => {
    const conv = seedMessage('m1');
    addToolCallSegments(conv.id, 'm1', [
      { id: 'tc1', name: 'a', args: {} },
      { id: 'tc2', name: 'b', args: {} },
      { id: 'tc3', name: 'c', args: {} },
    ]);
    updateToolResult(conv.id, 'm1', 'tc1', {
      summary: 'ok',
      status: 'completed',
    });
    updateToolResult(conv.id, 'm1', 'tc2', {
      summary: 'bad',
      status: 'failed',
    });
    markStoppedToolCalls(conv.id, 'm1');
    const segs = getConv(conv.id).messages[0].session!.segments;
    const byId = Object.fromEntries(
      segs.map((s) =>
        s.type === 'tool_call' ? [s.call.id, s.call.status] : [],
      ),
    );
    expect(byId['tc1']).toBe('completed');
    expect(byId['tc2']).toBe('failed');
    expect(byId['tc3']).toBe('stopped');
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

describe('tab batch close helpers (chat tab context menu)', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
    invokeMock.mockClear();
  });

  function seedThreeTabs() {
    const a = createNewTab('conn_1');
    const b = createNewTab('conn_1');
    const c = createNewTab('conn_1');
    return { a, b, c };
  }

  it('closeOtherTabs keeps only the target tab', () => {
    const { b } = seedThreeTabs();
    closeOtherTabs(b.id);
    expect(chat.conversations.map((c) => c.id)).toEqual([b.id]);
    expect(chat.activeConversationId).toBe(b.id);
  });

  it('closeOtherTabs moves the active tab when the active one is closed', () => {
    const { a } = seedThreeTabs();
    // Closing the others removes the active tab (c), so the survivor becomes
    // active instead of leaving a dangling id.
    closeOtherTabs(a.id);
    expect(chat.conversations.map((c) => c.id)).toEqual([a.id]);
    expect(chat.activeConversationId).toBe(a.id);
  });

  it('closeTabsToRight closes only the tabs after the target', () => {
    const { a } = seedThreeTabs();
    closeTabsToRight(a.id);
    expect(chat.conversations.map((c) => c.id)).toEqual([a.id]);
  });

  it('closeTabsToLeft closes only the tabs before the target', () => {
    const { c } = seedThreeTabs();
    closeTabsToLeft(c.id);
    expect(chat.conversations.map((cv) => cv.id)).toEqual([c.id]);
  });

  it('closeAllTabs empties the strip and clears the active tab', () => {
    seedThreeTabs();
    closeAllTabs();
    expect(chat.conversations).toEqual([]);
    expect(chat.activeConversationId).toBeNull();
  });

  it('evicts every closed batch tab on the backend exactly once', () => {
    const { a, b, c } = seedThreeTabs();
    closeTabsToRight(a.id);
    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(invokeMock).toHaveBeenCalledWith('close_conversation', {
      conversationId: b.id,
    });
    expect(invokeMock).toHaveBeenCalledWith('close_conversation', {
      conversationId: c.id,
    });
  });

  it('is a no-op for an unknown anchor id', () => {
    seedThreeTabs();
    closeTabsToRight('does-not-exist');
    closeTabsToLeft('does-not-exist');
    expect(chat.conversations).toHaveLength(3);
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

describe('createConversation', () => {
  it('conversations carry a per-conversation error field', () => {
    const conv = createConversation('conn-1');
    expect(conv.error).toBeNull();
    conv.error = 'boom';
    expect(conv.error).toBe('boom');
  });
});

describe('hydrateConversations', () => {
  const persistedConvs = [
    {
      id: 'conv-a',
      connection_id: 'conn-a',
      title: 'First',
      archived: false,
      created_at: 1000,
      updated_at: 1001,
    },
    {
      id: 'conv-b',
      connection_id: 'conn-b',
      title: 'Second',
      archived: false,
      created_at: 2000,
      updated_at: 2001,
    },
  ];
  const persistedMessages: Record<string, PersistedChatMessage[]> = {
    'conv-a': [
      {
        id: 'm1',
        conversation_id: 'conv-a',
        role: 'user',
        content: 'hello',
        session_json: null,
        created_at: 1001,
      },
      {
        id: 'm2',
        conversation_id: 'conv-a',
        role: 'assistant',
        content: 'hi there',
        session_json: null,
        created_at: 1002,
      },
    ],
    'conv-b': [
      {
        id: 'm3',
        conversation_id: 'conv-b',
        role: 'user',
        content: 'second',
        session_json: null,
        created_at: 2001,
      },
    ],
  };

  beforeEach(() => {
    localStorage.clear();
    chat.conversations = [];
    chat.activeConversationId = null;
    invokeMock.mockReset();
    invokeMock.mockImplementation(
      async (cmd: string, args: { conversationId?: string }) => {
        if (cmd === 'list_chat_conversations') return persistedConvs;
        if (cmd === 'load_chat_conversation')
          return persistedMessages[args.conversationId ?? ''] ?? [];
        return undefined;
      },
    );
  });

  it('restores every persisted conversation and adapts its messages to the runtime shape', async () => {
    await hydrateConversations();

    expect(chat.conversations).toHaveLength(2);
    expect(chat.conversations.map((c) => c.id)).toEqual(['conv-a', 'conv-b']);
    expect(chat.activeConversationId).toBe('conv-a');

    // C1 runtime shape: `createdAt` is Unix ms, not the persisted seconds;
    // the idle conversation fields start empty.
    const a = getConv('conv-a');
    expect(a.connectionId).toBe('conn-a');
    expect(a.createdAt).toBe(1_000_000);
    expect(a.isPaused).toBe(false);
    expect(a.pausedDml).toBeNull();
    expect(a.pendingPermission).toBeNull();
    expect(a.messages).toHaveLength(2);
    expect(a.messages[0]).toEqual({
      id: 'm1',
      role: 'user',
      content: 'hello',
      createdAt: 1_001_000,
    });
    expect(a.messages[1]).toMatchObject({
      id: 'm2',
      role: 'assistant',
      content: 'hi there',
      createdAt: 1_002_000,
    });

    // The non-active conversation is hydrated too, so its tab renders its
    // title and history when selected.
    const b = getConv('conv-b');
    expect(b.connectionId).toBe('conn-b');
    expect(b.messages).toHaveLength(1);
    expect(b.messages[0]).toMatchObject({
      id: 'm3',
      role: 'user',
      content: 'second',
      createdAt: 2_001_000,
    });
  });

  it('parses persisted session_json into the runtime work session', async () => {
    const session = {
      segments: [{ type: 'note', content: 'remembered' }],
      startedAt: 5,
      active: false,
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_chat_conversations') return [persistedConvs[0]];
      if (cmd === 'load_chat_conversation')
        return [
          {
            id: 'm1',
            conversation_id: 'conv-a',
            role: 'assistant',
            content: 'answer',
            session_json: JSON.stringify(session),
            created_at: 10,
          },
        ];
      return undefined;
    });

    await hydrateConversations();

    expect(getConv('conv-a').messages[0].session).toEqual(session);
  });

  it('leaves the store untouched when nothing is persisted', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_chat_conversations') return [];
      return undefined;
    });

    await hydrateConversations();

    expect(chat.conversations).toEqual([]);
    expect(chat.activeConversationId).toBeNull();
  });

  it('swallows IPC failures instead of rejecting app startup', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    invokeMock.mockRejectedValue(new Error('memory.db is unreadable'));

    await expect(hydrateConversations()).resolves.toBeUndefined();
    expect(chat.conversations).toEqual([]);
    expect(errorSpy).toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('restores ONLY conversations that were left open when closing', async () => {
    localStorage.setItem(OPEN_TABS_STORAGE_KEY, JSON.stringify(['conv-b']));

    await hydrateConversations();

    expect(chat.conversations).toHaveLength(1);
    expect(chat.conversations[0].id).toBe('conv-b');
    expect(chat.activeConversationId).toBe('conv-b');
    localStorage.removeItem(OPEN_TABS_STORAGE_KEY);
  });

  it('restores no chat tabs if all tabs were closed before exit', async () => {
    localStorage.setItem(OPEN_TABS_STORAGE_KEY, JSON.stringify([]));

    await hydrateConversations();

    expect(chat.conversations).toHaveLength(0);
    expect(chat.activeConversationId).toBeNull();
    localStorage.removeItem(OPEN_TABS_STORAGE_KEY);
  });

  it('openConversation loads and adds a past conversation to open tabs', async () => {
    chat.conversations = [];
    chat.activeConversationId = null;

    const conv = await openConversation('conv-a');

    expect(conv).toBeDefined();
    expect(conv!.id).toBe('conv-a');
    expect(chat.conversations).toHaveLength(1);
    expect(chat.activeConversationId).toBe('conv-a');
    expect(getConv('conv-a').messages).toHaveLength(2);
  });

  it('deleteConversationHistory deletes conversation and closes open tab', async () => {
    const conv = createConversation('conn-1');
    chat.conversations = [conv];
    chat.activeConversationId = conv.id;

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'delete_chat_conversation') return true;
      return undefined;
    });

    const res = await deleteConversationHistory(conv.id);

    expect(res).toBe(true);
    expect(chat.conversations).toHaveLength(0);
  });

  it('openConversation reloads messages if conversation tab exists but has empty messages', async () => {
    const conv = createConversation('conn-1');
    conv.id = 'conv-empty';
    conv.messages = [];
    chat.conversations = [conv];
    chat.activeConversationId = conv.id;

    invokeMock.mockImplementation(async (cmd: string, args: any) => {
      if (cmd === 'load_chat_conversation' && args?.conversationId === 'conv-empty') {
        return [
          {
            id: 'm1',
            conversation_id: 'conv-empty',
            role: 'user',
            content: 'Hello database',
            session_json: null,
            created_at: 1000,
          },
          {
            id: 'm2',
            conversation_id: 'conv-empty',
            role: 'assistant',
            content: 'Hello user',
            session_json: null,
            created_at: 1001,
          },
        ];
      }
      return undefined;
    });

    const res = await openConversation('conv-empty');

    expect(res).toBeDefined();
    expect(res!.messages).toHaveLength(2);
    expect(res!.messages[0].content).toBe('Hello database');
    expect(res!.messages[1].content).toBe('Hello user');
  });

  it('openConversation refreshes messages for an existing tab with stale messages', async () => {
    const conv = createConversation('conn-1');
    conv.id = 'conv-stale';
    conv.messages = [
      {
        id: 'm1',
        role: 'user',
        content: 'Initial question',
        createdAt: 1000,
      },
    ];
    chat.conversations = [conv];
    chat.activeConversationId = 'other-conv';

    invokeMock.mockImplementation(async (cmd: string, args: any) => {
      if (cmd === 'load_chat_conversation' && args?.conversationId === 'conv-stale') {
        return [
          {
            id: 'm1',
            conversation_id: 'conv-stale',
            role: 'user',
            content: 'Initial question',
            session_json: null,
            created_at: 1000,
          },
          {
            id: 'm2',
            conversation_id: 'conv-stale',
            role: 'assistant',
            content: 'Here is the detailed response',
            session_json: JSON.stringify({
              segments: [
                {
                  type: 'thinking',
                  content: 'Reasoning about data',
                  streaming: false,
                  startedAt: 1000,
                  durationMs: 400,
                },
              ],
              startedAt: 1000,
              durationMs: 400,
              active: false,
            }),
            created_at: 1001,
          },
        ];
      }
      return undefined;
    });

    const res = await openConversation('conv-stale');

    expect(res).toBeDefined();
    expect(res!.messages).toHaveLength(2);
    expect(res!.messages[0].content).toBe('Initial question');
    expect(res!.messages[1].content).toBe('Here is the detailed response');
    expect(res!.messages[1].session?.segments).toHaveLength(1);
    expect(chat.activeConversationId).toBe('conv-stale');
  });
});

describe('adaptPersistedMessage', () => {
  it('converts seconds to milliseconds timestamp', () => {
    const p: PersistedChatMessage = {
      id: 'msg-1',
      conversation_id: 'c-1',
      role: 'user',
      content: 'hi',
      session_json: null,
      created_at: 1700000000,
    };
    const adapted = adaptPersistedMessage(p);
    expect(adapted.createdAt).toBe(1700000000000);
    expect(adapted.role).toBe('user');
    expect(adapted.content).toBe('hi');
    expect(adapted.session).toBeUndefined();
  });

  it('preserves timestamps that are already in milliseconds', () => {
    const p: PersistedChatMessage = {
      id: 'msg-1',
      conversation_id: 'c-1',
      role: 'assistant',
      content: 'hi',
      session_json: null,
      created_at: 1700000000123,
    };
    const adapted = adaptPersistedMessage(p);
    expect(adapted.createdAt).toBe(1700000000123);
  });

  it('parses session_json and normalizes active/streaming/tool status', () => {
    const sessionPayload = {
      segments: [
        {
          type: 'thinking',
          content: 'thinking deltas...',
          streaming: true,
          startedAt: 100,
        },
        {
          type: 'tool_call',
          call: {
            id: 'call_1',
            name: 'run_readonly_query',
            args: { sql: 'SELECT 1' },
            summary: '1 row',
            // status omitted
          },
        },
        {
          type: 'tool_call',
          call: {
            id: 'call_2',
            name: 'run_readonly_query',
            args: { sql: 'SELECT error' },
            summary: 'error: syntax error',
            // status omitted
          },
        },
      ],
      startedAt: 100,
      durationMs: 500,
      active: true,
    };

    const p: PersistedChatMessage = {
      id: 'msg-2',
      conversation_id: 'c-1',
      role: 'assistant',
      content: 'Here is what I found',
      session_json: JSON.stringify(sessionPayload),
      created_at: 1700000000,
    };

    const adapted = adaptPersistedMessage(p);
    expect(adapted.session).toBeDefined();
    expect(adapted.session!.active).toBe(false);
    expect(adapted.session!.segments).toHaveLength(3);

    const thinking = adapted.session!.segments[0] as any;
    expect(thinking.type).toBe('thinking');
    expect(thinking.streaming).toBe(false);

    const tool1 = adapted.session!.segments[1] as any;
    expect(tool1.call.status).toBe('completed');

    const tool2 = adapted.session!.segments[2] as any;
    expect(tool2.call.status).toBe('failed');
  });

  it('normalizes running tool call status to completed or stopped', () => {
    const sessionPayload = {
      segments: [
        {
          type: 'tool_call',
          call: {
            id: 'call_1',
            name: 'run_readonly_query',
            args: { sql: 'SELECT 1' },
            summary: '1 row',
            status: 'running',
          },
        },
        {
          type: 'tool_call',
          call: {
            id: 'call_2',
            name: 'run_readonly_query',
            args: { sql: 'SELECT 2' },
            summary: null,
            status: 'running',
          },
        },
      ],
      startedAt: 100,
      durationMs: 500,
      active: true,
    };

    const p: PersistedChatMessage = {
      id: 'msg-running',
      conversation_id: 'c-1',
      role: 'assistant',
      content: 'Here is what I found',
      session_json: JSON.stringify(sessionPayload),
      created_at: 1700000000,
    };

    const adapted = adaptPersistedMessage(p);
    const seg1 = adapted.session!.segments[0] as any;
    expect(seg1.call.status).toBe('completed');
    const seg2 = adapted.session!.segments[1] as any;
    expect(seg2.call.status).toBe('stopped');
  });

  it('auto-expands session when assistant response content is empty', () => {
    const sessionPayload = {
      segments: [
        {
          type: 'tool_call',
          call: {
            id: 'call_1',
            name: 'search_schema',
            args: { pattern: 'users' },
            summary: '3 matches',
            status: 'completed',
          },
        },
      ],
      startedAt: 100,
      durationMs: 300,
      active: false,
    };

    const p: PersistedChatMessage = {
      id: 'msg-tool-only',
      conversation_id: 'c-1',
      role: 'assistant',
      content: '',
      session_json: JSON.stringify(sessionPayload),
      created_at: 1700000000,
    };

    const adapted = adaptPersistedMessage(p);
    expect(adapted.session!.expanded).toBe(true);
  });

  it('handles invalid session_json without throwing', () => {
    const p: PersistedChatMessage = {
      id: 'msg-corrupt',
      conversation_id: 'c-1',
      role: 'assistant',
      content: 'test',
      session_json: '{invalid json...',
      created_at: 1700000000,
    };

    const adapted = adaptPersistedMessage(p);
    expect(adapted.content).toBe('test');
    expect(adapted.session).toBeUndefined();
  });
});

describe('persistConversationMessage', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    chat.conversations = [];
  });

  it('serializes message with session and calls save_chat_message', async () => {
    invokeMock.mockResolvedValue(undefined);
    const conv = createConversation('conn-1');
    conv.id = 'conv-persist';
    const msg = {
      id: 'msg-persisted',
      role: 'assistant' as const,
      content: 'Final response',
      createdAt: 1700000005000,
      session: {
        segments: [
          {
            type: 'thinking' as const,
            content: 'thought',
            streaming: false,
            startedAt: 1700000005000,
          },
        ],
        startedAt: 1700000005000,
        durationMs: 1200,
        active: false,
      },
    };
    conv.messages = [msg];
    chat.conversations = [conv];

    await persistConversationMessage('conv-persist', 'msg-persisted');

    expect(invokeMock).toHaveBeenCalledWith('save_chat_message', {
      message: {
        id: 'msg-persisted',
        conversation_id: 'conv-persist',
        role: 'assistant',
        content: 'Final response',
        session_json: JSON.stringify(msg.session),
        created_at: 1700000005,
      },
    });
  });

  it('is a safe no-op if message cannot be found', async () => {
    await persistConversationMessage('unknown-conv', 'unknown-msg');
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
