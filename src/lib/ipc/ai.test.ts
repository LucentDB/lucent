import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  chat,
  createConversation,
  createNewTab,
  addMessage,
  formatUsageLine,
  pauseForPermission,
  resumeFromPermission,
  clearRejectedDml,
  updateLast,
  setRunPhase,
  isConvRunning,
} from '../stores/chat.svelte.ts';
import {
  handleAiEvent,
  flushStreamPending,
  createAiSession,
  aiSessionFor,
  releaseAiSession,
  listAiModels,
  listRegistryAgents,
  installAcpAgent,
  uninstallAcpAgent,
  listInstalledAcpAgents,
  respondAgentPermission,
  rejectDml,
  rejectPendingDml,
  saveAiSettings,
  saveChatMessage,
  sendMessage,
  type AgentPermissionPayload,
} from './ai.ts';
import { aiConfig } from '../stores/ai-config.svelte.ts';

const invokeMock = vi.fn().mockResolvedValue(undefined);

// Fidelity fake for Tauri's Channel semantics — mirrors
// @tauri-apps/api/core.js (Channel: per-instance monotonically increasing
// JS index counter, `{end:true}` unregisters the callback) plus the Rust
// side's per-invoke index counter (channel.rs `channel_on`) and the
// `on_drop` end marker (commands.rs:2987 overwrites `event_channel`).
// Defined via vi.hoisted so the vi.mock factory below can reference it.
const chanHarness = vi.hoisted(() => {
  type RawChanMsg = { index: number; end?: boolean; message?: unknown };
  const chanCallbacks = new Map<number, (data: RawChanMsg) => void>();
  const chanDropped: number[] = [];
  const chanSentCount = new Map<number, number>();
  let chanSeq = 0;
  class FakeChannel {
    id: number;
    onmessage: (e: never) => void;
    next = 0;
    pending: Record<number, unknown> = {};
    messageEndIndex: number | undefined;
    constructor() {
      this.id = ++chanSeq;
      this.onmessage = () => {};
      const self = this;
      chanCallbacks.set(this.id, (raw) => self.receive(raw));
    }
    deliver(msg: unknown) {
      this.onmessage(msg as never);
      this.next += 1;
    }
    receive(raw: RawChanMsg) {
      if ('end' in raw) {
        // cleanupCallback: only unregisters when the end index matches.
        if (raw.index === this.next) chanCallbacks.delete(this.id);
        else this.messageEndIndex = raw.index;
        return;
      }
      if (raw.index === this.next) {
        this.deliver(raw.message);
        while (this.next in this.pending) {
          const m = this.pending[this.next];
          delete this.pending[this.next];
          this.deliver(m);
        }
      } else {
        // Parked in pendingMessages, never flushed — the index-counter half
        // of why a reused channel drops turn-2 events.
        this.pending[raw.index] = raw.message;
      }
    }
    toJSON() {
      return `__CHANNEL__:${this.id}`;
    }
  }
  // Stands in for the Rust side's webview.eval(format_raw_js(id, json)).
  // `tauri channel.rs:138,145` mints a FRESH index counter per command
  // invocation, so each `chanSender` (one per send/turn) counts from 0 —
  // even when the JS channel id is reused. That is the faithful model of
  // the index-counter half of the reuse bug: a reused JS channel whose
  // `next` has advanced parks every turn-2 message in `pendingMessages`.
  function chanSender(id: number) {
    let idx = 0;
    return {
      send(msg: unknown) {
        chanSentCount.set(id, (chanSentCount.get(id) ?? 0) + 1);
        const cb = chanCallbacks.get(id);
        if (!cb) {
          chanDropped.push(id);
          return;
        }
        const i = idx;
        idx += 1;
        cb({ message: msg, index: i });
      },
    };
  }
  // Stands in for the Rust `on_drop` eval that fires when the previous
  // channel is dropped (commands.rs:2987). The Rust side reports its message
  // count so far as the index — channel.rs:186-191.
  function chanEnd(id: number) {
    chanCallbacks.get(id)?.({ index: chanSentCount.get(id) ?? 0, end: true });
  }
  function chanReset() {
    chanCallbacks.clear();
    chanDropped.length = 0;
    chanSentCount.clear();
  }
  return { FakeChannel, chanSender, chanEnd, chanReset, chanDropped };
});
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  Channel: chanHarness.FakeChannel,
}));

const listenMock = vi.fn();
const listenerCbs: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(
    async (event: string, cb: (e: { payload: unknown }) => void) => {
      listenerCbs[event] = cb;
      listenMock(event, cb);
      return () => {};
    },
  ),
}));

function seedActiveConversationWithMessage(messageId: string) {
  const conv = createConversation('conn_1');
  chat.conversations = [conv];
  chat.activeConversationId = conv.id;
  addMessage(conv.id, {
    id: messageId,
    role: 'assistant',
    content: '',
    createdAt: Date.now(),
  });
  return conv;
}

function seedConversation(convId: string) {
  chat.conversations = [];
  // Assign the id BEFORE the conversation enters the store: $state proxies
  // do not propagate post-hoc id mutation on the original reference.
  const conv = createConversation('conn-1');
  conv.id = convId;
  chat.conversations = [conv];
  chat.activeConversationId = convId;
  return conv;
}

// Re-fetch conversation from store — $state creates internal copies, so
// the original `conv` reference doesn't see mutations made by store functions.
function getConv(id: string) {
  return chat.conversations.find((c) => c.id === id)!;
}

