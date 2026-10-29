import { invoke } from '@tauri-apps/api/core';
import {
  listChatConversations,
  loadChatConversation,
  deleteChatConversation,
  saveChatMessage,
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
  /** Ids of the learned memory rules injected into this turn (F-C2), used by
   *  the attribution popover. Undefined when the turn predates attribution or
   *  the backend did not report per-rule ids. */
  appliedRuleIds?: string[];
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

export const OPEN_TABS_STORAGE_KEY = 'lucent-open-chat-ids';
export const ACTIVE_TAB_STORAGE_KEY = 'lucent-active-chat-id';

export function persistOpenChatTabs() {
  if (typeof localStorage === 'undefined') return;
  try {
    const ids = chat.conversations.map((c) => c.id);
    localStorage.setItem(OPEN_TABS_STORAGE_KEY, JSON.stringify(ids));
    if (chat.activeConversationId) {
      localStorage.setItem(ACTIVE_TAB_STORAGE_KEY, chat.activeConversationId);
    } else {
      localStorage.removeItem(ACTIVE_TAB_STORAGE_KEY);
    }
  } catch {
    // Ignore storage errors
  }
}

export function createNewTab(connectionId: string): Conversation {
  const conv = createConversation(connectionId);
  chat.conversations = [...chat.conversations, conv];
  chat.activeConversationId = conv.id;
  persistOpenChatTabs();
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
  persistOpenChatTabs();
  // Backend keeps full ConversationState (history + query_cache) keyed by
  // this id until told otherwise — without this it never gets evicted.
  void invoke('close_conversation', { conversationId: convId }).catch(() => {});
}

/**
 * Batch close helpers behind the chat tab context menu. Each delegates to
 * `closeTab`, so active-tab reassignment, localStorage persistence, and
 * backend state eviction behave exactly like clicking a single tab's ×.
 * The id list is snapshotted before closing so iteration is not affected
 * by the shrinking `chat.conversations`.
 */
export function closeOtherTabs(convId: string) {
  const others = chat.conversations
    .filter((c) => c.id !== convId)
    .map((c) => c.id);
  for (const id of others) closeTab(id);
}

export function closeTabsToRight(convId: string) {
  const ids = chat.conversations.map((c) => c.id);
  const idx = ids.indexOf(convId);
  if (idx === -1) return;
  for (const id of ids.slice(idx + 1)) closeTab(id);
}

export function closeTabsToLeft(convId: string) {
  const ids = chat.conversations.map((c) => c.id);
  const idx = ids.indexOf(convId);
  if (idx === -1) return;
  for (const id of ids.slice(0, idx)) closeTab(id);
}

export function closeAllTabs() {
  for (const id of chat.conversations.map((c) => c.id)) closeTab(id);
}

export function switchTab(convId: string) {
  chat.activeConversationId = convId;
  persistOpenChatTabs();
}

/**
 * Maps a row from `chat_messages` onto the runtime `ChatMessage` shape.
 *
 * The persisted shape (`PersistedChatMessage`) stores timestamps as Unix
 * *seconds* and the work session as an opaque `session_json` string; the
 * runtime shape uses Unix *milliseconds* and a structured `session`. F-I5
 * split the two types so this conversion is explicit rather than implicit.
 */
export function adaptPersistedMessage(p: PersistedChatMessage): ChatMessage {
  const message: ChatMessage = {
    id: p.id,
    role: p.role === 'assistant' ? 'assistant' : 'user',
    content: p.content,
    createdAt: p.created_at > 1e11 ? p.created_at : p.created_at * 1000,
  };
  if (p.session_json) {
    try {
      const session = JSON.parse(p.session_json) as WorkSession;
      if (session && Array.isArray(session.segments)) {
        session.active = false;
        for (const seg of session.segments) {
          if (seg.type === 'thinking') {
            seg.streaming = false;
          } else if (seg.type === 'tool_call' && seg.call) {
            if (!seg.call.status || seg.call.status === 'running') {
              seg.call.status =
                seg.call.summary === 'error' ||
                seg.call.summary?.startsWith('error')
                  ? 'failed'
                  : seg.call.summary
                    ? 'completed'
                    : 'stopped';
            }
          }
        }
        if (!p.content && session.expanded === undefined) {
          session.expanded = true;
        }
        message.session = session;
      }
    } catch {
      // A corrupt or legacy blob must not sink the rest of the hydration.
    }
  }
  return message;
}

/**
 * Persists a runtime ChatMessage from the chat store to SQLite via Tauri IPC.
 */
export async function persistConversationMessage(
  convId: string,
  messageId: string,
): Promise<void> {
  const msg = findMessage(convId, messageId);
  if (!msg) return;
  try {
    const persisted: PersistedChatMessage = {
      id: msg.id,
      conversation_id: convId,
      role: msg.role,
      content: msg.content,
      session_json: msg.session ? JSON.stringify(msg.session) : null,
      created_at: Math.floor(msg.createdAt / 1000),
    };
    await saveChatMessage(persisted);
  } catch (e) {
    console.error('Failed to persist conversation message:', e);
  }
}

/**
 * Restores conversations persisted in `memory.db` on app startup. Only the
 * conversations that were left open when the app was closed are restored as tabs.
 * Every restored conversation's messages are loaded eagerly, so each tab renders its
 * history (and derives its title) without a further round trip. Failures are
 * logged, never thrown: a missing or unreadable memory DB must not block boot.
 *
 * Call once at startup (App.svelte's `onMount`), not on every panel mount.
 */
export async function hydrateConversations(connectionId?: string) {
  try {
    const convs = await listChatConversations(connectionId);
    if (!convs || convs.length === 0) {
      chat.conversations = [];
      chat.activeConversationId = null;
      return;
    }

    const rawStored =
      typeof localStorage !== 'undefined'
        ? localStorage.getItem(OPEN_TABS_STORAGE_KEY)
        : null;

    let allowedIds: string[] | null = null;
    if (rawStored !== null) {
      try {
        const parsed = JSON.parse(rawStored);
        if (Array.isArray(parsed)) {
          allowedIds = parsed;
        }
      } catch {
        // Fall back to null if corrupt
      }
    }

    let toRestoreConvs: typeof convs = [];
    if (allowedIds !== null) {
      if (allowedIds.length === 0) {
        chat.conversations = [];
        chat.activeConversationId = null;
        return;
      }
      const allowedSet = new Set(allowedIds);
      toRestoreConvs = convs.filter((c) => allowedSet.has(c.id));
      toRestoreConvs.sort((a, b) => {
        const idxA = allowedIds!.indexOf(a.id);
        const idxB = allowedIds!.indexOf(b.id);
        return idxA - idxB;
      });
    } else {
      toRestoreConvs = convs;
    }

    if (toRestoreConvs.length === 0) {
      chat.conversations = [];
      chat.activeConversationId = null;
      persistOpenChatTabs();
      return;
    }

    const restored: Conversation[] = toRestoreConvs.map((c) => ({
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
    const storedActive =
      typeof localStorage !== 'undefined'
        ? localStorage.getItem(ACTIVE_TAB_STORAGE_KEY)
        : null;
    if (storedActive && restored.some((c) => c.id === storedActive)) {
      chat.activeConversationId = storedActive;
    } else {
      chat.activeConversationId = restored[0].id;
    }
    persistOpenChatTabs();
  } catch (e) {
    console.error('Failed to hydrate chat conversations:', e);
  }
}

/**
 * Opens a previous conversation into `chat.conversations` (if not already open)
 * and focuses it.
 */
export async function openConversation(
  convId: string,
): Promise<Conversation | undefined> {
  const existing = chat.conversations.find((c) => c.id === convId);
  if (existing) {
    if (!chat.isStreaming || chat.activeConversationId !== convId) {
      try {
        const persisted = await loadChatConversation(convId);
        if (persisted && persisted.length > 0) {
          existing.messages = persisted.map(adaptPersistedMessage);
        }
      } catch (e) {
        console.error('Failed to load messages for existing conversation:', e);
      }
    }
    chat.activeConversationId = existing.id;
    persistOpenChatTabs();
    return existing;
  }
  try {
    const persisted = await loadChatConversation(convId);
    const convs = await listChatConversations();
    const meta = convs.find((c) => c.id === convId);
    const conv: Conversation = {
      id: convId,
      connectionId: meta?.connection_id ?? '',
      messages: persisted.map(adaptPersistedMessage),
      isPaused: false,
      pausedDml: null,
      pendingPermission: null,
      dmlResult: null,
      dmlError: null,
      usage: null,
      error: null,
      createdAt: meta?.created_at ? meta.created_at * 1000 : Date.now(),
    };
    chat.conversations = [...chat.conversations, conv];
    chat.activeConversationId = conv.id;
    persistOpenChatTabs();
    return conv;
  } catch (e) {
    console.error('Failed to open conversation:', e);
  }
}

/**
 * Deletes a conversation from persistent storage and closes its tab if currently open.
 */
export async function deleteConversationHistory(
  convId: string,
): Promise<boolean> {
  try {
    const res = await deleteChatConversation(convId);
    if (chat.conversations.some((c) => c.id === convId)) {
      closeTab(convId);
    }
    return res;
  } catch (e) {
    console.error('Failed to delete conversation:', e);
    return false;
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
  const msg = findMessage(convId, messageId);
  if (msg?.session) {
    msg.session = { ...msg.session, expanded };
  }
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
