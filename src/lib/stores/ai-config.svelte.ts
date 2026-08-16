export type AiProviderId =
  | 'openai'
  | 'anthropic'
  | 'gemini'
  | 'openrouter'
  | 'mistral'
  | 'deepseek'
  | 'groq'
  | 'xai'
  | 'ollama'
  | 'custom'
  | 'opencode'
  | 'acp';

export const aiConfig = $state({
  provider: 'openai' as AiProviderId,
  endpoint: '',
  model: 'gpt-4o',
  maxTokens: 4096,
  maxTurns: 50,
  rowLimit: 500,
  sampleColumnValues: true,
  enableBlastRadiusCheck: true,
  providerModels: {} as Record<string, string>,
  /** ACP provider selection; `null` keeps the rig/provider-key path. */
  acp: null as {
    agentId: string;
    command: string | null;
    env: Record<string, string>;
    autoDenyPermissions: boolean;
  } | null,
});