// Deterministic rAF stub: the stream-coalescing path in ai.ts buffers
// text/thinking behind requestAnimationFrame.
function stubRaf() {
  const rafQueue: FrameRequestCallback[] = [];
  const realRaf = globalThis.requestAnimationFrame;
  globalThis.requestAnimationFrame = ((cb: FrameRequestCallback) => {
    rafQueue.push(cb);
    return rafQueue.length;
  }) as typeof requestAnimationFrame;
  return {
    flushFrame: () => {
      for (const cb of rafQueue.splice(0)) cb(0);
    },
    restoreRaf: () => {
      globalThis.requestAnimationFrame = realRaf;
    },
  };
}

function turnDoneEvent(convId: string) {
  return {
    type: 'done',
    conversation_id: convId,
    final_message: 'ok',
    cancelled: false,
    usage: {
      prompt_tokens: 10,
      completion_tokens: 5,
      cached_prompt_tokens: 0,
    },
  } as const;
}

describe('handleAiEvent', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
    invokeMock.mockReset();
  });

  it('accumulates multiple thinking deltas onto one segment on the same message', () => {
    const conv = seedActiveConversationWithMessage('m1');
    handleAiEvent(conv.id, { type: 'thinking', content: 'Investigating ' });
    handleAiEvent(conv.id, { type: 'thinking', content: 'the schema' });
    flushStreamPending();
    const c = getConv(conv.id);
    expect(c.messages).toHaveLength(1);
    const session = c.messages[0].session!;
    expect(session.segments).toHaveLength(1);
    expect(session.segments[0]).toMatchObject({
      type: 'thinking',
      content: 'Investigating the schema',
    });
  });

  it('finalizes the thinking segment and adds a tool_call segment on tool_calls', () => {
    const conv = seedActiveConversationWithMessage('m1');
    handleAiEvent(conv.id, { type: 'thinking', content: 'Investigating' });
    handleAiEvent(conv.id, {
      type: 'tool_calls',
      tools: [{ id: 'call_1', name: 'search_schema', args: {} }],
    });
    const c = getConv(conv.id);
    const session = c.messages[0].session!;
    expect(session.segments).toHaveLength(2);
    expect(session.segments[0]).toMatchObject({
      type: 'thinking',
      streaming: false,
    });
    expect(session.segments[1]).toMatchObject({
      type: 'tool_call',
      call: { name: 'search_schema' },
    });
  });

  it('demotes mid-work commentary text into a note segment when tool_calls follows', () => {
    const conv = seedActiveConversationWithMessage('m1');
    handleAiEvent(conv.id, {
      type: 'text',
      content: 'Let me also check the views',
    });
    handleAiEvent(conv.id, {
      type: 'tool_calls',
      tools: [{ id: 'call_1', name: 'run_readonly_query', args: {} }],
    });
    const c = getConv(conv.id);
    const msg = c.messages[0];
    expect(msg.content).toBe('');
    const session = msg.session!;
    expect(session.segments[0]).toEqual({
      type: 'note',
      content: 'Let me also check the views',
    });
    expect(session.segments[1]).toMatchObject({
      type: 'tool_call',
      call: { name: 'run_readonly_query' },
    });
  });

  it('keeps the true final answer in content, untouched, when done arrives with no more tool calls', () => {
    const conv = seedActiveConversationWithMessage('m1');
    handleAiEvent(conv.id, { type: 'thinking', content: 'Reasoning' });
    handleAiEvent(conv.id, {
      type: 'tool_calls',
      tools: [{ id: 'call_1', name: 'search_schema', args: {} }],
    });
    handleAiEvent(conv.id, { type: 'text', content: 'Here is the answer' });
    handleAiEvent(conv.id, {
      type: 'done',
      conversation_id: conv.id,
      final_message: 'Here is the answer',
      cancelled: false,
      usage: {
        prompt_tokens: 10,
        completion_tokens: 5,
        cached_prompt_tokens: 0,
      },
    });
    const c = getConv(conv.id);
    const msg = c.messages[0];
    expect(msg.content).toBe('Here is the answer');
    expect(msg.session!.active).toBe(false);
    expect(typeof msg.session!.durationMs).toBe('number');
    expect(msg.usage).toEqual({
      promptTokens: 10,
      completionTokens: 5,
      cachedPromptTokens: 0,
      cachedWritePromptTokens: 0,
    });
  });

  it('threads applied_memory_count from done into the message rulesApplied', () => {
    const conv = seedActiveConversationWithMessage('m1');
    handleAiEvent(conv.id, {
      type: 'done',
      conversation_id: conv.id,
      final_message: 'ok',
      cancelled: false,
      usage: {
        prompt_tokens: 10,
        completion_tokens: 5,
        cached_prompt_tokens: 0,
      },
      applied_memory_count: 3,
    });
    expect(getConv(conv.id).messages[0].rulesApplied).toBe(3);
  });

  it('defaults rulesApplied to 0 when done omits applied_memory_count', () => {
    const conv = seedActiveConversationWithMessage('m1');
    handleAiEvent(conv.id, {
      type: 'done',
      conversation_id: conv.id,
      final_message: 'ok',
      cancelled: false,
      usage: {
        prompt_tokens: 10,
        completion_tokens: 5,
        cached_prompt_tokens: 0,
      },
    });
    expect(getConv(conv.id).messages[0].rulesApplied).toBe(0);
  });

  it('fetches accumulated usage once on done and stores it on the conversation', async () => {
    const conv = seedActiveConversationWithMessage('m1');
    invokeMock.mockResolvedValue({
      prompt_tokens: 120,
      completion_tokens: 45,
      cached_prompt_tokens: 30,
    });
    handleAiEvent(conv.id, {
      type: 'done',
      conversation_id: conv.id,
      final_message: 'ok',
      cancelled: false,
      usage: {
        prompt_tokens: 120,
        completion_tokens: 45,
        cached_prompt_tokens: 30,
      },
    });
    await vi.waitFor(() => {
      expect(getConv(conv.id).usage).toEqual({
        promptTokens: 120,
        completionTokens: 45,
        cachedPromptTokens: 30,
        cachedWritePromptTokens: 0,
      });
    });
    expect(invokeMock).toHaveBeenCalledWith('get_ai_usage', {
      conversationId: conv.id,
    });
    expect(invokeMock).toHaveBeenCalledWith('save_chat_message', expect.objectContaining({
      message: expect.objectContaining({
        id: 'm1',
        conversation_id: conv.id,
      }),
    }));
  });

  it('keeps the previous usage when the get_ai_usage fetch fails', async () => {
    const conv = seedActiveConversationWithMessage('m1');
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_ai_usage') throw new Error('nope');
      return undefined;
    });
    handleAiEvent(conv.id, {
      type: 'done',
      conversation_id: conv.id,
      final_message: 'ok',
      cancelled: false,
      usage: {
        prompt_tokens: 10,
        completion_tokens: 5,
        cached_prompt_tokens: 0,
      },
    });
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('get_ai_usage', {
        conversationId: conv.id,
      });
    });
    expect(getConv(conv.id).usage).toBeNull();
  });

  it('persists the completed assistant message on done', async () => {
    const conv = seedActiveConversationWithMessage('m1');
    handleAiEvent(conv.id, {
      type: 'done',
      conversation_id: conv.id,
      final_message: 'Finished task',
      cancelled: false,
      usage: {
        prompt_tokens: 10,
        completion_tokens: 5,
        cached_prompt_tokens: 0,
      },
    });
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('save_chat_message', expect.objectContaining({
        message: expect.objectContaining({
          id: 'm1',
          conversation_id: conv.id,
          role: 'assistant',
        }),
      }));
    });
  });

  it('updates the correct tool_call segment by id on tool_result', () => {
    const conv = seedActiveConversationWithMessage('m1');
    handleAiEvent(conv.id, {
      type: 'tool_calls',
      tools: [
        { id: 'call_1', name: 'search_schema', args: {} },
        { id: 'call_2', name: 'run_readonly_query', args: {} },
      ],
    });
    handleAiEvent(conv.id, {
      type: 'tool_result',
      id: 'call_2',
      tool: 'run_readonly_query',
      summary: '4 rows',
      output: null,
    });
    const c = getConv(conv.id);
    const session = c.messages[0].session!;
    const seg1 = session.segments[0] as {
      type: 'tool_call';
      call: { summary: string | null; status?: string };
    };
    const seg2 = session.segments[1] as {
      type: 'tool_call';
      call: { summary: string | null; status?: string };
    };
    expect(seg1.call.summary).toBeNull();
    expect(seg2.call.summary).toBe('4 rows');
    expect(seg2.call.status).toBe('completed');
  });

  it('forwards explicit tool_result status (completed/failed)', () => {
    const conv = seedActiveConversationWithMessage('m1');
    handleAiEvent(conv.id, {
      type: 'tool_calls',
      tools: [{ id: 'call_1', name: 'run_readonly_query', args: {} }],
    });
    handleAiEvent(conv.id, {
      type: 'tool_result',
      id: 'call_1',
      tool: 'run_readonly_query',
      summary: 'read-only guard refused',
      status: 'failed',
      output: null,
    });
    const c = getConv(conv.id);
    const seg = c.messages[0].session!.segments[0];
    if (seg.type === 'tool_call') {
      expect(seg.call.status).toBe('failed');
      expect(seg.call.summary).toBe('read-only guard refused');
    }
  });

  it('marks unresolved tool calls stopped when done arrives with cancelled: true', () => {
    const conv = seedActiveConversationWithMessage('m1');
    handleAiEvent(conv.id, {
      type: 'tool_calls',
      tools: [
        { id: 'call_1', name: 'a', args: {} },
        { id: 'call_2', name: 'b', args: {} },
      ],
    });
    handleAiEvent(conv.id, {
      type: 'tool_result',
      id: 'call_1',
      tool: 'a',
      summary: 'ok',
      status: 'completed',
      output: null,
    });
    handleAiEvent(conv.id, {
      type: 'done',
      conversation_id: conv.id,
      final_message: '',
      cancelled: true,
      usage: {
        prompt_tokens: 10,
        completion_tokens: 5,
        cached_prompt_tokens: 0,
      },
    });
    const c = getConv(conv.id);
    const segs = c.messages[0].session!.segments;
    const byId = Object.fromEntries(
      segs.map((s) =>
        s.type === 'tool_call' ? [s.call.id, s.call.status] : [],
      ),
    );
    expect(byId['call_1']).toBe('completed');
    expect(byId['call_2']).toBe('stopped');
  });

  it('does nothing and does not throw when no message exists yet for the conversation', () => {
    const conv = createConversation('conn_1');
    chat.conversations = [conv];
    chat.activeConversationId = conv.id;
    expect(() =>
      handleAiEvent(conv.id, { type: 'thinking', content: 'x' }),
    ).not.toThrow();
    expect(conv.messages).toHaveLength(0);
  });
});

