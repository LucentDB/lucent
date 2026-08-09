import { invoke, Channel } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  chat,
  appendToLast,
  appendToThinking,
  finalizeActiveThinkingSegment,
  demoteContentToNote,
  addToolCallSegments,
  updateToolResult,
  finalizeSession,
  updateLast,
  type TokenUsage,
} from '../stores/chat.svelte.ts';

export type ToolOutputPayload =
  | { type: 'text'; data: string }
  | {
      type: 'query_result';
      columns: { name: string; type: string }[];
      rows: unknown[][];
      row_count: number;
      sql: string;
      execution_time_ms: number;
      truncated: boolean;
    }
  | {
      type: 'dml_preview';
      sql: string;
      statement_type: string;
      tables_affected: string[];
      description: string;
      estimated_rows_affected: number | null;
    };

export type AiChannelEvent =
  | { type: 'thinking'; content: string }
  | { type: 'text'; content: string }
  | { type: 'tool_calls'; tools: { id: string; name: string; args: unknown }[] }
  | {
      type: 'tool_result';
      id: string;
      tool: string;
      summary: string;
      output: ToolOutputPayload | null;
    }
  | {
      type: 'done';
      conversation_id: string;
      final_message: string;
      usage: {
        prompt_tokens: number;
        completion_tokens: number;
        estimated_cost_usd: number | null;
        cached_prompt_tokens: number;
      };
    };

export interface DmlApprovalPayload {
  conversation_id: string;
  sql: string;
  description: string;
  estimated_rows_affected: number | null;
}

/**
 * Handles one AI channel event for a conversation. There is exactly one
 * message per user request (pre-seeded by handleAiSend in App.svelte before
 * any event arrives), so every event targets that conversation's last message
 * — no per-turn message bookkeeping is needed.
 */
export function handleAiEvent(conversationId: string, e: AiChannelEvent) {
  const conv = chat.conversations.find((c) => c.id === conversationId);
  if (!conv || conv.messages.length === 0) return;
  const messageId = conv.messages[conv.messages.length - 1].id;

  switch (e.type) {
    case 'thinking':
      appendToThinking(conversationId, messageId, e.content);
      break;
    case 'text':
      finalizeActiveThinkingSegment(conversationId, messageId);
      appendToLast(conversationId, e.content);
      break;
    case 'tool_calls':
      finalizeActiveThinkingSegment(conversationId, messageId);
      demoteContentToNote(conversationId, messageId);
      addToolCallSegments(conversationId, messageId, e.tools);
      break;
    case 'tool_result':
      updateToolResult(conversationId, messageId, e.id, {
        summary: e.summary,
        output: e.output ?? undefined,
      });
      break;
    case 'done':
      chat.isStreaming = false;
      finalizeSession(conversationId, messageId);
      updateLast(conversationId, {
        usage: {
          promptTokens: e.usage.prompt_tokens,
          completionTokens: e.usage.completion_tokens,
          estimatedCostUsd: e.usage.estimated_cost_usd,
          cachedPromptTokens: e.usage.cached_prompt_tokens,
        },
      });
      // One fetch per completed message — not continuous polling — to refresh
      // the header's conversation totals (the backend accumulates on Done).
      void refreshConversationUsage(conversationId);
      break;
  }
}

/** Raw shape the backend returns for `get_ai_usage` (snake_case via serde). */
export interface BackendTokenUsage {
  prompt_tokens: number;
  completion_tokens: number;
  estimated_cost_usd: number | null;
  cached_prompt_tokens: number;
}

/** Accumulated LLM token usage for a conversation (zeros if none yet). */
export async function getAiUsage(conversationId: string): Promise<TokenUsage> {
  const raw = await invoke<BackendTokenUsage>('get_ai_usage', {
    conversationId,
  });
  return {
    promptTokens: raw.prompt_tokens,
    completionTokens: raw.completion_tokens,
    estimatedCostUsd: raw.estimated_cost_usd,
    cachedPromptTokens: raw.cached_prompt_tokens,
  };
}

async function refreshConversationUsage(conversationId: string): Promise<void> {
  try {
    const usage = await getAiUsage(conversationId);
    const conv = chat.conversations.find((c) => c.id === conversationId);
    if (conv) conv.usage = usage;
  } catch {
    // Non-fatal — the header simply keeps its previous value.
  }
}

function finalizeLastMessageSession(conversationId: string) {
  const conv = chat.conversations.find((c) => c.id === conversationId);
  const last = conv?.messages[conv.messages.length - 1];
  if (last) finalizeSession(conversationId, last.id);
}

export function createAiSession(conversationId: string) {
  const channel = new Channel<AiChannelEvent>();
  const unlisteners: UnlistenFn[] = [];

  channel.onmessage = (e: AiChannelEvent) => handleAiEvent(conversationId, e);

  return {
    channel,
    setupListeners: async (handlers: {
      onDmlApproval: (p: DmlApprovalPayload) => void;
      onError: (p: { conversation_id: string; message: string }) => void;
    }) => {
      unlisteners.push(
        await listen<DmlApprovalPayload>('ai:dml_approval', (e) => {
          handlers.onDmlApproval(e.payload);
          chat.isStreaming = false;
          finalizeLastMessageSession(conversationId);
        }),
      );
      unlisteners.push(
        await listen<{ conversation_id: string; message: string }>(
          'ai:error',
          (e) => {
            handlers.onError(e.payload);
            chat.isStreaming = false;
            chat.error = e.payload.message;
            finalizeLastMessageSession(conversationId);
          },
        ),
      );
    },
    cleanup: async () => {
      for (const u of unlisteners) u();
    },
  };
}

export async function sendMessage(
  message: string,
  channel: Channel<AiChannelEvent>,
  conversationId: string,
  connectionId: string,
) {
  chat.isStreaming = true;
  chat.error = null;
  try {
    return await invoke('ai_chat', {
      message,
      channel,
      conversationId,
      connectionId,
    });
  } catch (e) {
    chat.isStreaming = false;
    chat.error = String(e);
    throw e;
  }
}

export async function cancelRun(conversationId: string) {
  return invoke('ai_cancel', { conversationId });
}

export async function executeDml(
  conversationId: string,
): Promise<{ rows_affected: number; sql: string }> {
  return invoke('execute_dml', { conversationId });
}

export async function saveAiSettings(
  config: {
    provider: string;
    endpoint?: string;
    model: string;
    maxTokens: number;
    maxTurns: number;
    rowLimit: number;
    sendResultsToAi: boolean;
    enableBlastRadiusCheck: boolean;
  },
  apiKey?: string,
) {
  return invoke('save_ai_settings', { config, apiKey: apiKey ?? null });
}

export async function getAiSettings() {
  return invoke<typeof import('../stores/ai-config.svelte.ts').aiConfig>(
    'get_ai_settings',
  );
}
