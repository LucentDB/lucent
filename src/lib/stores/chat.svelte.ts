import { invoke } from '@tauri-apps/api/core';
import type { ToolOutputPayload, AgentPermissionPayload } from '../ipc/ai.ts';

export interface TokenUsage {
  promptTokens: number;
  completionTokens: number;
  /** Prompt tokens served from the provider's prefix cache (0 = no cache hit). */
  cachedPromptTokens: number;
}

export interface ToolCallCard {
  id: string;
  name: string;
  args: unknown;
  summary: string | null;
  output?: ToolOutputPayload;
}

export type WorkSegment =
  | {
      type: 'thinking';
      content: string;
      streaming: boolean;
      startedAt: number;
      durationMs?: number;
    }
  | { type: 'note'; content: string }
  | { type: 'tool_call'; call: ToolCallCard };

export interface WorkSession {
  segments: WorkSegment[];
  startedAt: number;
  durationMs?: number;
  active: boolean;
  expanded?: boolean;
}

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  session?: WorkSession;
  dmlApproval?: {
    sql: string;
    description: string;
    estimatedRowsAffected: number | null;
  };
  /** The agent's tool-permission request awaiting an answer (ACP mode, E4). */
  permissionRequest?: AgentPermissionPayload;
  usage?: TokenUsage;
  createdAt: number;
}

export interface Conversation {
  id: string;
  connectionId: string;
  messages: ChatMessage[];
  isPaused: boolean;
  pausedDml: {
    sql: string;
    description: string;
    estimatedRowsAffected: number | null;
  } | null;
  /** The agent's tool-permission request awaiting an answer (ACP mode, E4). */
  pendingPermission: AgentPermissionPayload | null;
  /** Real affected row count from the executed DML (C1), shown on the card. */
  dmlResult: number | null;
  /** Error from the DML execution attempt (C1), shown on the card. */
  dmlError: string | null;
  usage: TokenUsage | null;
  createdAt: number;
}

function createChatStore() {
  let conversations = $state<Conversation[]>([]);
  let activeConversationId = $state<string | null>(null);
  let isStreaming = $state(false);
  let error = $state<string | null>(null);

  return {
    get conversations() {
      return conversations;
    },
    set conversations(v) {
      conversations = v;
    },
    get activeConversationId() {
      return activeConversationId;
    },
    set activeConversationId(v) {
      activeConversationId = v;
    },
    get isStreaming() {
      return isStreaming;
    },
    set isStreaming(v) {
      isStreaming = v;
    },
    get error() {
      return error;
    },
    set error(v) {
      error = v;
    },
  };
}

export const chat = createChatStore();

export function createConversation(connectionId: string): Conversation {
  return {
    id: crypto.randomUUID(),
    connectionId,
    messages: [],
    isPaused: false,
    pausedDml: null,
    pendingPermission: null,
    dmlResult: null,
    dmlError: null,
    usage: null,
    createdAt: Date.now(),
  };
}

export function getActive(): Conversation | undefined {
  return chat.conversations.find((c) => c.id === chat.activeConversationId);
}

export function createNewTab(connectionId: string): Conversation {
  const conv = createConversation(connectionId);
  chat.conversations = [...chat.conversations, conv];
  chat.activeConversationId = conv.id;
  return conv;
}

export function closeTab(convId: string) {
  const idx = chat.conversations.findIndex((c) => c.id === convId);
  if (idx === -1) return;
  chat.conversations = chat.conversations.filter((c) => c.id !== convId);
  if (chat.activeConversationId === convId) {
    chat.activeConversationId =
      chat.conversations.length > 0
        ? chat.conversations[Math.min(idx, chat.conversations.length - 1)].id
        : null;
  }
  // Backend keeps full ConversationState (history + query_cache) keyed by
  // this id until told otherwise — without this it never gets evicted.
  void invoke('close_conversation', { conversationId: convId }).catch(() => {});
}

export function switchTab(convId: string) {
  chat.activeConversationId = convId;
}

export function addMessage(convId: string, msg: ChatMessage) {
  const c = chat.conversations.find((c) => c.id === convId);
  if (c) c.messages = [...c.messages, msg];
}

export function appendToLast(convId: string, chunk: string) {
  const c = chat.conversations.find((c) => c.id === convId);
  if (c && c.messages.length > 0) {
    const last = c.messages[c.messages.length - 1];
    if (last.role === 'assistant') last.content += chunk;
  }
}

function findMessage(
  convId: string,
  messageId: string,
): ChatMessage | undefined {
  const c = chat.conversations.find((c) => c.id === convId);
  return c?.messages.find((m) => m.id === messageId);
}

function ensureSession(msg: ChatMessage): WorkSession {
  if (!msg.session) {
    msg.session = { segments: [], startedAt: Date.now(), active: true };
  }
  return msg.session;
}