describe('formatUsageLine', () => {
  it('renders a one-line in/out token summary', () => {
    expect(
      formatUsageLine({
        promptTokens: 120,
        completionTokens: 45,
        cachedPromptTokens: 0,
      }),
    ).toBe('120 in / 45 out tokens');
  });
});

describe('createAiSession listeners', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.isStreaming = false;
    for (const k of Object.keys(listenerCbs)) delete listenerCbs[k];
  });

  it('ignores dml_approval events for other conversations', async () => {
    const conv = seedConversation('conv-a');
    const session = createAiSession('conv-a');
    const onDmlApproval = vi.fn();
    await session.setupListeners({
      onDmlApproval,
      onAgentPermission: vi.fn(),
      onError: vi.fn(),
    });
    listenerCbs['ai:dml_approval']?.({
      payload: {
        conversation_id: 'conv-b',
        sql: 'delete from t',
        description: 'x',
        estimated_rows_affected: null,
      },
    });
    expect(onDmlApproval).not.toHaveBeenCalled();
    expect(conv.isPaused).toBe(false);
  });

  it('routes dml_approval for its own conversation', async () => {
    seedConversation('conv-a');
    const session = createAiSession('conv-a');
    const onDmlApproval = vi.fn();
    await session.setupListeners({
      onDmlApproval,
      onAgentPermission: vi.fn(),
      onError: vi.fn(),
    });
    listenerCbs['ai:dml_approval']?.({
      payload: {
        conversation_id: 'conv-a',
        sql: 'delete from t',
        description: 'x',
        estimated_rows_affected: null,
      },
    });
    expect(onDmlApproval).toHaveBeenCalledTimes(1);
  });

  it('ignores agent_permission events for other conversations', async () => {
    seedConversation('conv-a');
    const session = createAiSession('conv-a');
    const onAgentPermission = vi.fn();
    await session.setupListeners({
      onDmlApproval: vi.fn(),
      onAgentPermission,
      onError: vi.fn(),
    });
    listenerCbs['ai:agent_permission']?.({
      payload: {
        conversationId: 'conv-b',
        title: 't',
        description: 'd',
        options: [],
      },
    });
    expect(onAgentPermission).not.toHaveBeenCalled();
  });

  it('ignores error events for other conversations', async () => {
    const conv = seedConversation('conv-a');
    const session = createAiSession('conv-a');
    const onError = vi.fn();
    await session.setupListeners({
      onDmlApproval: vi.fn(),
      onAgentPermission: vi.fn(),
      onError,
    });
    listenerCbs['ai:error']?.({
      payload: { conversation_id: 'conv-b', message: 'boom' },
    });
    expect(onError).not.toHaveBeenCalled();
    expect(conv.error).toBeNull();
  });

  it('passes matching error events to the handler', async () => {
    seedConversation('conv-a');
    const session = createAiSession('conv-a');
    const onError = vi.fn();
    await session.setupListeners({
      onDmlApproval: vi.fn(),
      onAgentPermission: vi.fn(),
      onError,
    });
    listenerCbs['ai:error']?.({
      payload: { conversation_id: 'conv-a', message: 'boom' },
    });
    expect(onError).toHaveBeenCalledWith({
      conversation_id: 'conv-a',
      message: 'boom',
    });
  });
});

