import { invoke } from '@tauri-apps/api/core';

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

export interface AiConfigState {
  provider: AiProviderId;
  endpoint: string;
  model: string;
  maxTokens: number;
  maxTurns: number;
  rowLimit: number;
  sampleColumnValues: boolean;
  enableBlastRadiusCheck: boolean;
  providerModels: Record<string, string>;
  acp: {
    agentId: string;
    command: string | null;
    env: Record<string, string>;
    autoDenyPermissions: boolean;
  } | null;
}

export const AI_CONFIG_STORAGE_KEY = 'lucent-ai-config';

function getInitialAiConfig(): AiConfigState {
  const defaults: AiConfigState = {
    provider: 'openai',
    endpoint: '',
    model: 'gpt-4o',
    maxTokens: 4096,
    maxTurns: 50,
    rowLimit: 500,
    sampleColumnValues: true,
    enableBlastRadiusCheck: true,
    providerModels: {},
    acp: null,
  };

  if (typeof localStorage !== 'undefined') {
    try {
      const stored = localStorage.getItem(AI_CONFIG_STORAGE_KEY);
      if (stored) {
        const parsed = JSON.parse(stored);
        if (parsed && typeof parsed === 'object') {
          return {
            ...defaults,
            ...parsed,
          };
        }
      }
    } catch {}
  }
  return defaults;
}

export const aiConfig = $state<AiConfigState>(getInitialAiConfig());

export function persistAiConfig(): void {
  if (typeof localStorage === 'undefined') return;
  try {
    localStorage.setItem(
      AI_CONFIG_STORAGE_KEY,
      JSON.stringify({
        provider: aiConfig.provider,
        endpoint: aiConfig.endpoint,
        model: aiConfig.model,
        maxTokens: aiConfig.maxTokens,
        maxTurns: aiConfig.maxTurns,
        rowLimit: aiConfig.rowLimit,
        sampleColumnValues: aiConfig.sampleColumnValues,
        enableBlastRadiusCheck: aiConfig.enableBlastRadiusCheck,
        providerModels: aiConfig.providerModels,
        acp: aiConfig.acp,
      }),
    );
  } catch {}
}

export function getActiveAiModelDisplay(): string {
  if (aiConfig.provider === 'acp' || (!aiConfig.provider && aiConfig.acp)) {
    return aiConfig.acp?.agentId || 'ACP';
  }
  // When using non-OpenAI providers (e.g. opencode, deepseek, mistral), a lingering
  // 'gpt-4o' value is a stale initial default from memory, not an active model.
  if (
    aiConfig.provider &&
    aiConfig.provider !== 'openai' &&
    aiConfig.provider !== 'custom' &&
    aiConfig.provider !== 'openrouter' &&
    aiConfig.model === 'gpt-4o'
  ) {
    return aiConfig.providerModels[aiConfig.provider] || aiConfig.provider;
  }
  return aiConfig.model || aiConfig.provider || '';
}

export async function loadAiConfig(): Promise<void> {
  try {
    const cfg = (await invoke('get_ai_settings')) as any;
    if (cfg) {
      aiConfig.provider = cfg.provider ?? aiConfig.provider;
      aiConfig.endpoint = cfg.endpoint ?? aiConfig.endpoint;
      aiConfig.model = cfg.model ?? aiConfig.model;
      aiConfig.maxTokens = cfg.maxTokens ?? aiConfig.maxTokens;
      aiConfig.maxTurns = cfg.maxTurns ?? aiConfig.maxTurns;
      aiConfig.rowLimit = cfg.rowLimit ?? aiConfig.rowLimit;
      aiConfig.sampleColumnValues =
        cfg.sampleColumnValues ?? aiConfig.sampleColumnValues;
      aiConfig.enableBlastRadiusCheck =
        cfg.enableBlastRadiusCheck ?? aiConfig.enableBlastRadiusCheck;
      aiConfig.providerModels =
        cfg.providerModels && Object.keys(cfg.providerModels).length > 0
          ? cfg.providerModels
          : aiConfig.providerModels;
      aiConfig.acp = cfg.acp ?? null;
      persistAiConfig();
    }
  } catch {
    // Backend may not be available in headless test environments
  }
}
