import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  chat,
  createConversation,
  addMessage,
  formatUsageLine,
  pauseForPermission,
  resumeFromPermission,
  clearRejectedDml,
  updateLast,
} from '../stores/chat.svelte.ts';
import {
  handleAiEvent,
  createAiSession,
  listAiModels,
  listRegistryAgents,
  installAcpAgent,
  uninstallAcpAgent,
  listInstalledAcpAgents,
  respondAgentPermission,
  rejectDml,
  rejectPendingDml,
  saveAiSettings,
  type AgentPermissionPayload,
} from './ai.ts';
import { aiConfig } from '../stores/ai-config.svelte.ts';

const invokeMock = vi.fn().mockResolvedValue(undefined);
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  Channel: class {},
}));

const listenMock = vi.fn();
vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => listenMock(...args),
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

// Re-fetch conversation from store — $state creates internal copies, so
// the original `conv` reference doesn't see mutations made by store functions.
function getConv(id: string) {
  return chat.conversations.find((c) => c.id === id)!;
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
    });
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
      });
    });
    expect(invokeMock).toHaveBeenCalledWith('get_ai_usage', {
      conversationId: conv.id,
    });
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('keeps the previous usage when the get_ai_usage fetch fails', async () => {
    const conv = seedActiveConversationWithMessage('m1');
    invokeMock.mockRejectedValue(new Error('nope'));
    handleAiEvent(conv.id, {
      type: 'done',
      conversation_id: conv.id,
      final_message: 'ok',
      usage: {
        prompt_tokens: 10,
        completion_tokens: 5,
        cached_prompt_tokens: 0,
      },
    });
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledTimes(1);
    });
    expect(getConv(conv.id).usage).toBeNull();
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
      call: { summary: string | null };
    };
    const seg2 = session.segments[1] as {
      type: 'tool_call';
      call: { summary: string | null };
    };
    expect(seg1.call.summary).toBeNull();
    expect(seg2.call.summary).toBe('4 rows');
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
    chat.isStreaming = true;
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
    expect(chat.isStreaming).toBe(false);
    await session.cleanup();
  });

  it('forwards ai:agent_permission payloads to the handler and stops streaming', async () => {
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

    chat.isStreaming = true;
    cb({ payload });
    expect(onAgentPermission).toHaveBeenCalledWith(payload);
    expect(chat.isStreaming).toBe(false);
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