describe('acp registry, install, and permission surface', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    listenMock.mockResolvedValue(vi.fn());
    aiConfig.provider = 'openai';
  });

  it('sends the acp block inside config with saveAiSettings', async () => {
    invokeMock.mockResolvedValue(undefined);
    await saveAiSettings(
      {
        provider: 'acp',
        model: '',
        maxTokens: 4096,
        maxTurns: 50,
        rowLimit: 500,
        sampleColumnValues: true,
        enableBlastRadiusCheck: true,
        providerModels: {},
      },
      undefined,
      {
        agentId: 'opencode',
        command: null,
        env: {},
        autoDenyPermissions: false,
      },
    );
    const [name, args] = invokeMock.mock.calls[0];
    expect(name).toBe('save_ai_settings');
    expect(args.config.acp).toEqual({
      agentId: 'opencode',
      command: null,
      env: {},
      autoDenyPermissions: false,
    });
  });

  it('sends a null acp block when none is provided (backward compatible)', async () => {
    invokeMock.mockResolvedValue(undefined);
    await saveAiSettings({
      provider: 'openai',
      model: 'gpt-4o',
      maxTokens: 4096,
      maxTurns: 50,
      rowLimit: 500,
      sampleColumnValues: true,
      enableBlastRadiusCheck: true,
      providerModels: {},
    });
    const [, args] = invokeMock.mock.calls[0];
    expect(args.config.acp).toBeNull();
  });

  it('passes registry agent summaries through listRegistryAgents', async () => {
    const summary = {
      id: 'opencode',
      name: 'OpenCode',
      version: '1.2.3',
      description: 'terminal agent',
      license: 'MIT',
      icon: null,
      installedVersion: '1.2.3',
      updateAvailable: true,
    };
    invokeMock.mockResolvedValue([summary]);
    const result = await listRegistryAgents();
    expect(invokeMock).toHaveBeenCalledWith('list_registry_agents');
    expect(result).toEqual([summary]);
  });

  it('installs an acp agent and returns its launch spec', async () => {
    invokeMock.mockResolvedValue({
      id: 'opencode',
      version: '1.0.0',
      launch: { cmd: 'npx', args: ['-y', '@opencode/agent'], env: {} },
    });
    const result = await installAcpAgent('opencode');
    expect(invokeMock).toHaveBeenCalledWith('install_acp_agent', {
      agentId: 'opencode',
    });
    expect(result.launch.cmd).toBe('npx');
  });

  it('uninstalls an acp agent', async () => {
    invokeMock.mockResolvedValue(undefined);
    await uninstallAcpAgent('opencode');
    expect(invokeMock).toHaveBeenCalledWith('uninstall_acp_agent', {
      agentId: 'opencode',
    });
  });

  it('lists installed acp agents', async () => {
    const installed = {
      id: 'opencode',
      version: '1.0.0',
      launch: { cmd: 'npx', args: [], env: {} },
    };
    invokeMock.mockResolvedValue([installed]);
    const result = await listInstalledAcpAgents();
    expect(invokeMock).toHaveBeenCalledWith('list_installed_acp_agents');
    expect(result).toEqual([installed]);
  });

  it('answers an agent permission request with allow', async () => {
    invokeMock.mockResolvedValue(undefined);
    await respondAgentPermission('conv_1', true);
    expect(invokeMock).toHaveBeenCalledWith('respond_agent_permission', {
      conversationId: 'conv_1',
      allow: true,
    });
  });

  it('rejects a pending DML through the backend', async () => {
    invokeMock.mockResolvedValue(undefined);
    await rejectDml('conv_1');
    expect(invokeMock).toHaveBeenCalledWith('reject_dml', {
      conversationId: 'conv_1',
    });
  });

  it('pauses the conversation on ai:agent_permission and resumes after the response', () => {
    const conv = seedActiveConversationWithMessage('m1');
    const payload: AgentPermissionPayload = {
      conversationId: conv.id,
      title: 'Run a shell command',
      description: 'The agent wants to run: ls -la',
      options: [{ id: 'allow', name: 'Allow once' }],
    };

    pauseForPermission(conv.id, payload);
    const paused = getConv(conv.id);
    expect(paused.isPaused).toBe(true);
    expect(paused.pendingPermission).toEqual(payload);
    expect(
      paused.messages[paused.messages.length - 1].permissionRequest,
    ).toEqual(payload);

    // Resume on response — the pause lifts and the card clears (App.svelte
    // runs this after respond_agent_permission resolves).
    resumeFromPermission(conv.id);
    const resumed = getConv(conv.id);
    expect(resumed.isPaused).toBe(false);
    expect(resumed.pendingPermission).toBeNull();
    expect(
      resumed.messages[resumed.messages.length - 1].permissionRequest,
    ).toBeUndefined();
  });

  it('clears the pending DML card on dml:rejected (store helper)', () => {
    const conv = seedActiveConversationWithMessage('m1');
    const c0 = getConv(conv.id);
    c0.isPaused = true;
    c0.pausedDml = {
      sql: 'DELETE FROM t',
      description: 'Delete rows',
      estimatedRowsAffected: 3,
    };
    c0.dmlError = 'stale error';
    updateLast(conv.id, {
      dmlApproval: {
        sql: 'DELETE FROM t',
        description: 'Delete rows',
        estimatedRowsAffected: 3,
      },
    });

    clearRejectedDml(conv.id);
    const cleared = getConv(conv.id);
    expect(cleared.isPaused).toBe(false);
    expect(cleared.pausedDml).toBeNull();
    expect(cleared.dmlResult).toBeNull();
    expect(cleared.dmlError).toBeNull();
    expect(
      cleared.messages[cleared.messages.length - 1].dmlApproval,
    ).toBeUndefined();
  });

  it('rejects a pending DML through the backend in ACP mode and clears the card on dml:rejected', async () => {
    const conv = seedActiveConversationWithMessage('m1');
    const c0 = getConv(conv.id);
    c0.isPaused = true;
    c0.pausedDml = {
      sql: 'DELETE FROM t',
      description: 'Delete rows',
      estimatedRowsAffected: 3,
    };
    updateLast(conv.id, {
      dmlApproval: {
        sql: 'DELETE FROM t',
        description: 'Delete rows',
        estimatedRowsAffected: 3,
      },
    });
    aiConfig.provider = 'acp';

    const session = createAiSession(conv.id);
    await session.setupListeners({
      onDmlApproval: vi.fn(),
      onAgentPermission: vi.fn(),
      onError: vi.fn(),
    });

    await rejectPendingDml(conv.id);
    expect(invokeMock).toHaveBeenCalledWith('reject_dml', {
      conversationId: conv.id,
    });

    // The backend emits `dml:rejected` — the listener (single source of
    // truth for clearing on success) drops the card and unpauses.
    const cb = listenMock.mock.calls.find(
      ([event]) => event === 'dml:rejected',
    )?.[1] as (e: { payload: { conversation_id: string } }) => void;
    cb({ payload: { conversation_id: conv.id } });
    const cleared = getConv(conv.id);
    expect(cleared.isPaused).toBe(false);
    expect(cleared.pausedDml).toBeNull();
    expect(
      cleared.messages[cleared.messages.length - 1].dmlApproval,
    ).toBeUndefined();
    await session.cleanup();
  });

  it('keeps the rig-path cancel behavior for non-ACP providers', async () => {
    const conv = seedActiveConversationWithMessage('m1');
    const c0 = getConv(conv.id);
    c0.isPaused = true;
    c0.pausedDml = {
      sql: 'DELETE FROM t',
      description: 'Delete rows',
      estimatedRowsAffected: 3,
    };
    updateLast(conv.id, {
      dmlApproval: {
        sql: 'DELETE FROM t',
        description: 'Delete rows',
        estimatedRowsAffected: 3,
      },
    });
    aiConfig.provider = 'openai';

    await rejectPendingDml(conv.id);
    expect(invokeMock).toHaveBeenCalledWith('ai_cancel', {
      conversationId: conv.id,
    });
    const cleared = getConv(conv.id);
    expect(cleared.isPaused).toBe(false);
    expect(cleared.pausedDml).toBeNull();
    expect(
      cleared.messages[cleared.messages.length - 1].dmlApproval,
    ).toBeUndefined();
  });

  it('surfaces a reject_dml failure on the card and keeps it for retry', async () => {
    const conv = seedActiveConversationWithMessage('m1');
    const c0 = getConv(conv.id);
    c0.isPaused = true;
    c0.pausedDml = {
      sql: 'DELETE FROM t',
      description: 'Delete rows',
      estimatedRowsAffected: 3,
    };
    updateLast(conv.id, {
      dmlApproval: {
        sql: 'DELETE FROM t',
        description: 'Delete rows',
        estimatedRowsAffected: 3,
      },
    });
    aiConfig.provider = 'acp';
    invokeMock.mockRejectedValue(
      'No pending DML for this conversation (bridge not active)',
    );

    await rejectPendingDml(conv.id);
    const still = getConv(conv.id);
    expect(still.isPaused).toBe(true);
    expect(still.dmlError).toBe(
      'No pending DML for this conversation (bridge not active)',
    );
    expect(still.messages[still.messages.length - 1].dmlApproval).toBeDefined();
  });

  it('forwards dml:rejected only for the session conversation and clears the card', async () => {
    const conv = seedActiveConversationWithMessage('m1');
    const c0 = getConv(conv.id);
    c0.isPaused = true;
    c0.pausedDml = {
      sql: 'DELETE FROM t',
      description: 'Delete rows',
      estimatedRowsAffected: 3,
    };
    updateLast(conv.id, {
      dmlApproval: {
        sql: 'DELETE FROM t',
        description: 'Delete rows',
        estimatedRowsAffected: 3,
      },
    });

    const session = createAiSession(conv.id);
    await session.setupListeners({
      onDmlApproval: vi.fn(),
      onAgentPermission: vi.fn(),
      onError: vi.fn(),
    });
    const cb = listenMock.mock.calls.find(
      ([event]) => event === 'dml:rejected',
    )?.[1] as (e: { payload: { conversation_id: string } }) => void;
    expect(cb).toBeTruthy();

    // The backend emits the event globally — a payload for another
    // conversation must not touch this session's card.
    setRunPhase(conv.id, 'running');
    cb({ payload: { conversation_id: 'other_conv' } });
    expect(getConv(conv.id).isPaused).toBe(true);
    expect(chat.isStreaming).toBe(true);

    cb({ payload: { conversation_id: conv.id } });
    const cleared = getConv(conv.id);
    expect(cleared.isPaused).toBe(false);
    expect(cleared.pausedDml).toBeNull();
    expect(
      cleared.messages[cleared.messages.length - 1].dmlApproval,
    ).toBeUndefined();
    // dml:rejected is not terminal — the turn is still live.
    expect(chat.isStreaming).toBe(true);
    setRunPhase(conv.id, 'idle');
    await session.cleanup();
  });

  it('forwards ai:agent_permission payloads to the handler and keeps the turn running', async () => {
    const conv = seedActiveConversationWithMessage('m1');
    const payload: AgentPermissionPayload = {
      conversationId: conv.id,
      title: 'Run a shell command',
      description: 'The agent wants to run: ls -la',
      options: [{ id: 'allow', name: 'Allow once' }],
    };
    const onAgentPermission = vi.fn();
    const session = createAiSession(conv.id);
    await session.setupListeners({
      onDmlApproval: vi.fn(),
      onAgentPermission,
      onError: vi.fn(),
    });
    const cb = listenMock.mock.calls.find(
      ([event]) => event === 'ai:agent_permission',
    )?.[1] as (e: { payload: AgentPermissionPayload }) => void;
    expect(cb).toBeTruthy();

    setRunPhase(conv.id, 'running');
    cb({ payload });
    expect(onAgentPermission).toHaveBeenCalledWith(payload);
    // A permission pause is not terminal — the turn is still live.
    expect(chat.isStreaming).toBe(true);
    setRunPhase(conv.id, 'idle');
    await session.cleanup();
  });
});

