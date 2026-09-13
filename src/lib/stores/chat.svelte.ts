import { invoke } from '@tauri-apps/api/core';
import {
  listChatConversations,
  loadChatConversation,
  type ToolOutputPayload,
  type AgentPermissionPayload,
  type PersistedChatMessage,
} from '../ipc/ai.ts';
import { argsMissing } from './tool-args.ts';

export interface TokenUsage {
  promptTokens: number;
  completionTokens: number;
  /** Prompt tokens served from the provider's prefix cache (0 = no cache hit). */
  cachedPromptTokens: number;
}

/** Explicit lifecycle status of a tool call (spec D7). `stopped` is set
 *  when the turn ends cancelled before the call resolved. */
export type ToolCallStatus = 'running' | 'completed' | 'failed' | 'stopped';

export interface ToolCallCard {
  id: string;
  name: string;
  args: unknown;
  summary: string | null;
  output?: ToolOutputPayload;
  status?: ToolCallStatus;
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
  /** How many learned memory rules were injected into this turn's system
   *  prompt (F-C2). 0/undefined hides the memory pill. On ACP follow-up turns
   *  this is 0 by design — only DELIVERED rules count. */
  rulesApplied?: number;
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
  /** Turn-level error for THIS conversation (ai:error), rendered in ChatPanel. */
  error: string | null;
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
    error: null,
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

/**
 * Maps a row from `chat_messages` onto the runtime `ChatMessage` shape.
 *
 * The persisted shape (`PersistedChatMessage`) stores timestamps as Unix
 * *seconds* and the work session as an opaque `session_json` string; the
 * runtime shape uses Unix *milliseconds* and a structured `session`. F-I5
 * split the two types so this conversion is explicit rather than implicit.
 */
function adaptPersistedMessage(p: PersistedChatMessage): ChatMessage {
  const message: ChatMessage = {
    id: p.id,
    role: p.role === 'assistant' ? 'assistant' : 'user',
    content: p.content,
    createdAt: p.created_at * 1000,
  };
  if (p.session_json) {
    try {
      const session = JSON.parse(p.session_json) as WorkSession;
      if (session && Array.isArray(session.segments)) message.session = session;
    } catch {
      // A corrupt or legacy blob must not sink the rest of the hydration.
    }
  }
  return message;
}

/**
 * Restores conversations persisted in `memory.db` on app startup. Every
 * restored conversation's messages are loaded eagerly, so each tab renders its
 * history (and derives its title) without a further round trip. Failures are
 * logged, never thrown: a missing or unreadable memory DB must not block boot.
 *
 * Call once at startup (App.svelte's `onMount`), not on every panel mount.
 */
export async function hydrateConversations(connectionId?: string) {
  try {
    const convs = await listChatConversations(connectionId);
    if (!convs || convs.length === 0) return;
    const restored: Conversation[] = convs.map((c) => ({
      id: c.id,
      connectionId: c.connection_id,
      messages: [],
      isPaused: false,
      pausedDml: null,
      pendingPermission: null,
      dmlResult: null,
      dmlError: null,
      usage: null,
      error: null,
      createdAt: c.created_at * 1000,
    }));
    await Promise.all(
      restored.map(async (conv) => {
        const persisted = await loadChatConversation(conv.id);
        conv.messages = persisted.map(adaptPersistedMessage);
      }),
    );
    chat.conversations = restored;
    chat.activeConversationId = restored[0].id;
  } catch (e) {
    console.error('Failed to hydrate chat conversations:', e);
  }
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
  update: {
    summary: string;
    status?: ToolCallStatus;
    output?: ToolCallCard['output'];
    /** Backfilled arguments — only replaces what the card already has when
     *  the call reported none (the ACP CLI path; see `AiEvent::ToolResult`). */
    args?: unknown;
  },
) {
  const session = findMessage(convId, messageId)?.session;
  if (!session) return;
  for (const seg of session.segments) {
    if (seg.type === 'tool_call' && seg.call.id === toolId) {
      seg.call = {
        ...seg.call,
        args:
          argsMissing(seg.call.args) && update.args !== undefined
            ? update.args
            : seg.call.args,
        summary: update.summary,
        output: update.output,
        // Explicit status wins; legacy events (the rig path pre-status)
        // fall back to the error-prefix convention.
        status:
          update.status ??
          (update.summary === 'error' || update.summary.startsWith('error')
            ? 'failed'
            : 'completed'),
      };
    }
  }
}

/** Marks every unresolved tool call as `stopped` — called when a turn ends
 *  with `stopReason: cancelled` so cards never lie about their state (D7). */
export function markStoppedToolCalls(convId: string, messageId: string) {
  const session = findMessage(convId, messageId)?.session;
  if (!session) return;
  for (const seg of session.segments) {
    if (
      seg.type === 'tool_call' &&
      (seg.call.status === undefined || seg.call.status === 'running')
    ) {
      seg.call = { ...seg.call, status: 'stopped' };
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
