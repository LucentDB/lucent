import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import {
  render,
  screen,
  fireEvent,
  cleanup,
  waitFor,
} from '@testing-library/svelte';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  Channel: class {},
}));

import AiSettings from './AiSettings.svelte';
import { aiConfig } from '../../stores/ai-config.svelte.ts';

afterEach(cleanup);

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd) => {
    if (cmd === 'get_ai_settings') return { ...aiConfig };
    return undefined;
  });
  aiConfig.provider = 'openai';
  aiConfig.model = 'gpt-4o';
  aiConfig.endpoint = '';
  aiConfig.providerModels = {};
  aiConfig.acp = null;
});

describe('AiSettings', () => {
  it('remembers the last-picked model per provider when switching back and forth', async () => {
    render(AiSettings, { onClose: vi.fn() });
    aiConfig.providerModels = { anthropic: 'claude-sonnet-5' };

    await fireEvent.click(screen.getByRole('radio', { name: 'OpenAI' }));
    await fireEvent.click(screen.getByRole('radio', { name: 'Anthropic' }));

    expect(aiConfig.model).toBe('claude-sonnet-5');
  });

  it('shows the endpoint field only for Ollama and Custom', async () => {
    render(AiSettings, { onClose: vi.fn() });
    expect(screen.queryByLabelText(/endpoint/i)).toBeNull();

    await fireEvent.click(screen.getByRole('radio', { name: 'OpenAI' }));
    await fireEvent.click(
      screen.getByRole('radio', { name: 'Ollama (local)' }),
    );
    const endpointInput = screen.getByLabelText(
      /endpoint/i,
    ) as HTMLInputElement;
    expect(endpointInput).toBeTruthy();
    expect(endpointInput.value).toBe('http://localhost:11434/v1');
  });

  it('fetches models on click and populates the picker on success', async () => {
    invokeMock.mockImplementation(async (cmd) => {
      if (cmd === 'get_ai_settings') return { ...aiConfig };
      if (cmd === 'list_ai_models') {
        return [{ id: 'gpt-4o', displayName: 'gpt-4o' }];
      }
      return undefined;
    });
    render(AiSettings, { onClose: vi.fn() });

    await fireEvent.click(
      screen.getByRole('button', { name: /Fetch Models/i }),
    );

    await waitFor(() => expect(screen.getByText('gpt-4o')).toBeTruthy());
  });

  it('degrades to manual text entry when fetch fails, without blocking Save', async () => {
    invokeMock.mockImplementation(async (cmd) => {
      if (cmd === 'get_ai_settings') return { ...aiConfig };
      if (cmd === 'list_ai_models') throw 'Could not reach OpenAI.';
      return undefined;
    });
    render(AiSettings, { onClose: vi.fn() });

    await fireEvent.click(
      screen.getByRole('button', { name: /Fetch Models/i }),
    );

    await waitFor(() =>
      expect(screen.getByText('Could not reach OpenAI.')).toBeTruthy(),
    );
    expect(
      screen.getByRole('button', { name: /^Save/ }).hasAttribute('disabled'),
    ).toBe(false);
  });

  it('disables Fetch Models for Custom provider without an endpoint', async () => {
    render(AiSettings, { onClose: vi.fn() });
    await fireEvent.click(screen.getByRole('radio', { name: 'OpenAI' }));
    await fireEvent.click(
      screen.getByRole('radio', { name: 'Custom (OpenAI-compatible)' }),
    );

    const fetchBtn = screen.getByRole('button', {
      name: /Fetch Models/i,
    }) as HTMLButtonElement;
    expect(fetchBtn.disabled).toBe(true);

    await fireEvent.input(screen.getByLabelText(/endpoint/i), {
      target: { value: 'http://localhost:8080/v1' },
    });

    expect(
      (
        screen.getByRole('button', {
          name: /Fetch Models/i,
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(false);
  });

  it('ignores stale fetch results when provider changes mid-flight', async () => {
    let resolvePromise: (value: unknown) => void;
    const modelPromise = new Promise((resolve) => {
      resolvePromise = resolve;
    });

    invokeMock.mockImplementation(async (cmd) => {
      if (cmd === 'get_ai_settings') return { ...aiConfig };
      if (cmd === 'list_ai_models') return modelPromise;
      return undefined;
    });

    render(AiSettings, { onClose: vi.fn() });

    // Start fetch for openai
    await fireEvent.click(
      screen.getByRole('button', { name: /Fetch Models/i }),
    );

    // Switch to Anthropic mid-flight
    await fireEvent.click(screen.getByRole('radio', { name: 'OpenAI' }));
    await fireEvent.click(screen.getByRole('radio', { name: 'Anthropic' }));

    // Resolve with openai models
    resolvePromise!([{ id: 'gpt-4o', displayName: 'gpt-4o' }]);
    await new Promise((r) => setTimeout(r, 0));

    // Should still show idle — not the stale openai models
    expect(screen.getByText(/Fetch Models to load/)).toBeTruthy();
    expect(screen.queryByText('gpt-4o')).toBeNull();
  });

  it('blocks Save for Custom provider with an empty endpoint', async () => {
    render(AiSettings, { onClose: vi.fn() });
    await fireEvent.click(screen.getByRole('radio', { name: 'OpenAI' }));
    await fireEvent.click(
      screen.getByRole('radio', { name: 'Custom (OpenAI-compatible)' }),
    );

    expect(
      (screen.getByRole('button', { name: /^Save/ }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);

    await fireEvent.input(screen.getByLabelText(/endpoint/i), {
      target: { value: 'http://localhost:8080/v1' },
    });

    expect(
      (screen.getByRole('button', { name: /^Save/ }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  it('toggles API key visibility', async () => {
    render(AiSettings, { onClose: vi.fn() });
    const input = screen.getByLabelText(/API Key/i, {
      selector: 'input',
    }) as HTMLInputElement;
    expect(input.type).toBe('password');
    await fireEvent.click(screen.getByRole('button', { name: 'Show API key' }));
    expect(input.type).toBe('text');
  });

  it('shows a status label in the header', () => {
    render(AiSettings, { onClose: vi.fn() });
    expect(screen.getByText('Not tested')).toBeTruthy();
  });

  it('loads the ACP registry and installs an agent through the panel', async () => {
    const agent = {
      id: 'opencode',
      name: 'OpenCode',
      version: '1.2.3',
      description: 'Terminal agent',
      license: 'MIT',
      icon: null,
      installedVersion: null,
      updateAvailable: false,
    };
    invokeMock.mockImplementation(async (cmd) => {
      if (cmd === 'get_ai_settings') return { ...aiConfig };
      if (cmd === 'list_registry_agents') return [agent];
      return undefined;
    });
    render(AiSettings, { onClose: vi.fn() });

    // "Terminal agent" is the registry row's description — the provider
    // picker also contains an "OpenCode" card, so the row text must be
    // disambiguated from it.
    await screen.findByText('Terminal agent');
    await fireEvent.click(
      await screen.findByRole('button', { name: 'Install' }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('install_acp_agent', {
        agentId: 'opencode',
      }),
    );
  });

  it('hides the API key field and model picker when an ACP provider is selected', async () => {
    invokeMock.mockImplementation(async (cmd) => {
      if (cmd === 'get_ai_settings') return { ...aiConfig };
      if (cmd === 'list_installed_acp_agents') {
        return [
          {
            id: 'opencode',
            version: '1.2.3',
            launch: { cmd: 'npx', args: [], env: {} },
          },
        ];
      }
      return undefined;
    });
    render(AiSettings, { onClose: vi.fn() });

    await fireEvent.click(
      await screen.findByRole('radio', { name: 'ACP Agent — opencode' }),
    );

    expect(screen.queryByLabelText(/API Key/i)).toBeNull();
    expect(screen.queryByRole('button', { name: /Fetch Models/i })).toBeNull();
  });

  it('shows the ACP config section with the selected agent and auto-deny when provider is acp', async () => {
    invokeMock.mockImplementation(async (cmd) => {
      if (cmd === 'get_ai_settings') return { ...aiConfig };
      if (cmd === 'list_installed_acp_agents') {
        return [
          {
            id: 'opencode',
            version: '1.2.3',
            launch: { cmd: 'npx', args: [], env: {} },
          },
        ];
      }
      return undefined;
    });
    render(AiSettings, { onClose: vi.fn() });

    await fireEvent.click(
      await screen.findByRole('radio', { name: 'ACP Agent — opencode' }),
    );

    expect(screen.getByText('Selected agent')).toBeTruthy();
    expect(aiConfig.acp?.agentId).toBe('opencode');
    expect(screen.getByRole('checkbox', { name: /Auto-deny/i })).toBeTruthy();
  });

  it('passes the acp block to save_ai_settings when saving an ACP provider', async () => {
    aiConfig.provider = 'acp';
    aiConfig.acp = {
      agentId: 'opencode',
      command: null,
      env: {},
      autoDenyPermissions: false,
    };
    render(AiSettings, { onClose: vi.fn() });

    await fireEvent.click(screen.getByRole('button', { name: /^Save/ }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_ai_settings', {
        config: expect.objectContaining({
          provider: 'acp',
          acp: {
            agentId: 'opencode',
            command: null,
            env: {},
            autoDenyPermissions: false,
          },
        }),
        apiKey: null,
      }),
    );
  });
});