describe('listAiModels', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('invokes list_ai_models with the given provider, key, and endpoint', async () => {
    invokeMock.mockResolvedValue([{ id: 'gpt-4o', displayName: 'gpt-4o' }]);
    const result = await listAiModels('openai', 'sk-test', undefined);
    expect(invokeMock).toHaveBeenCalledWith('list_ai_models', {
      provider: 'openai',
      apiKey: 'sk-test',
      endpoint: null,
    });
    expect(result).toEqual([{ id: 'gpt-4o', displayName: 'gpt-4o' }]);
  });

  it('sends null instead of an empty api key so the backend falls back to the saved key', async () => {
    invokeMock.mockResolvedValue([]);
    await listAiModels('anthropic', '', undefined);
    expect(invokeMock).toHaveBeenCalledWith('list_ai_models', {
      provider: 'anthropic',
      apiKey: null,
      endpoint: null,
    });
  });
});

describe('saveChatMessage', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('invokes save_chat_message with persisted chat message payload', async () => {
    invokeMock.mockResolvedValue(undefined);
    const msg = {
      id: 'msg-1',
      conversation_id: 'conv-1',
      role: 'assistant',
      content: 'Here is your result',
      session_json: '{"segments":[]}',
      created_at: 1700000000,
    };
    await saveChatMessage(msg);
    expect(invokeMock).toHaveBeenCalledWith('save_chat_message', {
      message: msg,
    });
  });
});