export function appendToThinking(
  convId: string,
  messageId: string,
  chunk: string,
) {
  const msg = findMessage(convId, messageId);
  if (!msg || msg.role !== 'assistant') return;
  const session = ensureSession(msg);
  const last = session.segments[session.segments.length - 1];
  if (last && last.type === 'thinking' && last.streaming) {
    last.content += chunk;
  } else {
    session.segments.push({
      type: 'thinking',
      content: chunk,
      streaming: true,
      startedAt: Date.now(),
    });
  }
}

export function finalizeActiveThinkingSegment(
  convId: string,
  messageId: string,
) {
  const session = findMessage(convId, messageId)?.session;
  if (!session) return;
  const last = session.segments[session.segments.length - 1];
  if (last && last.type === 'thinking' && last.streaming) {
    last.streaming = false;
    last.durationMs = Date.now() - last.startedAt;
  }
}

export function demoteContentToNote(convId: string, messageId: string) {
  const msg = findMessage(convId, messageId);
  if (!msg || !msg.content) return;
  const session = ensureSession(msg);
  session.segments.push({ type: 'note', content: msg.content });
  msg.content = '';
}

/**
 * Appends a system note segment to the message's work session (rendered as
 * a note, not as agent text). Used for Lucent-originated notices such as
 * "database tools unavailable for this agent".
 */
export function addNote(convId: string, messageId: string, content: string) {
  const msg = findMessage(convId, messageId);
  if (!msg) return;
  const session = ensureSession(msg);
  session.segments.push({ type: 'note', content });
}

export function addToolCallSegments(
  convId: string,
  messageId: string,
  tools: { id: string; name: string; args: unknown }[],
) {
  const msg = findMessage(convId, messageId);
  if (!msg) return;
  const session = ensureSession(msg);
  for (const t of tools) {
    session.segments.push({
      type: 'tool_call',
      call: { id: t.id, name: t.name, args: t.args, summary: null },
    });
  }
}

export function updateToolResult(
  convId: string,
  messageId: string,
  toolId: string,
  update: { summary: string; output?: ToolCallCard['output'] },
) {
  const session = findMessage(convId, messageId)?.session;
  if (!session) return;
  for (const seg of session.segments) {
    if (seg.type === 'tool_call' && seg.call.id === toolId) {
      seg.call = {
        ...seg.call,
        summary: update.summary,
        output: update.output,
      };
    }
  }
}

export function finalizeSession(convId: string, messageId: string) {
  finalizeActiveThinkingSegment(convId, messageId);
  const session = findMessage(convId, messageId)?.session;
  if (!session || !session.active) return;
  session.active = false;
  session.durationMs = Date.now() - session.startedAt;
}

export function setSessionExpanded(
  convId: string,
  messageId: string,
  expanded: boolean,
) {
  const session = findMessage(convId, messageId)?.session;
  if (session) session.expanded = expanded;
}

export function updateLast(convId: string, update: Partial<ChatMessage>) {
  const c = chat.conversations.find((c) => c.id === convId);
  if (c && c.messages.length > 0)
    Object.assign(c.messages[c.messages.length - 1], update);
}

/** Pauses the conversation for an agent tool-permission request (E4): marks
 *  the pause, records the payload, and stamps the card on the last message. */
export function pauseForPermission(
  convId: string,
  payload: AgentPermissionPayload,
) {
  const conv = chat.conversations.find((c) => c.id === convId);
  if (!conv) return;
  conv.isPaused = true;
  conv.pendingPermission = payload;
  updateLast(convId, { permissionRequest: payload });
}

/** Clears the permission pause once the user answered (allow or reject). */
export function resumeFromPermission(convId: string) {
  const conv = chat.conversations.find((c) => c.id === convId);
  if (!conv) return;
  conv.isPaused = false;
  conv.pendingPermission = null;
  updateLast(convId, { permissionRequest: undefined });
}

/**
 * Clears a rejected DML preview: unpauses and drops the card (E5). Driven by
 * the `dml:rejected` event listener in ACP mode; the rig path clears inline
 * in App.svelte's cancel handler instead.
 */
export function clearRejectedDml(convId: string) {
  const conv = chat.conversations.find((c) => c.id === convId);
  if (!conv) return;
  conv.isPaused = false;
  conv.pausedDml = null;
  conv.dmlResult = null;
  conv.dmlError = null;
  updateLast(convId, { dmlApproval: undefined });
}

export function getConversationTitle(conv: Conversation): string {
  const firstUserMsg = conv.messages.find((m) => m.role === 'user');
  if (firstUserMsg) {
    const text = firstUserMsg.content.trim();
    if (text) {
      return text.length > 30 ? text.substring(0, 30) + '…' : text;
    }
  }
  return 'New Chat';
}

/** One-line usage summary for the panel header, e.g. `120 in / 45 out tokens`. */
export function formatUsageLine(usage: TokenUsage): string {
  return `${usage.promptTokens} in / ${usage.completionTokens} out tokens`;
}
