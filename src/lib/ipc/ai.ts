import { invoke, Channel } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  chat,
  appendToLast,
  appendToThinking,
  finalizeActiveThinkingSegment,
  demoteContentToNote,
  addNote,
  addToolCallSegments,
  updateToolResult,
  markStoppedToolCalls,
  finalizeSession,
  persistConversationMessage,
  updateLast,
  clearRejectedDml,
  type TokenUsage,
} from '../stores/chat.svelte.ts';
import { aiConfig } from '../stores/ai-config.svelte.ts';

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
  | { type: 'notice'; content: string }
  | { type: 'tool_calls'; tools: { id: string; name: string; args: unknown }[] }
  | {
      type: 'tool_result';
      id: string;
      tool: string;
      summary: string;
      status?: 'completed' | 'failed';
      output: ToolOutputPayload | null;
      /** Arguments the tool actually ran with, when the call itself reported
       *  none (an ACP agent reaching the DB tools through the `lucent-tool`
       *  CLI reports a shell command with no structured input). */
      input?: unknown;
    }
  | {
      type: 'done';
      conversation_id: string;
      final_message: string;
      cancelled: boolean;
      usage: {
        prompt_tokens: number;
        completion_tokens: number;
        cached_prompt_tokens: number;
      };
      /** Learned memory rules injected into this turn's prompt (F-C2).
       *  Backend `#[serde(default)]`, so older events may omit it. */
      applied_memory_count?: number;
    };

export interface DmlApprovalPayload {
  conversation_id: string;
  sql: string;
  description: string;
  estimated_rows_affected: number | null;
}

/** A registry agent as shown in the Settings UI (backend `RegistryAgentSummary`, camelCase). */
export interface RegistryAgentSummary {
  id: string;
  name: string;
  version: string;
  description: string;
  license: string;
  icon: string | null;
  installedVersion: string | null;
  updateAvailable: boolean;
  /** Whether this agent can use Lucent's database tools (curated verdict). */
  dbTools: 'supported' | 'unsupported' | 'unknown';
}

/** An installed ACP agent with its resolved launch spec (`InstalledAgent`). */
export interface InstalledAcpAgent {
  id: string;
  version: string;
  launch: { cmd: string; args: string[]; env: Record<string, string> };
  /** Display name from the registry manifest (older installs lack it). */
  name?: string | null;
}

/** ACP provider selection block — `AiConfig.acp` on the backend (camelCase). */
export interface AcpConfigBlock {
  agentId: string;
  command: string | null;
  env: Record<string, string>;
  autoDenyPermissions: boolean;
}

/** One permission option the agent offers for its tool request (`ai:agent_permission`). */
export interface AgentPermissionOption {
  id: string;
  name: string;
}

/** The `ai:agent_permission` payload: the agent asks the user to run one of ITS tools. */
export interface AgentPermissionPayload {
  conversationId: string;
  title: string;
  description: string;
  options: AgentPermissionOption[];
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
    case 'notice':
      // A system note from Lucent itself (e.g. DB tools unavailable) —
      // rendered as a note segment, not as an agent message.
      finalizeActiveThinkingSegment(conversationId, messageId);
      addNote(conversationId, messageId, e.content);
      break;
    case 'tool_calls':
      finalizeActiveThinkingSegment(conversationId, messageId);
      demoteContentToNote(conversationId, messageId);
      addToolCallSegments(conversationId, messageId, e.tools);
      break;
    case 'tool_result':
      updateToolResult(conversationId, messageId, e.id, {
        summary: e.summary,
        status: e.status,
        output: e.output ?? undefined,
        args: e.input,
      });
      break;
    case 'done':
      chat.isStreaming = false;
      finalizeSession(conversationId, messageId);
      if (e.cancelled) markStoppedToolCalls(conversationId, messageId);
      updateLast(conversationId, {
        usage: {
          promptTokens: e.usage.prompt_tokens,
          completionTokens: e.usage.completion_tokens,
          cachedPromptTokens: e.usage.cached_prompt_tokens,
        },
        rulesApplied: e.applied_memory_count ?? 0,
      });
      void persistConversationMessage(conversationId, messageId);
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
  if (last) {
    finalizeSession(conversationId, last.id);
    void persistConversationMessage(conversationId, last.id);
  }
}