describe('sendMessage', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    chat.isStreaming = false;
    chat.error = null;
  });

  it('passes userMessageId and assistantMessageId to invoke ai_chat', async () => {
    invokeMock.mockResolvedValue(undefined);
    const channel = {} as any;
    const conv = createConversation('conn-1');
    conv.id = 'conv-1';
    chat.conversations = [conv];
    await sendMessage(
      'hello',
      channel,
      'conv-1',
      'conn-1',
      'profile-1',
      'user-msg-id',
      'asst-msg-id',
    );
    expect(invokeMock).toHaveBeenCalledWith('ai_chat', {
      message: 'hello',
      channel,
      conversationId: 'conv-1',
      connectionId: 'conn-1',
      profileId: 'profile-1',
      userMessageId: 'user-msg-id',
      assistantMessageId: 'asst-msg-id',
    });
    expect(getConv('conv-1').runPhase).toBe('running');
    expect(chat.isStreaming).toBe(true);
  });
});

describe('run phase transitions', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
    chat.isStreaming = false;
  });

  function doneEvent(convId: string, cancelled = false) {
    return {
      type: 'done',
      conversation_id: convId,
      final_message: 'ok',
      cancelled,
      usage: {
        prompt_tokens: 10,
        completion_tokens: 5,
        cached_prompt_tokens: 0,
      },
    } as const;
  }

  it('clears running to idle on done, per conversation', () => {
    const a = seedActiveConversationWithMessage('m1');
    const b = createConversation('conn-1');
    chat.conversations = [...chat.conversations, b];
    setRunPhase(a.id, 'running');
    setRunPhase(b.id, 'running');
    expect(chat.isStreaming).toBe(true);

    handleAiEvent(a.id, doneEvent(a.id));

    expect(getConv(a.id).runPhase).toBe('idle');
    // The other tab's parallel turn is untouched.
    expect(getConv(b.id).runPhase).toBe('running');
    expect(chat.isStreaming).toBe(true);
    expect(isConvRunning(a.id)).toBe(false);
    expect(isConvRunning(b.id)).toBe(true);
  });

  it('clears stopping to idle on a cancelled done', () => {
    const conv = seedActiveConversationWithMessage('m1');
    setRunPhase(conv.id, 'stopping');
    handleAiEvent(conv.id, doneEvent(conv.id, true));
    expect(getConv(conv.id).runPhase).toBe('idle');
    expect(chat.isStreaming).toBe(false);
  });

  it('clears the phase on ai:error', async () => {
    seedConversation('conv-a');
    const session = createAiSession('conv-a');
    await session.setupListeners({
      onDmlApproval: vi.fn(),
      onAgentPermission: vi.fn(),
      onError: vi.fn(),
    });
    setRunPhase('conv-a', 'stopping');
    // The live registration (listenerCbs is overwritten per setup; the
    // listen mock history holds stale closures from earlier tests).
    const cb = listenerCbs['ai:error'] as (e: {
      payload: { conversation_id: string; message: string };
    }) => void;
    cb({ payload: { conversation_id: 'conv-a', message: 'boom' } });
    expect(getConv('conv-a').runPhase).toBe('idle');
    await session.cleanup();
  });
});

