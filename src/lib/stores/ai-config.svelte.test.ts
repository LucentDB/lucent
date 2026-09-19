import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  aiConfig,
  getActiveAiModelDisplay,
  loadAiConfig,
} from './ai-config.svelte.ts';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

describe('aiConfig store', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    aiConfig.provider = 'openai';
    aiConfig.endpoint = '';
    aiConfig.model = 'gpt-4o';
    aiConfig.maxTokens = 4096;
    aiConfig.maxTurns = 50;
    aiConfig.rowLimit = 500;
    aiConfig.sampleColumnValues = true;
    aiConfig.enableBlastRadiusCheck = true;
    aiConfig.providerModels = {};
    aiConfig.acp = null;
  });

  describe('getActiveAiModelDisplay', () => {
    it('returns model for default openai provider', () => {
      aiConfig.provider = 'openai';
      aiConfig.model = 'gpt-4o';
      expect(getActiveAiModelDisplay()).toBe('gpt-4o');
    });

    it('returns ACP agent id when ACP provider is selected', () => {
      aiConfig.provider = 'acp';
      aiConfig.acp = {
        agentId: 'opencode',
        command: null,
        env: {},
        autoDenyPermissions: false,
      };
      expect(getActiveAiModelDisplay()).toBe('opencode');
    });

    it('returns claude-code when claude-code ACP agent is selected', () => {
      aiConfig.provider = 'acp';
      aiConfig.acp = {
        agentId: 'claude-code',
        command: null,
        env: {},
        autoDenyPermissions: false,
      };
      expect(getActiveAiModelDisplay()).toBe('claude-code');
    });

    it('falls back to ACP when ACP provider is selected but agentId is missing', () => {
      aiConfig.provider = 'acp';
      aiConfig.acp = null;
      expect(getActiveAiModelDisplay()).toBe('ACP');
    });

    it('returns provider name when opencode API is used and model is stale gpt-4o', () => {
      aiConfig.provider = 'opencode';
      aiConfig.model = 'gpt-4o';
      expect(getActiveAiModelDisplay()).toBe('opencode');
    });

    it('returns custom model when opencode API has a specific model configured', () => {
      aiConfig.provider = 'opencode';
      aiConfig.model = 'opencode-v1';
      expect(getActiveAiModelDisplay()).toBe('opencode-v1');
    });

    it('returns provider name when opencode API model is empty', () => {
      aiConfig.provider = 'opencode';
      aiConfig.model = '';
      expect(getActiveAiModelDisplay()).toBe('opencode');
    });

    it('returns provider name when anthropic provider has stale gpt-4o model', () => {
      aiConfig.provider = 'anthropic';
      aiConfig.model = 'gpt-4o';
      expect(getActiveAiModelDisplay()).toBe('anthropic');
    });

    it('returns configured model for anthropic', () => {
      aiConfig.provider = 'anthropic';
      aiConfig.model = 'claude-3-5-sonnet';
      expect(getActiveAiModelDisplay()).toBe('claude-3-5-sonnet');
    });
  });

  describe('loadAiConfig', () => {
    it('populates aiConfig from get_ai_settings IPC call', async () => {
      invokeMock.mockResolvedValue({
        provider: 'acp',
        model: 'gpt-4o',
        maxTokens: 8192,
        acp: {
          agentId: 'opencode',
          command: null,
          env: { FOO: 'BAR' },
          autoDenyPermissions: true,
        },
      });

      await loadAiConfig();

      expect(invokeMock).toHaveBeenCalledWith('get_ai_settings');
      expect(aiConfig.provider).toBe('acp');
      expect(aiConfig.maxTokens).toBe(8192);
      expect(aiConfig.acp?.agentId).toBe('opencode');
      expect(aiConfig.acp?.autoDenyPermissions).toBe(true);
      expect(getActiveAiModelDisplay()).toBe('opencode');
    });

    it('handles backend errors gracefully without throwing', async () => {
      invokeMock.mockRejectedValue(new Error('backend disconnected'));

      await expect(loadAiConfig()).resolves.toBeUndefined();
      expect(aiConfig.provider).toBe('openai');
    });
  });
});