export function createAiSession(conversationId: string) {
  const channel = new Channel<AiChannelEvent>();
  const unlisteners: UnlistenFn[] = [];

  channel.onmessage = (e: AiChannelEvent) => handleAiEvent(conversationId, e);

  return {
    channel,
    setupListeners: async (handlers: {
      onDmlApproval: (p: DmlApprovalPayload) => void;
      onAgentPermission: (p: AgentPermissionPayload) => void;
      onError: (p: { conversation_id: string; message: string }) => void;
    }) => {
      unlisteners.push(
        await listen<DmlApprovalPayload>('ai:dml_approval', (e) => {
          // Events are global; only this conversation's payloads may pause it.
          if (e.payload.conversation_id !== conversationId) return;
          handlers.onDmlApproval(e.payload);
          chat.isStreaming = false;
          finalizeLastMessageSession(conversationId);
        }),
      );
      unlisteners.push(
        await listen<AgentPermissionPayload>('ai:agent_permission', (e) => {
          if (e.payload.conversationId !== conversationId) return;
          handlers.onAgentPermission(e.payload);
          chat.isStreaming = false;
          finalizeLastMessageSession(conversationId);
        }),
      );
      unlisteners.push(
        await listen<{ conversation_id: string }>('dml:rejected', (e) => {
          // The backend emits this globally for the conversation whose bridge
          // rejected the preview — only touch that conversation's card.
          if (e.payload.conversation_id !== conversationId) return;
          chat.isStreaming = false;
          clearRejectedDml(conversationId);
          finalizeLastMessageSession(conversationId);
        }),
      );
      unlisteners.push(
        await listen<{ conversation_id: string; message: string }>(
          'ai:error',
          (e) => {
            if (e.payload.conversation_id !== conversationId) return;
            handlers.onError(e.payload);
            chat.isStreaming = false;
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
  profileId?: string | null,
  userMessageId?: string | null,
  assistantMessageId?: string | null,
) {
  chat.isStreaming = true;
  chat.error = null;
  try {
    return await invoke('ai_chat', {
      message,
      channel,
      conversationId,
      connectionId,
      profileId: profileId ?? null,
      userMessageId: userMessageId ?? null,
      assistantMessageId: assistantMessageId ?? null,
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
    sampleColumnValues: boolean;
    enableBlastRadiusCheck: boolean;
    providerModels: Record<string, string>;
  },
  apiKey?: string,
  acp?: AcpConfigBlock | null,
) {
  return invoke('save_ai_settings', {
    config: { ...config, acp: acp ?? null },
    apiKey: apiKey ?? null,
  });
}

export async function getAiSettings() {
  return invoke<typeof import('../stores/ai-config.svelte.ts').aiConfig>(
    'get_ai_settings',
  );
}

export interface AiModelSummary {
  id: string;
  displayName: string;
}

export async function listAiModels(
  provider: string,
  apiKey: string | undefined,
  endpoint: string | undefined,
): Promise<AiModelSummary[]> {
  return invoke<AiModelSummary[]>('list_ai_models', {
    provider,
    apiKey: apiKey || null,
    endpoint: endpoint || null,
  });
}

/** Lists the agent registry merged with installed state (never fails — backend falls back to snapshot). */
export async function listRegistryAgents(): Promise<RegistryAgentSummary[]> {
  return invoke<RegistryAgentSummary[]>('list_registry_agents');
}

/** Installs a registry agent and returns its resolved launch spec. */
export async function installAcpAgent(
  agentId: string,
): Promise<InstalledAcpAgent> {
  return invoke<InstalledAcpAgent>('install_acp_agent', { agentId });
}

/** Removes an installed agent (idempotent). */
export async function uninstallAcpAgent(agentId: string): Promise<void> {
  return invoke('uninstall_acp_agent', { agentId });
}

/**
 * Lists every installed agent from disk — independent of the registry, so an
 * agent installed via command override (or whose registry entry vanished)
 * still shows up in the provider picker.
 */
export async function listInstalledAcpAgents(): Promise<InstalledAcpAgent[]> {
  return invoke<InstalledAcpAgent[]>('list_installed_acp_agents');
}

/** Answers the agent's `session/request_permission` for a conversation. */
export async function respondAgentPermission(
  conversationId: string,
  allow: boolean,
): Promise<void> {
  return invoke('respond_agent_permission', { conversationId, allow });
}

/**
 * Rejects a bridge-held `preview_dml` (ACP mode). On success the backend
 * emits the `dml:rejected` event with a SNAKE_CASE payload
 * (`{ conversation_id }`) — listeners must check the id, since the event is
 * emitted globally for the conversation whose bridge rejected the preview.
 */
export async function rejectDml(conversationId: string): Promise<void> {
  return invoke('reject_dml', { conversationId });
}

/** Unwraps a Tauri reject into the user-facing string (mirrors App.svelte's formatError). */
function errorMessage(e: unknown): string {
  if (typeof e === 'object' && e !== null && 'message' in e) {
    return String((e as { message: unknown }).message);
  }
  return typeof e === 'string' ? e : 'Unknown error';
}

/**
 * The DML card's Cancel action (E5). In ACP mode the preview is held by the
 * agent's DB-tools bridge — reject it through the backend; the `dml:rejected`
 * event (listener in `createAiSession`) is the single source of truth for
 * clearing the card on success, so failures are surfaced on the card and it
 * stays for retry. On the rig path this cancels the whole turn and clears
 * locally, preserving the pre-ACP behavior.
 */
export async function rejectPendingDml(conversationId: string): Promise<void> {
  if (aiConfig.provider === 'acp') {
    try {
      await rejectDml(conversationId);
    } catch (e) {
      const conv = chat.conversations.find((c) => c.id === conversationId);
      if (conv) conv.dmlError = errorMessage(e);
    }
    return;
  }
  await cancelRun(conversationId);
  const conv = chat.conversations.find((c) => c.id === conversationId);
  if (conv) {
    conv.isPaused = false;
    conv.pausedDml = null;
    conv.dmlResult = null;
    conv.dmlError = null;
    updateLast(conversationId, { dmlApproval: undefined });
  }
}

// ── AI Memory Subsystem IPC ──────────────────────────────────────────────────

export interface MemoryItem {
  id: string;
  connection_key: string;
  scope: 'global' | 'connection' | 'schema';
  scope_key?: string | null;
  category: 'metric' | 'join' | 'quirk' | 'preference' | 'playbook';
  key_phrase: string;
  rule_text: string;
  sql_snippet?: string | null;
  importance: number;
  stability_hours: number;
  last_accessed_at: number;
  access_count: number;
  source_trust:
    | 'user_explicit'
    | 'verified_consolidation'
    | 'error_resolution'
    | 'untrusted_tool_result';
  source_conv_id?: string | null;
  source_turn_id?: string | null;
  source_tool_id?: string | null;
  status: 'active' | 'archived' | 'stale_invalid' | 'superseded';
  supersedes_id?: string | null;
  valid_from: number;
  valid_until?: number | null;
  learned_at: number;
  tombstone: boolean;
  tombstoned_at?: number | null;
  doc_hash: string;
  embedding_model: string;
  embedding_version: number;
  created_at: number;
  updated_at: number;
  // Profile / playbook plane fields (Task 15). Optional so older fixtures and
  // list responses that predate the migration still type-check; the backend
  // always serializes them (Rust snake_case field names).
  injection?: 'always' | 'retrieved';
  preference_key?: string | null;
  origin?: 'owner' | 'agent' | 'untrusted' | 'system';
  steps_json?: string | null;
  merge_group_id?: string | null;
  confirmed?: boolean;
  confirmation_conv_id?: string | null;
}

/**
 * A row from `memory_observations` (Task 15). Backs the Learning Journal tab:
 * the raw signals Lucent logged before they were distilled into memories.
 */
export interface Observation {
  id: string;
  connection_key: string;
  conversation_id?: string | null;
  turn_id?: string | null;
  kind: string;
  origin: 'owner' | 'agent' | 'untrusted' | 'system';
  signal: string;
  signal_strength: number;
  occurrence_count: number;
  dedup_key: string;
  payload_json: string;
  status: string;
  derived_memory_id?: string | null;
  created_at: number;
  updated_at: number;
}

export interface GoldenQuery {
  id: string;
  connection_id: string;
  schema_name: string;
  natural_prompt: string;
  sql_text: string;
  tables_used: string[];
  verified: boolean;
  run_count: number;
  last_run_at: number;
  embedding_model: string;
  embedding_version: number;
  created_at: number;
}

export interface ChatConversation {
  id: string;
  connection_id: string;
  title: string;
  archived: boolean;
  created_at: number;
  updated_at: number;
}

/** A row from `chat_messages` in `memory.db`. Distinct from the runtime
 *  `ChatMessage` in `stores/chat.svelte.ts`: timestamps are Unix seconds and
 *  the work session is an opaque JSON string. */
export interface PersistedChatMessage {
  id: string;
  conversation_id: string;
  role: string;
  content: string;
  session_json?: string | null;
  created_at: number;
}

export interface DriftAlert {
  /** Frontend memory connection key the invalidated rule belongs to. Present on
   * alerts emitted by the background indexer's `memory:drift_detected` event;
   * optional for older fixtures/consolidation payloads. */
  connection_key?: string;
  memory_id: string;
  rule_text: string;
  reason: string;
  schema_name: string;
  table_name: string;
  column_name?: string | null;
}

export interface ConsolidationReport {
  pruned_session_json_count: number;
  archived_memory_count: number;
  drift_alerts: DriftAlert[];
}

export async function listChatConversations(
  connectionId?: string,
): Promise<ChatConversation[]> {
  return invoke('list_chat_conversations', { connectionId });
}

export async function loadChatConversation(
  conversationId: string,
): Promise<PersistedChatMessage[]> {
  return invoke('load_chat_conversation', { conversationId });
}

export async function saveChatMessage(
  message: PersistedChatMessage,
): Promise<void> {
  return invoke('save_chat_message', { message });
}

export async function deleteChatConversation(
  conversationId: string,
): Promise<boolean> {
  return invoke('delete_chat_conversation', { conversationId });
}

export async function listMemories(
  connectionKey: string,
  includeArchived = false,
): Promise<MemoryItem[]> {
  return invoke('list_memories', { connectionKey, includeArchived });
}

/**
 * Always-on profile memories for a connection (Task 15, R23).
 */
export async function listAlwaysMemories(
  connectionKey: string,
): Promise<MemoryItem[]> {
  return invoke('list_always_memories', { connectionKey });
}

/**
 * Raw observations for a connection (Task 15, R23). Defaults to the open
 * observation inbox; pass an explicit status to inspect another bucket.
 */
export async function listObservations(
  connectionKey: string,
  status = 'open',
): Promise<Observation[]> {
  return invoke('list_observations', { connectionKey, status });
}

export async function saveMemoryManual(
  connectionKey: string,
  category: string,
  keyPhrase: string,
  ruleText: string,
  sqlSnippet?: string,
  scope?: string,
): Promise<MemoryItem> {
  return invoke('save_memory_manual', {
    connectionKey,
    category,
    keyPhrase,
    ruleText,
    sqlSnippet,
    scope,
  });
}

export async function deleteMemory(id: string): Promise<boolean> {
  return invoke('delete_memory', { id });
}

export async function toggleMemoryStatus(
  id: string,
  status: string,
): Promise<void> {
  return invoke('toggle_memory_status', { id, status });
}

export async function resolveDrift(
  id: string,
  resolution: 'revalidate' | 'dismiss',
): Promise<void> {
  return invoke('resolve_drift', { id, resolution });
}

export async function exportMemoriesMarkdown(
  connectionKey: string,
): Promise<string> {
  return invoke('export_memories_markdown', { connectionKey });
}

export async function importMemoriesMarkdown(
  connectionKey: string,
  content: string,
): Promise<number> {
  return invoke('import_memories_markdown', { connectionKey, content });
}

export async function listGoldenQueries(
  connectionId: string,
): Promise<GoldenQuery[]> {
  return invoke('list_golden_queries', { connectionId });
}

export async function saveGoldenQuery(
  connectionId: string,
  naturalPrompt: string,
  sqlText: string,
  schemaName?: string,
  tablesUsed?: string[],
  verified?: boolean,
): Promise<GoldenQuery> {
  return invoke('save_golden_query', {
    connectionId,
    schemaName,
    naturalPrompt,
    sqlText,
    tablesUsed,
    verified,
  });
}

export async function deleteGoldenQuery(id: string): Promise<boolean> {
  return invoke('delete_golden_query', { id });
}

export async function runConsolidation(
  connectionKey?: string,
): Promise<ConsolidationReport> {
  return invoke('run_consolidation', { connectionKey });
}