describe('aiSessionFor registry', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
    listenMock.mockClear();
    for (const k of Object.keys(listenerCbs)) delete listenerCbs[k];
    chanHarness.chanReset();
  });

  const handlers = {
    onDmlApproval: () => {},
    onAgentPermission: () => {},
    onError: () => {},
  };

  it('creates a fresh channel per send while registering listeners once', async () => {
    seedConversation('conv-a');
    const first = aiSessionFor('conv-a');
    await first.ensureListeners(handlers);
    const second = aiSessionFor('conv-a');
    await second.ensureListeners(handlers);
    // Tauri Channels are single-use — each send gets its own channel.
    expect(second.createChannel()).not.toBe(first.createChannel());
    // Listeners registered exactly once despite two sends.
    expect(listenMock.mock.calls.filter(([e]) => e === 'ai:error')).toHaveLength(1);
    releaseAiSession('conv-a');
  });

  it('does not release a mid-turn session', async () => {
    seedConversation('conv-a');
    const s = aiSessionFor('conv-a');
    await s.ensureListeners(handlers);
    setRunPhase('conv-a', 'running');
    releaseAiSession('conv-a');
    // Still cached: its Done event must arrive to clear the phase, so
    // re-entry does not re-register listeners.
    await aiSessionFor('conv-a').ensureListeners(handlers);
    expect(listenMock.mock.calls.filter(([e]) => e === 'ai:error')).toHaveLength(1);
    setRunPhase('conv-a', 'idle');
    releaseAiSession('conv-a');
    // Released while idle: the next session registers listeners again.
    await aiSessionFor('conv-a').ensureListeners(handlers);
    expect(listenMock.mock.calls.filter(([e]) => e === 'ai:error')).toHaveLength(2);
    releaseAiSession('conv-a');
  });

  it('delivers turn-2 text and Done on a fresh channel after turn 1 ended', async () => {
    seedConversation('conv-turn2');
    invokeMock.mockResolvedValue(undefined);
    const { flushFrame, restoreRaf } = stubRaf();
    try {
      // ── turn 1 ──────────────────────────────────────────────────
      const s1 = aiSessionFor('conv-turn2');
      await s1.ensureListeners(handlers);
      addMessage('conv-turn2', {
        id: 'u1',
        role: 'user',
        content: 'hi',
        createdAt: Date.now(),
      });
      addMessage('conv-turn2', {
        id: 'a1',
        role: 'assistant',
        content: '',
        createdAt: Date.now(),
      });
      const ch1 = s1.createChannel();
      await sendMessage('hi', ch1, 'conv-turn2', 'conn-1', null, 'u1', 'a1');
      const tx1 = chanHarness.chanSender(ch1.id);
      tx1.send({ type: 'text', content: 'first reply' });
      flushFrame();
      tx1.send(turnDoneEvent('conv-turn2'));
      expect(getConv('conv-turn2').messages[1].content).toBe('first reply');
      expect(getConv('conv-turn2').runPhase).toBe('idle');

      // Rust replaces ConversationState.event_channel (commands.rs:2987) →
      // the turn-1 channel's on_drop eval runs.
      chanHarness.chanEnd(ch1.id);

      // ── turn 2 ──────────────────────────────────────────────────
      const s2 = aiSessionFor('conv-turn2');
      await s2.ensureListeners(handlers);
      addMessage('conv-turn2', {
        id: 'u2',
        role: 'user',
        content: 'oh great',
        createdAt: Date.now(),
      });
      addMessage('conv-turn2', {
        id: 'a2',
        role: 'assistant',
        content: '',
        createdAt: Date.now(),
      });
      const ch2 = s2.createChannel();
      await sendMessage(
        'oh great',
        ch2,
        'conv-turn2',
        'conn-1',
        null,
        'u2',
        'a2',
      );
      const tx2 = chanHarness.chanSender(ch2.id);
      tx2.send({
        type: 'text',
        content: 'glad it helped',
      });
      flushFrame();
      tx2.send(turnDoneEvent('conv-turn2'));

      // ── assertions ──────────────────────────────────────────────
      expect(chanHarness.chanDropped).toEqual([]);
      expect(getConv('conv-turn2').messages[3].content).toBe(
        'glad it helped',
      );
      expect(getConv('conv-turn2').runPhase).toBe('idle');
      expect(chat.isStreaming).toBe(false);
      // Listeners still registered exactly once across both turns.
      expect(listenMock.mock.calls.filter(([e]) => e === 'ai:error')).toHaveLength(1);
    } finally {
      restoreRaf();
      flushStreamPending();
      setRunPhase('conv-turn2', 'idle');
      releaseAiSession('conv-turn2');
    }
  });

  it('does not reuse a channel whose JS index counter has advanced', async () => {
    seedConversation('conv-turn2b');
    invokeMock.mockResolvedValue(undefined);
    const { flushFrame, restoreRaf } = stubRaf();
    try {
      const s1 = aiSessionFor('conv-turn2b');
      await s1.ensureListeners(handlers);
      addMessage('conv-turn2b', {
        id: 'u1',
        role: 'user',
        content: 'hi',
        createdAt: Date.now(),
      });
      addMessage('conv-turn2b', {
        id: 'a1',
        role: 'assistant',
        content: '',
        createdAt: Date.now(),
      });
      const ch1 = s1.createChannel();
      await sendMessage('hi', ch1, 'conv-turn2b', 'conn-1', null, 'u1', 'a1');
      const tx1 = chanHarness.chanSender(ch1.id);
      tx1.send({ type: 'text', content: 'first reply' });
      flushFrame();
      tx1.send(turnDoneEvent('conv-turn2b'));
      // NOTE: no chanEnd here — the JS index counter alone must not break
      // turn 2, so turn 2 needs a channel whose counter starts at 0.
      const s2 = aiSessionFor('conv-turn2b');
      await s2.ensureListeners(handlers);
      addMessage('conv-turn2b', {
        id: 'u2',
        role: 'user',
        content: 'again',
        createdAt: Date.now(),
      });
      addMessage('conv-turn2b', {
        id: 'a2',
        role: 'assistant',
        content: '',
        createdAt: Date.now(),
      });
      const ch2 = s2.createChannel();
      await sendMessage(
        'again',
        ch2,
        'conv-turn2b',
        'conn-1',
        null,
        'u2',
        'a2',
      );
      const tx2 = chanHarness.chanSender(ch2.id);
      tx2.send({ type: 'text', content: 'x' });
      flushFrame();
      tx2.send(turnDoneEvent('conv-turn2b'));

      expect(chanHarness.chanDropped).toEqual([]);
      expect(getConv('conv-turn2b').messages[3].content).toBe('x');
      expect(getConv('conv-turn2b').runPhase).toBe('idle');
    } finally {
      restoreRaf();
      flushStreamPending();
      setRunPhase('conv-turn2b', 'idle');
      releaseAiSession('conv-turn2b');
    }
  });
});

