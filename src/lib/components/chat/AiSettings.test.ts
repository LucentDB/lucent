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

/**
 * The dialog is a two-pane settings surface, so a control only exists while
 * its section is open. Tests that reach across sections navigate first, the
 * same way a user does.
 */
async function goTo(
  section: 'Provider' | 'Model' | 'Agents' | 'Data & Safety',
) {
  await fireEvent.click(screen.getByRole('button', { name: section }));
}

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

    await goTo('Model');
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

    await goTo('Model');
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

    await goTo('Model');
    expect(
      (
        screen.getByRole('button', {
          name: /Fetch Models/i,
        }) as HTMLButtonElement
      ).disabled,
    ).toBe(true);

    await goTo('Provider');
    await fireEvent.input(screen.getByLabelText(/endpoint/i), {
      target: { value: 'http://localhost:8080/v1' },
    });

    await goTo('Model');
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
    await goTo('Model');
    await fireEvent.click(
      screen.getByRole('button', { name: /Fetch Models/i }),
    );

    // Switch to Anthropic mid-flight
    await goTo('Provider');
    await fireEvent.click(screen.getByRole('radio', { name: 'OpenAI' }));
    await fireEvent.click(screen.getByRole('radio', { name: 'Anthropic' }));

    // Resolve with openai models
    resolvePromise!([{ id: 'gpt-4o', displayName: 'gpt-4o' }]);
    await new Promise((r) => setTimeout(r, 0));

    // Should still show idle — not the stale openai models
    await goTo('Model');
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

    await goTo('Agents');
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

  it('shows the agent config with the selected agent and auto-deny when provider is acp', async () => {
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

    expect(aiConfig.acp?.agentId).toBe('opencode');
    await goTo('Agents');
    expect(screen.getByText('Selected agent')).toBeTruthy();
    // A switch, not a checkbox: it applies immediately rather than being a
    // form value that Save collects.
    expect(screen.getByRole('switch', { name: /Auto-deny/i })).toBeTruthy();
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

  // ── The two-pane redesign ───────────────────────────────────────────────

  it('opens on Provider and shows one section at a time', async () => {
    render(AiSettings, { onClose: vi.fn() });
    // Provider content present, other sections' content absent.
    expect(
      screen.getByLabelText(/API Key/i, { selector: 'input' }),
    ).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Fetch Models/i })).toBeNull();
    expect(screen.queryByText('Row limit')).toBeNull();

    await goTo('Data & Safety');
    expect(screen.getByText('Row limit')).toBeTruthy();
    expect(
      screen.queryByLabelText(/API Key/i, { selector: 'input' }),
    ).toBeNull();
  });

  it('drops the Model section for an agent provider, which brings its own', async () => {
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
    expect(screen.getByRole('button', { name: 'Model' })).toBeTruthy();

    await fireEvent.click(
      await screen.findByRole('radio', { name: 'ACP Agent — opencode' }),
    );
    expect(screen.queryByRole('button', { name: 'Model' })).toBeNull();
  });

  // Through the UI the provider only changes from the Provider pane, so Model
  // cannot vanish while the user stands in it. The guard is defensive, and the
  // store is shared, so drive it from there rather than fake a UI path.
  it('falls back to Provider when the open section stops applying', async () => {
    render(AiSettings, { onClose: vi.fn() });
    await goTo('Model');
    expect(screen.getByRole('button', { name: /Fetch Models/i })).toBeTruthy();

    aiConfig.provider = 'acp';

    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'Model' })).toBeNull(),
    );
    // The pane recovered onto Provider rather than rendering nothing.
    expect(screen.getByText('Signed in as')).toBeTruthy();
  });

  // maxTokens, maxTurns and rowLimit were already loaded from disk and already
  // sent on every save, but no control for any of them existed in the dialog.
  it('saves the generation and row limits that previously had no controls', async () => {
    render(AiSettings, { onClose: vi.fn() });

    await goTo('Model');
    await fireEvent.input(screen.getByLabelText('Max tokens'), {
      target: { value: '8192' },
    });
    await fireEvent.input(screen.getByLabelText('Max turns'), {
      target: { value: '12' },
    });

    await goTo('Data & Safety');
    await fireEvent.input(screen.getByLabelText('Row limit'), {
      target: { value: '250' },
    });

    await fireEvent.click(screen.getByRole('button', { name: /^Save/ }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('save_ai_settings', {
        config: expect.objectContaining({
          maxTokens: 8192,
          maxTurns: 12,
          rowLimit: 250,
        }),
        apiKey: null,
      }),
    );
  });

  it('toggles the safety switches through their accessible role', async () => {
    render(AiSettings, { onClose: vi.fn() });
    await goTo('Data & Safety');

    const before = aiConfig.enableBlastRadiusCheck;
    await fireEvent.click(
      screen.getByRole('switch', { name: /Confirm before writes/i }),
    );
    expect(aiConfig.enableBlastRadiusCheck).toBe(!before);
  });

  it('closes on Escape', async () => {
    const onClose = vi.fn();
    const { container } = render(AiSettings, { onClose });
    await fireEvent.keyDown(container.querySelector('[role="dialog"]')!, {
      key: 'Escape',
    });
    expect(onClose).toHaveBeenCalled();
  });

  it('refreshes the provider picker after installing an agent', async () => {
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
    let installed: unknown[] = [];
    invokeMock.mockImplementation(async (cmd) => {
      if (cmd === 'get_ai_settings') return { ...aiConfig };
      if (cmd === 'list_registry_agents') return [agent];
      if (cmd === 'list_installed_acp_agents') return installed;
      if (cmd === 'install_acp_agent') {
        installed = [
          {
            id: 'opencode',
            version: '1.2.3',
            launch: { cmd: 'npx', args: [], env: {} },
            name: 'OpenCode',
          },
        ];
        return null;
      }
      return undefined;
    });
    render(AiSettings, { onClose: vi.fn() });

    await goTo('Agents');
    await screen.findByText('Terminal agent');
    await fireEvent.click(
      await screen.findByRole('button', { name: 'Install' }),
    );
    // Back to Provider: installing an agent is what makes it selectable there.
    await goTo('Provider');
    await waitFor(() =>
      expect(
        screen.getByRole('radio', { name: 'OpenCode — opencode' }),
      ).toBeTruthy(),
    );
  });
});
