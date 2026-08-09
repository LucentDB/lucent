import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  chat,
  createConversation,
  addMessage,
  formatUsageLine,
} from '../stores/chat.svelte.ts';
import { handleAiEvent } from './ai.ts';

const invokeMock = vi.fn().mockResolvedValue(undefined);
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  Channel: class {},
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
        estimated_cost_usd: null,
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
      estimatedCostUsd: null,
      cachedPromptTokens: 0,
    });
  });

  it('fetches accumulated usage once on done and stores it on the conversation', async () => {
    const conv = seedActiveConversationWithMessage('m1');
    invokeMock.mockResolvedValue({
      prompt_tokens: 120,
      completion_tokens: 45,
      estimated_cost_usd: 0.1234,
      cached_prompt_tokens: 30,
    });
    handleAiEvent(conv.id, {
      type: 'done',
      conversation_id: conv.id,
      final_message: 'ok',
      usage: {
        prompt_tokens: 120,
        completion_tokens: 45,
        estimated_cost_usd: 0.1234,
        cached_prompt_tokens: 30,
      },
    });
    await vi.waitFor(() => {
      expect(getConv(conv.id).usage).toEqual({
        promptTokens: 120,
        completionTokens: 45,
        estimatedCostUsd: 0.1234,
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
        estimated_cost_usd: null,
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
  it('renders in/out tokens without cost when the provider reports none', () => {
    expect(
      formatUsageLine({
        promptTokens: 120,
        completionTokens: 45,
        estimatedCostUsd: null,
        cachedPromptTokens: 0,
      }),
    ).toBe('120 in / 45 out tokens');
  });

  it('appends the cost with 4-decimal precision when present', () => {
    expect(
      formatUsageLine({
        promptTokens: 120,
        completionTokens: 45,
        estimatedCostUsd: 0.1234,
        cachedPromptTokens: 0,
      }),
    ).toBe('120 in / 45 out tokens · $0.1234');
  });
});