describe('stream coalescing', () => {
  beforeEach(() => {
    chat.conversations = [];
    chat.activeConversationId = null;
    flushStreamPending();
  });

  it('buffers text behind rAF and flushes in order before tool events', () => {
    const conv = seedActiveConversationWithMessage('m1');
    const rafCbs: FrameRequestCallback[] = [];
    const realRaf = globalThis.requestAnimationFrame;
    globalThis.requestAnimationFrame = (cb: FrameRequestCallback) => {
      rafCbs.push(cb);
      return rafCbs.length;
    };
    try {
      handleAiEvent(conv.id, { type: 'thinking', content: 'Reasoning' });
      handleAiEvent(conv.id, { type: 'text', content: 'Hel' });
      handleAiEvent(conv.id, { type: 'text', content: 'lo' });
      // Nothing applied yet — all buffered.
      expect(getConv(conv.id).messages[0].content).toBe('');
      // A tool event flushes the text first so segments stay ordered.
      handleAiEvent(conv.id, {
        type: 'tool_calls',
        tools: [{ id: 'call_1', name: 'search_schema', args: {} }],
      });
      const msg = getConv(conv.id).messages[0];
      // The flushed text was demoted into a note by the tool_calls event
      // (mid-work commentary), preserving arrival order.
      expect(msg.content).toBe('');
      expect(msg.session!.segments[0]).toMatchObject({
        type: 'thinking',
        content: 'Reasoning',
        streaming: false,
      });
      expect(msg.session!.segments[1]).toEqual({
        type: 'note',
        content: 'Hello',
      });
      expect(msg.session!.segments[2]).toMatchObject({
        type: 'tool_call',
        call: { name: 'search_schema' },
      });
    } finally {
      globalThis.requestAnimationFrame = realRaf;
      flushStreamPending();
    }
  });
});

