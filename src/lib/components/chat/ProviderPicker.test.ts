import { describe, it, expect, vi, afterEach, beforeEach } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/svelte';
import ProviderPicker from './ProviderPicker.svelte';

const listInstalledAcpAgentsMock = vi.fn();
vi.mock('../../ipc/ai.ts', () => ({
  listInstalledAcpAgents: (...args: unknown[]) =>
    listInstalledAcpAgentsMock(...args),
}));

afterEach(cleanup);

beforeEach(() => {
  listInstalledAcpAgentsMock.mockReset();
  listInstalledAcpAgentsMock.mockResolvedValue([]);
});

describe('ProviderPicker', () => {
  it('renders all 11 providers as radio cards across two groups', () => {
    render(ProviderPicker, { value: 'openai', onChange: vi.fn() });
    expect(screen.getAllByRole('radio')).toHaveLength(11);
    expect(screen.getByRole('radio', { name: 'OpenCode' })).toBeTruthy();
    expect(screen.getByText('Cloud providers')).toBeTruthy();
    expect(screen.getByText('Local & self-hosted')).toBeTruthy();
  });

  it('marks the current provider as checked', () => {
    render(ProviderPicker, { value: 'openai', onChange: vi.fn() });
    const openai = screen.getByRole('radio', { name: 'OpenAI' });
    expect(openai.getAttribute('aria-checked')).toBe('true');
    const anthropic = screen.getByRole('radio', { name: 'Anthropic' });
    expect(anthropic.getAttribute('aria-checked')).toBe('false');
  });

  it('calls onChange when a card is clicked', async () => {
    const onChange = vi.fn();
    render(ProviderPicker, { value: 'openai', onChange });
    await fireEvent.click(screen.getByRole('radio', { name: 'Groq' }));
    expect(onChange).toHaveBeenCalledWith('groq');
  });

  it('selects the next provider with ArrowRight and wraps', () => {
    const onChange = vi.fn();
    render(ProviderPicker, { value: 'openai', onChange });
    const cloud = screen.getAllByRole('radiogroup')[0];

    fireEvent.keyDown(cloud, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenLastCalledWith('anthropic');

    // 8 more presses walk the rest of the cloud group and wrap back to openai
    for (let i = 0; i < 8; i++) {
      fireEvent.keyDown(cloud, { key: 'ArrowRight' });
    }
    expect(onChange).toHaveBeenLastCalledWith('openai');
  });

  it('moves with ArrowRight and selects the focused card with Enter', () => {
    const onChange = vi.fn();
    render(ProviderPicker, { value: 'openai', onChange });
    const cloud = screen.getAllByRole('radiogroup')[0];

    fireEvent.keyDown(cloud, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenLastCalledWith('anthropic');

    fireEvent.keyDown(cloud, { key: 'Enter' });
    // Enter selects the focused card (anthropic) without moving focus
    expect(onChange).toHaveBeenLastCalledWith('anthropic');
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it('renders brand logos inside provider cards', () => {
    render(ProviderPicker, { value: 'openai', onChange: vi.fn() });
    const anthropic = screen.getByRole('radio', { name: 'Anthropic' });
    expect(anthropic.querySelector('svg')).toBeTruthy();
  });

  it('lists installed acp agents as providers', async () => {
    listInstalledAcpAgentsMock.mockResolvedValue([
      {
        id: 'opencode',
        version: '1.2.3',
        launch: { cmd: 'npx', args: [], env: {} },
      },
      {
        id: 'claude-acp',
        version: '0.9.0',
        launch: { cmd: 'npx', args: [], env: {} },
      },
    ]);
    render(ProviderPicker, { value: 'openai', onChange: vi.fn() });
    expect(
      await screen.findByRole('radio', { name: 'ACP Agent — opencode' }),
    ).toBeTruthy();
    expect(
      screen.getByRole('radio', { name: 'ACP Agent — claude-acp' }),
    ).toBeTruthy();
  });

  it('marks the acp provider selected when value is acp', async () => {
    listInstalledAcpAgentsMock.mockResolvedValue([
      {
        id: 'opencode',
        version: '1.2.3',
        launch: { cmd: 'npx', args: [], env: {} },
      },
    ]);
    render(ProviderPicker, {
      value: 'acp',
      acpAgentId: 'opencode',
      onChange: vi.fn(),
    });
    const card = await screen.findByRole('radio', {
      name: 'ACP Agent — opencode',
    });
    expect(card.getAttribute('aria-checked')).toBe('true');
  });

  it('emits provider id and agent id when an acp card is clicked', async () => {
    listInstalledAcpAgentsMock.mockResolvedValue([
      {
        id: 'opencode',
        version: '1.2.3',
        launch: { cmd: 'npx', args: [], env: {} },
      },
    ]);
    const onChange = vi.fn();
    render(ProviderPicker, { value: 'openai', onChange });
    await fireEvent.click(
      await screen.findByRole('radio', { name: 'ACP Agent — opencode' }),
    );
    expect(onChange).toHaveBeenCalledWith('acp', 'opencode');
  });
});
